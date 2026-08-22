use warframe_acquisition::AcquisitionError;

const MARKER: &[u8] = b"Inventory sync done";
const CARRY_LIMIT: usize = 64;

pub struct LogObservation {
    identity: String,
    len: u64,
    bytes: Vec<u8>,
}
impl LogObservation {
    pub fn new(identity: impl Into<String>, len: u64, bytes: Vec<u8>) -> Self {
        Self {
            identity: identity.into(),
            len,
            bytes,
        }
    }
}

pub struct MonitorInput {
    now: u64,
    discovery: Result<Option<u32>, AcquisitionError>,
    launcher_seen: bool,
    log: Result<Option<LogObservation>, AcquisitionError>,
}
impl MonitorInput {
    pub fn running(now: u64, pid: u32, log: Option<LogObservation>) -> Self {
        Self {
            now,
            discovery: Ok(Some(pid)),
            launcher_seen: false,
            log: Ok(log),
        }
    }
    pub fn absent(now: u64, launcher_seen: bool) -> Self {
        Self {
            now,
            discovery: Ok(None),
            launcher_seen,
            log: Ok(None),
        }
    }
    pub fn error(now: u64, error: AcquisitionError) -> Self {
        Self {
            now,
            discovery: Err(error),
            launcher_seen: false,
            log: Ok(None),
        }
    }
    pub fn running_with_log_error(now: u64, pid: u32) -> Self {
        Self {
            now,
            discovery: Ok(Some(pid)),
            launcher_seen: false,
            log: Err(AcquisitionError::MemoryReadFailed { pid }),
        }
    }
}

#[derive(Debug, Eq, PartialEq)]
pub struct MonitorResult {
    pub refresh: bool,
    pub acquisition_health: Option<AcquisitionError>,
    pub log_health: Option<LogMonitorDiagnostic>,
}

pub struct MonitorMachine {
    cooldown: u64,
    process: Option<u32>,
    attached_since: Option<u64>,
    last_refresh: Option<u64>,
    pending: bool,
    log_identity: Option<String>,
    log_offset: u64,
    carry: Vec<u8>,
}

impl MonitorMachine {
    pub fn new(cooldown_seconds: u64) -> Self {
        Self {
            cooldown: cooldown_seconds,
            process: None,
            attached_since: None,
            last_refresh: None,
            pending: false,
            log_identity: None,
            log_offset: 0,
            carry: Vec::new(),
        }
    }

    pub const fn log_offset(&self) -> u64 {
        self.log_offset
    }
    pub const fn process_pid(&self) -> Option<u32> {
        self.process
    }
    pub fn log_identity(&self) -> Option<&str> {
        self.log_identity.as_deref()
    }
    pub fn attached_since(&self) -> Option<u64> {
        self.attached_since
    }

    pub fn tick(&mut self, input: MonitorInput) -> MonitorResult {
        let pid = match input.discovery {
            Ok(Some(pid)) => pid,
            Ok(None) => {
                if self.process.is_some() {
                    log::info!("monitor: game process gone, resetting");
                }
                self.reset_process();
                let acquisition_health = if input.launcher_seen {
                    AcquisitionError::LauncherRunning
                } else {
                    AcquisitionError::GameNotRunning
                };
                return MonitorResult {
                    refresh: false,
                    acquisition_health: Some(acquisition_health),
                    log_health: Some(LogMonitorDiagnostic::Unavailable),
                };
            }
            Err(error) => {
                return MonitorResult {
                    refresh: false,
                    acquisition_health: Some(error),
                    log_health: Some(LogMonitorDiagnostic::Unavailable),
                };
            }
        };
        let mut event = false;
        if self.process != Some(pid) {
            self.reset_process();
            self.process = Some(pid);
            self.attached_since = Some(input.now);
            event = true;
        }
        match input.log {
            Err(error) => {
                log::warn!("monitor: EE.log read failed: {error}");
                MonitorResult {
                    refresh: self.schedule(input.now, event),
                    acquisition_health: None,
                    log_health: Some(LogMonitorDiagnostic::ReadFailed),
                }
            }
            Ok(Some(log)) => {
                event |= self.ingest(log);
                MonitorResult {
                    refresh: self.schedule(input.now, event),
                    acquisition_health: None,
                    log_health: Some(LogMonitorDiagnostic::Ready),
                }
            }
            Ok(None) => MonitorResult {
                refresh: self.schedule(input.now, event),
                acquisition_health: None,
                log_health: Some(LogMonitorDiagnostic::Unavailable),
            },
        }
    }

    fn schedule(&mut self, now: u64, event: bool) -> bool {
        self.pending |= event;
        let ready = self
            .last_refresh
            .is_none_or(|last| now.saturating_sub(last) >= self.cooldown);
        if self.pending && ready {
            self.pending = false;
            self.last_refresh = Some(now);
            true
        } else {
            false
        }
    }

    fn ingest(&mut self, log: LogObservation) -> bool {
        let rotated =
            self.log_identity.as_deref() != Some(&log.identity) || log.len < self.log_offset;
        if rotated {
            log::info!("monitor: EE.log rotated or changed identity");
            self.log_identity = Some(log.identity);
            self.log_offset = 0;
            self.carry.clear();
        }
        if self.log_offset == 0 && log.bytes.is_empty() {
            self.log_offset = log.len;
            return false;
        }
        let mut joined = std::mem::take(&mut self.carry);
        joined.extend_from_slice(&log.bytes);
        let mut triggered = false;
        let mut line_start = 0;
        for (index, byte) in joined.iter().enumerate() {
            if *byte == b'\n' {
                triggered |= joined[line_start..index]
                    .windows(MARKER.len())
                    .any(|window| window == MARKER);
                line_start = index + 1;
            }
        }
        self.carry.extend_from_slice(&joined[line_start..]);
        if self.carry.len() > CARRY_LIMIT {
            self.carry.drain(..self.carry.len() - CARRY_LIMIT);
        }
        self.log_offset = log.len;
        triggered
    }

    fn reset_process(&mut self) {
        self.process = None;
        self.attached_since = None;
        self.log_identity = None;
        self.log_offset = 0;
        self.carry.clear();
        self.pending = false;
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogMonitorDiagnostic {
    Ready,
    Unavailable,
    ReadFailed,
}

/// Unix seconds of the session a chunk of EE.log belongs to, taken from the `[UTC: ...]` clock the
/// game writes near the top of every log. EE.log lines themselves carry only engine uptime, so
/// this line is the only place a log says when it happened.
pub fn ee_log_session_start_utc(bytes: &[u8]) -> Option<u64> {
    let start = find_subslice(bytes, b"[UTC: ")? + b"[UTC: ".len();
    let end = start + find_subslice(&bytes[start..], b"]")?;
    let clock = std::str::from_utf8(&bytes[start..end]).ok()?;
    civil_from_clock(clock)
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// `Sat Aug 22 02:01:50 2026` as seconds since the epoch. The weekday is parsed past rather than
/// checked: the date carries it, and a mismatch is the game's problem to lie about, not ours.
fn civil_from_clock(clock: &str) -> Option<u64> {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let mut fields = clock.split_whitespace();
    fields.next()?;
    let month_name = fields.next()?;
    let month = MONTHS.iter().position(|name| *name == month_name)? as u32 + 1;
    let day = fields.next()?.parse::<u32>().ok()?;
    let mut time = fields.next()?.split(':');
    let hour = time.next()?.parse::<u32>().ok()?;
    let minute = time.next()?.parse::<u32>().ok()?;
    let second = time.next()?.parse::<u32>().ok()?;
    let year = fields.next()?.parse::<i64>().ok()?;
    if !(1..=31).contains(&day) || hour > 23 || minute > 59 || second > 59 {
        return None;
    }
    Some(
        (days_from_civil(year, month, day) * 86_400
            + hour as i64 * 3_600
            + minute as i64 * 60
            + second as i64) as u64,
    )
}

/// Days from 1970-01-01, civil algorithm (Hinnant): months are counted from March so a leap day
/// rides at the end of the year instead of splitting February.
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_from_march = (month as i64 + 9) % 12;
    let day_of_year = (153 * month_from_march + 2) / 5 + day as i64 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Byte offset of the first line at or after `floor` on the log's wall clock, or `None` when every
/// line is older. A line's wall time is the session start plus its leading uptime; a line without
/// one (the game drops it on a few shutdown lines) rides the uptime of the line above.
pub fn ee_log_stale_prefix_end(bytes: &[u8], session_start: u64, floor: u64) -> Option<usize> {
    let floor = floor as f64;
    let session_start = session_start as f64;
    let mut uptime = 0.0_f64;
    let fresh = |line: &[u8], uptime: &mut f64| -> bool {
        if let Some(parsed) = leading_uptime(line) {
            *uptime = parsed;
        }
        session_start + *uptime >= floor
    };
    let mut line_start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            if fresh(&bytes[line_start..index], &mut uptime) {
                return Some(line_start);
            }
            line_start = index + 1;
        }
    }
    if line_start < bytes.len() && fresh(&bytes[line_start..], &mut uptime) {
        return Some(line_start);
    }
    None
}

/// The engine uptime a line opens with, when it opens with one at all.
fn leading_uptime(line: &[u8]) -> Option<f64> {
    let end = line
        .iter()
        .position(|byte| *byte == b' ' || *byte == b'\t')
        .unwrap_or(line.len());
    let uptime = std::str::from_utf8(&line[..end])
        .ok()?
        .parse::<f64>()
        .ok()?;
    uptime.is_finite().then_some(uptime)
}

/// How far before attachment a line may still have happened and count as fresh. EE.log reaches
/// this process seconds after the events it describes -- measured at ~7.5s on 2026-07-27 -- so a
/// line written just after attach can carry a timestamp from just before it.
pub const EE_LOG_ATTACH_GRACE_SECS: u64 = 60;

/// Where a from-zero read of a replacement EE.log may start, given the moment the monitor attached
/// to the game process it is reading for. `None` means the whole file predates the session and
/// none of it may be replayed.
///
/// 2026-08-22 is why this exists. The EE.log path resolution flipped between Wine prefixes a
/// second after attach, the flip reset the read offset to zero, and the morning's fissure replayed
/// as if it were live: the poller armed from eleven-hour-old relic loads, the reward pipeline ran
/// against a screen that did not exist, and health ended the day degraded for a game that was
/// never running. A file that cannot be placed in time at all is treated as stale -- a missed
/// reward is quieter than a false report.
pub fn ee_log_rotation_keep_from(
    bytes: &[u8],
    file_created_unix: Option<u64>,
    attached_since: Option<u64>,
) -> Option<usize> {
    let Some(attached) = attached_since else {
        return Some(0);
    };
    let floor = attached.saturating_sub(EE_LOG_ATTACH_GRACE_SECS);
    if let Some(session_start) = ee_log_session_start_utc(bytes) {
        return ee_log_stale_prefix_end(bytes, session_start, floor);
    }
    if file_created_unix.is_some_and(|created| created >= floor) {
        return Some(0);
    }
    None
}
