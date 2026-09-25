use super::{
    AccessPolicy, SharedRuntime, apply_outcome, kiosk_geometry, kiosk_log,
    kiosk_ocr::{self, BasketRow, GridCell},
    kiosk_scroll,
    kiosk_view::{self, KioskState, KioskView},
    overlay_window, refresh_blocking, reward_capture,
    reward_log::{RewardLogEvent, RewardLogMachine},
    reward_observer::{RewardObservation, RewardObserverState},
    reward_ocr::{self, ScreenRewardSource},
    reward_recognition::{RecognitionTiming, RecognizedRewards, RewardRecognition},
    reward_source::LiveMemoryRewardState,
};
use app_core::InventoryRefreshOutcome;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, Emitter, Manager};
#[cfg(target_os = "linux")]
use warframe_acquisition::LinuxProc as ProcessObserver;
#[cfg(windows)]
use warframe_acquisition::WindowsProc as ProcessObserver;
use warframe_acquisition::{
    AcquisitionError, CatalogCache, CatalogIndex, GameProcess, MarketPriceCache, MemoryReader,
    ProcessDiscovery, RelicCatalogCache, RelicRewardIndex, RewardCatalogEntry, RewardMemoryScanner,
    WarmOutcome, WfcdCatalogHttp, WfcdRelicCatalogHttp,
};
use warframe_domain::RewardCandidate;

/// Upper bound on how long a single fissure mission is worth watching for.
/// Shared with the kiosk poller; reward recognition owns its own search/watch cadence.
const POLLER_LIFETIME: Duration = Duration::from_secs(45 * 60);
/// The kiosk poller's steady cadence: the kiosk stays up while the player browses, so there is
/// no "found it, watch faster" split like the reward screen's -- one rate fast enough to feel
/// live against basket edits and slow enough to keep OCR off the CPU.
const KIOSK_POLL_INTERVAL: Duration = Duration::from_millis(400);
/// The kiosk poller's cadence while the grid is drifting: a grim capture costs ~25ms, so a 60ms
/// tick streams scroll offsets in near real time and still leaves the CPU alone.
const KIOSK_MOTION_INTERVAL: Duration = Duration::from_millis(60);
/// How long a kiosk close stays a maybe: a sale confirm rebuilds the kiosk screen mid-visit
/// (EE.log `Saving profile`, then `HudVis 0` plus a foreign subscription, then the full open
/// markers one to two seconds later -- measured 1.67s on a 2026-09-20 live session), so the
/// monitor only tears down once the close markers stay silent past this window. A reopen
/// inside the window cancels the pending teardown.
pub const KIOSK_CLOSE_GRACE: Duration = Duration::from_millis(3000);

/// Tell the kiosk window which visit now owns it; it retires any previous visit synchronously,
/// then fetches the latest epoch itself.
fn emit_kiosk_update(app: &AppHandle, session: Option<u64>) {
    let _ = app.emit_to("kiosk-overlay", "kiosk-updated", session);
}

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
#[cfg(unix)]
pub(crate) fn inventory_log_path(pid: u32) -> Option<PathBuf> {
    inventory_log_path_at(Path::new("/proc"), pid)
}

/// On Windows the game writes to its own `%LOCALAPPDATA%`, so there is no prefix to discover and
/// the PID is not needed -- but the signature is shared with the Wine path, which does need it.
#[cfg(windows)]
pub(crate) fn inventory_log_path(_pid: u32) -> Option<PathBuf> {
    inventory_log_under(Path::new(&std::env::var_os("LOCALAPPDATA")?))
}

/// The log under a given `%LOCALAPPDATA%`, if the game has written one.
///
/// Taking the root as an argument is what makes the layout testable against a synthetic tree; the
/// Wine path is parameterised the same way and for the same reason.
#[cfg(windows)]
pub fn inventory_log_under(local_appdata: &Path) -> Option<PathBuf> {
    let path = local_appdata.join("Warframe/EE.log");
    // `is_file` and not `exists`: an uninstall can leave the folder behind, and taking a directory
    // for the log turns every later read into a permission error instead of "the game has not run".
    path.is_file().then_some(path)
}

#[cfg(unix)]
pub fn inventory_log_path_at(proc_root: &Path, pid: u32) -> Option<PathBuf> {
    let mut prefixes = Vec::new();
    let process_root = proc_root.join(pid.to_string());
    if let Ok(environment) = fs::read(process_root.join("environ")) {
        if let Some(prefix) = environment
            .split(|byte| *byte == 0)
            .find_map(|entry| entry.strip_prefix(b"WINEPREFIX="))
            .and_then(|value| String::from_utf8(value.to_vec()).ok())
        {
            prefixes.push(PathBuf::from(prefix));
        }
    }
    for source in [
        fs::read_link(process_root.join("exe"))
            .ok()
            .map(|path| path.to_string_lossy().into_owned()),
        fs::read_to_string(process_root.join("maps")).ok(),
    ]
    .into_iter()
    .flatten()
    {
        for line in source.lines() {
            let Some(path_start) = line.find('/') else {
                continue;
            };
            if let Some((prefix, _)) = line[path_start..].rsplit_once("/drive_c/") {
                prefixes.push(PathBuf::from(prefix));
            }
        }
    }
    prefixes.sort();
    prefixes.dedup();
    for prefix in prefixes {
        let users = prefix.join("drive_c/users");
        let Ok(users) = fs::read_dir(users) else {
            continue;
        };
        for user in users.flatten() {
            for relative in [
                "AppData/Local/Warframe/EE.log",
                "Local Settings/Application Data/Warframe/EE.log",
            ] {
                let path = user.path().join(relative);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
    }
    None
}

/// The kiosk half of the monitor loop.
///
/// EE.log opens a kiosk session and its `PopulateGrid()` lines ask for re-anchors, and the
/// same log closes it: `HudVis 0` (or input moving to another screen) sets the session's
/// `gone` flag, which both stops the poller thread at its next tick and is consumed by the
/// monitor as the close verdict. Both flags are shared with the poller thread; this struct
/// owns the lifecycle around them so the wiring can be tested with stub hooks instead of a
/// game, a window and a thread.
///
type SpawnKioskPoller<'a> = dyn Fn(
        u64,
        &Arc<std::sync::atomic::AtomicBool>,
        &Arc<std::sync::atomic::AtomicBool>,
    ) -> std::thread::JoinHandle<()>
    + 'a;

/// The log machine is a member and not a `monitor_game` local because closing must reset it: the
/// machine never un-opens, so without a reset the *next* kiosk visit's markers would be swallowed
/// by the previous session's `open` state.
#[derive(Default)]
pub struct KioskSession {
    machine: kiosk_log::KioskLogMachine,
    poller_active: bool,
    /// A close the monitor has not torn down yet. Deliberately not the poller's stop flag: see
    /// `KioskLogEvent::KioskClosed` below for what sharing one cost.
    close_pending: bool,
    /// A close observed but not yet believed: the teardown verdict waits until this deadline so
    /// a sale-confirm rebuild (close markers, then the open markers again one to two seconds
    /// later) rides out as the same visit. `None` while the kiosk is confidently open.
    close_deadline: Option<Instant>,
    /// The overlay is on screen, so a teardown has something to take down. Separate from
    /// `poller_active` because a close retires the poller at once while the window waits for
    /// the monitor's next tick -- and a reopen in between cancels the teardown entirely.
    overlay_up: bool,
    reanchor: Arc<std::sync::atomic::AtomicBool>,
    active_session: Option<u64>,
    gone: Arc<std::sync::atomic::AtomicBool>,
    poller: Option<std::thread::JoinHandle<()>>,
    retired_pollers: Vec<std::thread::JoinHandle<()>>,
}

impl KioskSession {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed incremental EE.log bytes into the kiosk lifecycle, stamped with the monitor tick
    /// that delivered them. `spawn_poller` receives the session identity and shared flags:
    /// `reanchor` requests a read and `gone` permanently stops the worker. A close only arms
    /// a teardown deadline -- a later open inside the window cancels it before anything retires,
    /// so a sale-confirm rebuild rides out as the same visit.
    pub fn observe(
        &mut self,
        bytes: &[u8],
        kiosk_view: &KioskState,
        show: &dyn Fn(),
        spawn_poller: &SpawnKioskPoller<'_>,
        now: Instant,
    ) {
        for event in self.machine.observe_bytes(bytes) {
            log::debug!("[DEBUG-kiosk] ee event {event:?}");
            match event {
                kiosk_log::KioskLogEvent::KioskOpened => {
                    if self.poller_active {
                        // The machine already de-duplicates, but both open markers can land in
                        // one batch and a second poller would race the first for the same flags.
                        // A teardown pending here means the open markers are the sale-confirm
                        // rebuild arriving inside the window: cancel it and carry on as the
                        // same visit instead of churning the session.
                        self.close_pending = false;
                        self.close_deadline = None;
                        continue;
                    }
                    self.arm(kiosk_view, show, spawn_poller);
                }
                kiosk_log::KioskLogEvent::GridPopulated => {
                    if self.poller_active {
                        self.reanchor.store(true, Ordering::Release);
                    }
                }
                kiosk_log::KioskLogEvent::KioskClosed => {
                    // A close is a maybe, not a verdict. The sale-confirm popup rebuilds the
                    // kiosk screen mid-visit -- EE.log shows `Saving profile`, then `HudVis 0`
                    // plus a foreign subscription, then the full open markers one to two seconds
                    // later -- and retiring here took the overlay down on every sale. So this
                    // only arms a deadline: the poller keeps reading, the session and the
                    // published view stay put, and the monitor's next ticks call `take_close`,
                    // which tears down only once the markers stay silent past the window.
                    // (One event, two readers still holds: the poller's stop comes from
                    // `close_overlay` at teardown, never from a flag shared with the monitor,
                    // so the 45-minute-leak lesson stands.)
                    self.close_pending = true;
                    self.close_deadline = Some(now + KIOSK_CLOSE_GRACE);
                }
            }
        }
    }

    /// Adopt whatever state a stretch of log *ends* in -- called once when the monitor attaches
    /// to a game process. Presence is edge-triggered and the live tail starts at EOF (replaying
    /// history is what produced the 2026-08-22 ghost report), so a kiosk already on screen when
    /// the app started had no open marker left to see and the overlay stayed dark until the
    /// player closed and reopened it by hand. A tail that ends closed arms nothing.
    pub fn adopt_log_tail(
        &mut self,
        bytes: &[u8],
        kiosk_view: &KioskState,
        show: &dyn Fn(),
        spawn_poller: &SpawnKioskPoller<'_>,
    ) {
        if self.poller_active || !kiosk_log::KioskLogMachine::state_after(bytes) {
            return;
        }
        // The machine has to know it is open, or the exit line for a session joined late
        // would be read as chatter and the overlay would never come down.
        self.machine.adopt_open();
        self.arm(kiosk_view, show, spawn_poller);
    }

    /// Join only pollers that have already exited. A poller can spend seconds in OCR, and the
    /// monitor must never wait for it before hiding a closed kiosk.
    fn reap_finished_pollers(&mut self) {
        let mut pending = Vec::with_capacity(self.retired_pollers.len());
        for poller in self.retired_pollers.drain(..) {
            if !poller.is_finished() {
                pending.push(poller);
            } else if poller.join().is_err() {
                log::warn!("[DEBUG-kiosk] poller panicked during shutdown");
            }
        }
        self.retired_pollers = pending;
    }

    /// Start a session: fresh identity and flags, a poller on them, and the overlay up.
    fn arm(
        &mut self,
        kiosk_view: &KioskState,
        show: &dyn Fn(),
        spawn_poller: &SpawnKioskPoller<'_>,
    ) {
        // Pending is already false here -- a close no longer retires, so an open while one is
        // pending cancels it in `observe` and never reaches this -- but a reopen must never
        // inherit a stale deadline, so both are cleared unconditionally.
        self.close_pending = false;
        self.close_deadline = None;
        self.poller_active = true;
        self.overlay_up = true;
        // Flags are per poller, never recycled. The previous visit's thread may still be
        // winding down -- it reads its stop every 60-400ms -- and clearing a shared flag for
        // this poller would un-stop that one as well, which a player who reopens quickly can
        // trigger by hand.
        self.reanchor = Arc::new(std::sync::atomic::AtomicBool::new(true));
        self.gone = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let session = kiosk_view.begin_session();
        self.active_session = Some(session);
        self.poller = Some(spawn_poller(session, &self.reanchor, &self.gone));
        show();
    }
    /// Did the session end? True only once a close has stayed silent past the grace window, at
    /// which point the teardown is `close_overlay`'s. Inside the window this is false: the visit
    /// is still alive and a later open cancels the pending teardown outright. The verdict comes
    /// from sustained log silence, not a single line -- the poller only ever stops looking, it
    /// does not judge.
    pub fn take_close(&mut self, kiosk_view: &KioskState, hide: &dyn Fn(), now: Instant) -> bool {
        if !self.close_pending {
            return false;
        }
        if self.close_deadline.is_some_and(|deadline| now < deadline) {
            return false;
        }
        self.close_overlay(kiosk_view, hide);
        true
    }

    /// Tear the session down because the game process died. Unlike a normal kiosk close, process
    /// teardown must wait for every capture worker before the shared portal session is destroyed.
    /// A retained webview also needs the null lifecycle edge before another process can show it.
    pub fn close(&mut self, kiosk_view: &KioskState, hide: &dyn Fn(), retire_frontend: &dyn Fn()) {
        let had_session = self.active_session.is_some();
        self.close_overlay(kiosk_view, hide);
        if had_session {
            retire_frontend();
        }
        for poller in self.retired_pollers.drain(..) {
            if poller.join().is_err() {
                log::warn!("[DEBUG-kiosk] poller panicked during shutdown");
            }
        }
    }

    /// Clear and hide synchronously, then reap only workers that have already exited. OCR may still
    /// be blocked in Tesseract during an ordinary kiosk close, and stale UI must not remain visible
    /// while the monitor waits for it. A later open gets fresh flags, so a retired worker cannot
    /// publish into that session after it observes its permanent stop flag.
    fn close_overlay(&mut self, kiosk_view: &KioskState, hide: &dyn Fn()) {
        self.close_pending = false;
        self.close_deadline = None;
        self.poller_active = false;
        self.gone.store(true, Ordering::Release);
        if let Some(poller) = self.poller.take() {
            self.retired_pollers.push(poller);
        }
        self.machine = kiosk_log::KioskLogMachine::default();
        self.active_session = None;
        kiosk_view.clear();
        if std::mem::take(&mut self.overlay_up) {
            hide();
        }
        self.reap_finished_pollers();
    }
}
#[cfg(target_os = "linux")]
fn should_close_portal(previous: Option<u32>, current: Option<u32>) -> bool {
    previous.is_some() && current.is_none()
}

fn process_was_replaced(previous: Option<u32>, current: Option<u32>) -> bool {
    matches!((previous, current), (Some(previous), Some(current)) if previous != current)
}

fn confirmed_process_observation<T: Copy, E>(
    discovered: &Result<Option<T>, E>,
) -> Option<Option<T>> {
    discovered.as_ref().ok().copied()
}

/// The responder scans that belong to one reward attempt.
///
/// A generation invalidates workers already in flight; the two indexes must turn over with that
/// generation or an old responder can block or populate a later fissure.
#[derive(Default)]
struct RecordScans {
    records: Arc<Mutex<BTreeMap<String, String>>>,
    active: Arc<Mutex<BTreeSet<String>>>,
    generation: Arc<AtomicU64>,
    workers: Mutex<Vec<std::thread::JoinHandle<()>>>,
}

impl RecordScans {
    fn reset(&self) {
        self.generation.fetch_add(1, Ordering::AcqRel);
        if let Ok(mut records) = self.records.lock() {
            records.clear();
        }
        if let Ok(mut active) = self.active.lock() {
            active.clear();
        }
        if let Ok(mut workers) = self.workers.lock() {
            for worker in workers.drain(..) {
                let _ = worker.join();
            }
        }
    }

    fn track_worker(&self, worker: std::thread::JoinHandle<()>) {
        if let Ok(mut workers) = self.workers.lock() {
            workers.push(worker);
        } else {
            let _ = worker.join();
        }
    }

    fn scan(
        &self,
        identity: String,
        process: GameProcess,
        candidates: &[warframe_acquisition::RewardNeedle],
        generation: MonitorGeneration,
    ) {
        if candidates.is_empty() {
            return;
        }
        let Ok(mut active) = self.active.lock() else {
            return;
        };
        if !active.insert(identity.clone()) {
            return;
        }
        drop(active);

        let candidates = candidates.to_vec();
        let records = Arc::clone(&self.records);
        let active = Arc::clone(&self.active);
        let mission_generation = Arc::clone(&self.generation);
        let expected_generation = mission_generation.load(Ordering::Acquire);
        if !generation.is_current() {
            release_player_record_scan(&identity, &active);
            return;
        }
        let worker = std::thread::spawn(move || {
            let started = Instant::now();
            if !generation.is_current() {
                release_player_record_scan(&identity, &active);
                return;
            }
            let procfs = ProcessObserver::new();
            let scanner = RewardMemoryScanner::new(
                256 * 1024,
                768 * 1024 * 1024,
                Duration::from_millis(1_500),
            );
            let resolution = scan_player_record_until_ready(
                expected_generation,
                &mission_generation,
                Duration::from_millis(750),
                || {
                    if !generation.is_current() {
                        return warframe_acquisition::RewardResolution::Incomplete;
                    }
                    scanner
                        .resolve_records(
                            &procfs,
                            &process,
                            &candidates,
                            warframe_acquisition::RewardRecordQuery {
                                responders: &[identity.as_str()],
                                local_identity: None,
                                local_choice: None,
                            },
                            warframe_acquisition::RewardRecordPolicy::LiveStructured,
                        )
                        .unwrap_or(warframe_acquisition::RewardResolution::Incomplete)
                },
            );
            trace_responder_reward_scan(&identity, started.elapsed(), &resolution);
            if generation.is_current() {
                store_player_record_if_current(
                    expected_generation,
                    &mission_generation,
                    &identity,
                    resolution,
                    &records,
                );
            }
            release_player_record_scan(&identity, &active);
        });
        self.track_worker(worker);
    }
}

/// What one observed log event may do: the effective access policy, the monitor generation
/// it belongs to, and the second it was observed in. Bundled so handler signatures stay small.
#[derive(Clone, Copy)]
struct EventScope<'a> {
    policy: AccessPolicy,
    generation: &'a MonitorGeneration,
}

/// Everything whose lifetime is one monitored Warframe process's reward stream.
///
/// The monitor supplies events and application side effects; this session owns the coupled
/// observation, responder-scan and recognition state that must turn over together.
/// Recognition owns the catalog-derived relic pool, background OCR, acceptance and
/// stale-result retirement. The monitor never captures synchronously.
struct RewardSession {
    recognition: RewardRecognition<ScreenRewardSource, fn() -> ScreenRewardSource>,
    memory: LiveMemoryRewardState,
    observer: RewardObserverState,
    scans: RecordScans,
    price_cache: MarketPriceCache,
    /// Consecutive background failures and their reason, so routine pre-screen blanks do not
    /// mark health degraded every poll while broken capture still reports immediately.
    degraded_reason: Option<String>,
    degraded_streak: u32,
}

impl RewardSession {
    fn new(
        catalog: Option<CatalogIndex>,
        relics: Option<RelicRewardIndex>,
        rewards: Vec<RewardCatalogEntry>,
        price_cache: MarketPriceCache,
    ) -> Self {
        Self {
            recognition: RewardRecognition::new(
                catalog,
                relics,
                rewards,
                RecognitionTiming::live(),
                ScreenRewardSource::new,
            ),
            memory: LiveMemoryRewardState::new(RewardMemoryScanner::new(
                256 * 1024,
                768 * 1024 * 1024,
                Duration::from_millis(1_500),
            )),
            observer: RewardObserverState::new(1, 1),
            scans: RecordScans::default(),
            price_cache,
            degraded_reason: None,
            degraded_streak: 0,
        }
    }

    fn handle_event(
        &mut self,
        event: RewardLogEvent,
        process: Option<GameProcess>,
        procfs: &ProcessObserver,
        shared: &SharedRuntime,
        app: &AppHandle,
        scope: EventScope<'_>,
    ) {
        match event {
            RewardLogEvent::RewardWindowOpened => {
                if scope.policy.read_process_memory
                    && let Some(process) = process
                {
                    let _ = procfs.reset_recent_writes(&process);
                }
            }
            RewardLogEvent::ResponderExpected { identity } => {
                self.scan_responder(identity, process, scope.policy, scope.generation);
            }
            RewardLogEvent::ResponderReceived { identity, is_local } => {
                if !is_local {
                    self.scan_responder(identity, process, scope.policy, scope.generation);
                }
            }
            RewardLogEvent::ResponsesComplete { .. } => {
                // Constraints for the background reader; never captures here.
                // A vanished game has no window to read, so skip stale rosters.
                if process.is_some() {
                    self.recognition.observe(&event);
                }
            }
            RewardLogEvent::BaselineRequested { .. } => {
                self.scans.reset();
                let Some(_process) = process else {
                    self.memory.clear();
                    self.recognition.close();
                    return;
                };
                self.recognition.observe(&event);
                // Keep the dormant responder-scan candidates in step with recognition.
                // Recognition owns the catalog derivation; memory only mirrors it.
                let candidates = self.recognition.candidates();
                if scope.policy.read_process_memory {
                    self.memory.prepare_candidates(&candidates);
                } else {
                    self.memory.clear();
                }
                // Price the pool while the mission is still running; the reward screen is
                // minutes away. Warming never blocks event handling.
                let entries = self.recognition.pool_entries();
                spawn_market_price_warm(&entries, &self.price_cache);
            }
            RewardLogEvent::ChoicesReady { .. } => {
                if process.is_none() {
                    return;
                }
                self.recognition.observe(&event);
            }
            RewardLogEvent::Closed => self.close(shared, app),
        }
    }

    fn scan_responder(
        &self,
        identity: String,
        process: Option<GameProcess>,
        policy: AccessPolicy,
        generation: &MonitorGeneration,
    ) {
        if self.recognition.resolved() || !policy.read_process_memory {
            return;
        }
        let Some(process) = process else {
            return;
        };
        self.scans.scan(
            identity,
            process,
            &self.recognition.candidates(),
            generation.clone(),
        );
    }

    fn publish(
        &mut self,
        recognized: RecognizedRewards,
        shared: &SharedRuntime,
        app: &AppHandle,
        now: u64,
        generation: &MonitorGeneration,
    ) {
        let publication = recognized.publication.clone();
        let names = recognized.names;
        let elapsed = recognized.elapsed;
        // Only the first publication for this epoch runs; late duplicates and stale
        // delayed effects are declined by the same gate.
        publication.publish(|| {
            let observations = names
                .into_iter()
                .map(RewardObservation::certain)
                .collect::<Vec<_>>();
            let transition = self.observer.observe(observations);
            let mut overlay_notice = None;
            if transition.publish {
                apply_reward_observations(
                    shared,
                    self.recognition.rewards(),
                    &transition.choices,
                    &BTreeMap::new(),
                );
                overlay_notice = overlay_window::show_reward_overlay(app, transition.choices.len());
                let _ = app.emit_to("reward-overlay", "reward-updated", ());
                spawn_market_price_fetch(
                    &transition.choices,
                    shared,
                    app,
                    self.recognition.rewards(),
                    &self.price_cache,
                    now,
                    generation,
                );
            }
            if let Ok(mut runtime) = shared.lock() {
                let _ = runtime.core.record_capture_source_ready(
                    "ocr",
                    elapsed.as_millis(),
                    now.to_string(),
                );
                // Read the cards but could not find the window to draw over: on Windows that is
                // exclusive fullscreen, and the player is the only one who can fix it.
                if let Some(notice) = overlay_notice {
                    let _ = runtime.core.record_capture_degraded(notice);
                }
            }
        });
    }

    fn drain_recognition(
        &mut self,
        shared: &SharedRuntime,
        app: &AppHandle,
        now: u64,
        generation: &MonitorGeneration,
    ) {
        // The monitor never captures here; it only drains what the background reader found.
        // A slow capture therefore cannot delay overlay hide, game-gone or the next event.
        let update = self.recognition.poll();
        let had_recognized = update.recognized.is_some();
        if let Some(recognized) = update.recognized {
            self.publish(recognized, shared, app, now, generation);
        }
        if let Some(failure) = update.failure {
            let (reason, streak) = if self.degraded_reason.as_deref() == Some(&failure.reason) {
                self.degraded_streak = self.degraded_streak.saturating_add(1);
                (
                    self.degraded_reason.clone().unwrap_or_default(),
                    self.degraded_streak,
                )
            } else {
                self.degraded_reason = Some(failure.reason.clone());
                self.degraded_streak = 1;
                (failure.reason.clone(), 1)
            };
            // Routine pre-screen blanks only matter after a sustained streak; broken capture
            // reports immediately. Without this gate a baseline set minutes before the screen
            // would mark health degraded every poll while nothing is expected on screen.
            if crate::reward_recognition::poll_failure_is_worth_warning(&reason, streak)
                && let Ok(mut runtime) = shared.lock()
            {
                let _ = runtime
                    .core
                    .record_capture_degraded(format!("Screen capture failed: {reason}"));
            }
        } else if had_recognized || update.hide {
            self.degraded_reason = None;
            self.degraded_streak = 0;
        }
        // The capture signal beats EE.log's delayed shutdown line and prevents a stale overlay.
        if update.hide && self.observer.miss().hide {
            overlay_window::hide_reward_overlay(app);
        }
    }

    fn game_gone(&mut self, hide: &dyn Fn()) {
        // Suspend without clearing the pool: a transient process-absent tick must not blind the
        // fissure, since relic baselines are one-shot log lines with no recovery path. The worker
        // drops its source and pauses; recognition resumes against the preserved pool when the
        // process returns. Full clearing is reserved for the log's Closed event.
        self.recognition.suspend();
        self.memory.clear();
        self.scans.reset();
        if self.observer.miss().hide {
            hide();
        }
    }

    fn resume(&mut self) {
        self.recognition.resume();
    }

    fn close(&mut self, shared: &SharedRuntime, app: &AppHandle) {
        self.recognition.close();
        self.scans.reset();
        self.memory.clear();
        self.observer.miss();
        // A new screen starts a fresh degraded-health streak; otherwise the next fissure's
        // first routine blank could hit the warning threshold on the previous screen's count.
        self.degraded_reason = None;
        self.degraded_streak = 0;
        overlay_window::hide_reward_overlay(app);
        if let Ok(mut runtime) = shared.lock() {
            let _ = runtime.core.apply_reward_candidates(Vec::new());
        }
    }
}

pub(crate) fn run(
    shared: SharedRuntime,
    app: AppHandle,
    policy: AccessPolicy,
    generation: MonitorGeneration,
) {
    debug_assert!(policy.observe_process_presence);
    let procfs = ProcessObserver::new();
    let mut machine = MonitorMachine::new(15);
    let mut reward_log = RewardLogMachine::default();
    let mut kiosk_session = KioskSession::new();
    // The kiosk session's window/thread side effects, as hooks so the session logic itself stays
    // testable without an AppHandle.
    let kiosk_show = {
        let app = app.clone();
        move || overlay_window::show_kiosk_overlay(&app)
    };
    let kiosk_hide = {
        let app = app.clone();
        move || overlay_window::hide_kiosk_overlay(&app)
    };
    let kiosk_retire = {
        let app = app.clone();
        move || emit_kiosk_update(&app, None)
    };
    let mut announced_process = None;
    let mut tracked_resolution: Option<(u32, Option<PathBuf>)> = None;
    // Survives across missions on purpose: the same relic pools recur all evening, so a price
    // fetched two runs ago is one this run does not have to make.
    let price_cache = shared
        .lock()
        .map(|runtime| runtime.live_prices.clone())
        .unwrap_or_default();
    let catalog = shared
        .lock()
        .ok()
        .and_then(|runtime| load_catalog(&runtime.app_data));
    if let (Some(catalog), Ok(mut runtime)) = (catalog.as_ref(), shared.lock()) {
        // Before enrichment, so the first view it publishes already carries ducat values.
        runtime
            .core
            .set_collection_ducats(Arc::new(catalog.ducat_table()));
        let _ = runtime.core.enrich_collection_from_catalog(catalog);
    }
    let reward_catalog = catalog
        .as_ref()
        .map(CatalogIndex::reward_entries)
        .unwrap_or_default();
    // The kiosk poller's join inputs, cloned per arm: the closed candidate set, the runtime's
    // price/collection state, and the live market cache that outranks the daily dump.
    let kiosk_candidates = Arc::new(reward_catalog.clone());
    let kiosk_spawn = |session: u64,
                       reanchor: &Arc<std::sync::atomic::AtomicBool>,
                       gone: &Arc<std::sync::atomic::AtomicBool>| {
        let joiner = {
            let shared = Arc::clone(&shared);
            let cache = price_cache.clone();
            move |epoch: u64, frame: &KioskRead| {
                // Take the join inputs under one short lock hold, then build outside it: the
                // OCR thread never makes the UI wait on a lock it does not need.
                let table = shared
                    .lock()
                    .map(|runtime| runtime.core.collection_prices())
                    .unwrap_or_default();
                kiosk_view::build_view(epoch, &frame.cells, &frame.basket, |name| {
                    // The dump's median of completed trades is the honest number for a
                    // sell-advice overlay: what copies actually went for. The live cache
                    // holds the lowest current ask, which a single joke listing can set
                    // arbitrarily high, so it only stands in where the dump has no price.
                    table
                        .as_ref()
                        .and_then(|table| table.price_for(name))
                        .or_else(|| cache.get(name))
                })
            }
        };
        let publish = {
            let app = app.clone();
            let first_publish = Arc::new(std::sync::atomic::AtomicBool::new(true));
            move |view: KioskView| {
                let Some(kiosk) = app.try_state::<KioskState>() else {
                    return;
                };
                let _ = kiosk.set_if_current(session, view, || {
                    if first_publish.swap(false, Ordering::AcqRel) {
                        // On native Wayland the open log marker arrives before capture has located
                        // the game monitor. The first show is deliberately deferred; retry now that
                        // `select_kiosk_strip` has published the matched capture rectangle.
                        overlay_window::show_kiosk_overlay(&app);
                    }
                    emit_kiosk_update(&app, Some(session));
                });
            }
        };
        let emit_scroll = {
            let app = app.clone();
            move |verdict: Option<i32>| {
                // Gate every side effect at the same session mutex as publication, then carry the
                // identity across IPC so an event already queued for the webview cannot cross a
                // close/reopen boundary.
                if let Some(kiosk) = app.try_state::<KioskState>() {
                    let _ = kiosk.run_if_current(session, || {
                        let _ = app.emit_to(
                            "kiosk-overlay",
                            "kiosk-scroll",
                            serde_json::json!({ "session": session, "dy": verdict }),
                        );
                    });
                }
            }
        };
        let poller = spawn_kiosk_poller_with(
            reanchor,
            gone,
            KioskPollerTiming::live(),
            Arc::clone(&kiosk_candidates),
            joiner,
            publish,
            emit_scroll,
            ScreenKioskSource::default,
        );
        log::debug!(
            "[DEBUG-kiosk] poller spawned with {} candidates",
            kiosk_candidates.len()
        );
        poller
    };
    let relic_catalog = shared
        .lock()
        .ok()
        .and_then(|runtime| load_relic_catalog(&runtime.app_data));
    let mut reward_session =
        RewardSession::new(catalog, relic_catalog, reward_catalog, price_cache.clone());
    // EE.log reaches us seconds after the events it describes -- measured at ~7.5s on 2026-07-27,
    // by which time the fifteen-second reward screen can already be gone. The relic-load signal
    // arrives minutes ahead of the screen though, so it can arm a poller that watches for the cards
    // directly. The closed-set match is its own detector: only the reward screen yields four names
    // from this squad's relic pool.

    while generation.is_current() {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let discovered = procfs.discover();
        let confirmed_process = confirmed_process_observation(&discovered);
        let process_replaced = confirmed_process.is_some_and(|current| {
            process_was_replaced(
                announced_process.map(|process: GameProcess| process.pid()),
                current.map(|process| process.pid()),
            )
        });
        #[cfg(target_os = "linux")]
        let mut close_portal = false;
        if let Some(process) = confirmed_process {
            if process != announced_process {
                #[cfg(target_os = "linux")]
                {
                    close_portal = should_close_portal(
                        announced_process.map(|process: GameProcess| process.pid()),
                        process.map(|process| process.pid()),
                    );
                }
                if let Ok(mut runtime) = shared.lock() {
                    runtime.game_running = process.is_some();
                    if process.is_some() {
                        let _ = runtime.core.record_game_process_ready();
                    }
                }
                announced_process = process;
            }
        }
        // A discovery error is not evidence that Warframe exited. Keep the last confirmed process
        // for reward teardown and event handling until procfs reports an actual absence.
        let process = confirmed_process.unwrap_or(announced_process);
        // A direct relaunch may replace one live PID with another without an observable absent
        // poll. Retire the old kiosk before adopting the replacement process's log tail; otherwise
        // its open machine and capture worker would be carried across the process boundary.
        if process_replaced && let Some(kiosk_view_cell) = app.try_state::<KioskState>() {
            kiosk_session.close(kiosk_view_cell.inner(), &kiosk_hide, &kiosk_retire);
        }
        let (input, log_bytes) = match discovered {
            Ok(None) => (
                MonitorInput::absent(now, procfs.launcher_present()),
                Vec::new(),
            ),
            Err(error) => (MonitorInput::error(now, error), Vec::new()),
            Ok(Some(process)) => {
                let path = inventory_log_path(process.pid());
                if monitor_path_changed(tracked_resolution.as_ref(), process.pid(), path.as_deref())
                {
                    log::debug!(
                        "monitor: EE.log path resolution pid={} found={}",
                        process.pid(),
                        path.is_some()
                    );
                    tracked_resolution = Some((process.pid(), path.clone()));
                    if let Some(ref ee_path) = path {
                        if let Ok(mut runtime) = shared.lock() {
                            runtime.last_ee_log_path = Some(ee_path.clone());
                        }
                        // The overlay's presence is edge-triggered from the log and the live
                        // tail starts at EOF, so a kiosk already on screen when the app started
                        // has no open marker left for us to see. Fold the recent tail once, at
                        // attach, to adopt the session in progress.
                        if let Some(kiosk_view_cell) = app.try_state::<KioskState>() {
                            kiosk_session.adopt_log_tail(
                                &log_tail(ee_path, KIOSK_TAIL_SCAN),
                                kiosk_view_cell.inner(),
                                &kiosk_show,
                                &kiosk_spawn,
                            );
                        }
                    }
                }
                build_monitor_input(&machine, now, process.pid(), path)
            }
        };
        let result = machine.tick(input);
        if result.refresh && policy.acquire_inventory {
            let refresh = Arc::clone(&shared);
            let refresh_generation = generation.clone();
            spawn_monitor_refresh_task(move || {
                if refresh_generation.is_current() {
                    let _ = refresh_blocking(&refresh);
                }
            });
        }
        if policy.acquire_inventory
            && let Some(error) = result.acquisition_health
            && generation.is_current()
        {
            let _ = apply_outcome(
                &shared,
                InventoryRefreshOutcome::acquisition_failed(
                    warframe_acquisition::AcquisitionFailure::from_error(error),
                ),
            );
        }
        if let Some(log_health) = result.log_health {
            if let Ok(mut runtime) = shared.lock() {
                let _ = match log_health {
                    LogMonitorDiagnostic::Ready => runtime.core.record_log_monitor_ready(),
                    LogMonitorDiagnostic::Unavailable => {
                        runtime.core.record_log_monitor_idle("Waiting for Warframe")
                    }
                    LogMonitorDiagnostic::ReadFailed => runtime
                        .core
                        .record_log_monitor_failure("EE.log could not be read"),
                };
            }
        }
        for event in reward_log.observe_bytes(&log_bytes) {
            if generation.is_current() {
                reward_session.handle_event(
                    event,
                    process,
                    &procfs,
                    &shared,
                    &app,
                    EventScope {
                        policy,
                        generation: &generation,
                    },
                );
            }
        }
        // Same bytes, second machine: the kiosk's lifecycle is independent of the reward screen's
        // (the two never occur at once in practice, but neither knows about the other).
        if let Some(kiosk_view_cell) = app.try_state::<KioskState>() {
            // A monotonic stamp for the close grace window: the tick's wall clock has only
            // one-second resolution, too coarse for the window.
            let tick = Instant::now();
            kiosk_session.observe(
                &log_bytes,
                kiosk_view_cell.inner(),
                &kiosk_show,
                &kiosk_spawn,
                tick,
            );
            // The log's close line and the game process dying are the only closes there are;
            // both land here. A close tears down only once it has stayed silent past the grace
            // window, so a sale-confirm rebuild never blinks; the emit retires the visit in the
            // frontend exactly when the backend clears it.
            if kiosk_session.take_close(kiosk_view_cell.inner(), &kiosk_hide, tick) {
                emit_kiosk_update(&app, kiosk_view_cell.active_session());
            }
        }
        reward_session.drain_recognition(&shared, &app, now, &generation);
        if process.is_none() {
            reward_session.game_gone(&|| overlay_window::hide_reward_overlay(&app));
            // No game, no kiosk: the capture source is gone even if the miss streak has not
            // finished counting.
            if let Some(kiosk_view_cell) = app.try_state::<KioskState>() {
                kiosk_session.close(kiosk_view_cell.inner(), &kiosk_hide, &kiosk_retire);
            }
            reward_ocr::clear_latest_matched_rect();
            #[cfg(target_os = "linux")]
            if close_portal && let Err(reason) = reward_capture::portal::PortalCapture::close() {
                log::warn!(
                    "[DEBUG-capture] could not close the screencast session ({reason}); \
                     the screen-sharing indicator may stay lit until this process exits"
                );
            }
        } else {
            // A returning process resumes recognition against the preserved pool; suspend
            // made the absence idempotent, so this is a no-op while already active.
            reward_session.resume();
        }
        let poll_interval = if reward_log.reward_window_open() {
            Duration::from_millis(10)
        } else {
            Duration::from_millis(100)
        };
        std::thread::park_timeout(poll_interval);
    }
    reward_session.close(&shared, &app);
    if let Some(kiosk_view_cell) = app.try_state::<KioskState>() {
        kiosk_session.close(kiosk_view_cell.inner(), &kiosk_hide, &kiosk_retire);
    }
    reward_ocr::clear_latest_matched_rect();
    #[cfg(target_os = "linux")]
    let _ = reward_capture::portal::PortalCapture::close();
    if let Ok(mut runtime) = shared.lock() {
        runtime.game_running = false;
    }
}

pub fn spawn_monitor_refresh_task(
    task: impl FnOnce() + Send + 'static,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(task)
}

fn load_catalog(app_data: &Path) -> Option<CatalogIndex> {
    let cache = CatalogCache::new(app_data.join("catalog"));
    if let Ok(catalog) = cache.load_cached() {
        return Some(catalog.index().clone());
    }
    let source = WfcdCatalogHttp::new().ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    cache
        .load(&source, now)
        .ok()
        .map(|catalog| catalog.index().clone())
}

fn load_relic_catalog(app_data: &Path) -> Option<RelicRewardIndex> {
    let cache = RelicCatalogCache::new(app_data.join("catalog"));
    if let Ok(catalog) = cache.load_cached() {
        return Some(catalog.index().clone());
    }
    let source = WfcdRelicCatalogHttp::new().ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    cache
        .load(&source, now)
        .ok()
        .map(|catalog| catalog.index().clone())
}

/// How often the kiosk poller looks, and how long it may run.
///
/// Two rates: 400ms in the steady state (the kiosk stays up while the player browses, and that
/// is fast enough to feel live against basket edits while keeping full OCR passes off the CPU),
/// 60ms while the grid is moving (a capture costs ~25ms through grim, so following a scroll in
/// real time is just polling faster).
#[derive(Clone, Copy, Debug)]
pub struct KioskPollerTiming {
    pub interval: Duration,
    /// The tick while the grid is drifting: cheap strip looks only, streamed to the frontend
    /// as offsets, until a two-look pause says the grid has settled.
    pub motion_interval: Duration,
    pub lifetime: Duration,
}

impl KioskPollerTiming {
    pub const fn live() -> Self {
        Self {
            interval: KIOSK_POLL_INTERVAL,
            motion_interval: KIOSK_MOTION_INTERVAL,
            lifetime: POLLER_LIFETIME,
        }
    }
}

/// One whole poller attempt: what the screen said, per slot.
#[derive(Clone, Default)]
pub struct KioskRead {
    pub cells: Vec<GridCell>,
    pub basket: Vec<BasketRow>,
}

/// The screen behind the kiosk poller, as one method so a test can script it -- the same shape
/// that made the reward poller testable without playing a fissure.
pub trait KioskFrameSource {
    /// `dy` is the grid's tracked scroll offset in pixels: the label bands are calibrated for
    /// the unscrolled grid, so a read at any other scroll position must look there instead.
    fn read_kiosk(
        &mut self,
        candidates: &[RewardCatalogEntry],
        dy: i32,
    ) -> Result<KioskRead, &'static str>;

    /// Mean luma per row across the grid pane, for the scroll tracker's cheap ticks. Sources
    /// that cannot look cheaply leave this as `Err`, and the poller simply falls back to full
    /// reads at the steady cadence.
    fn strip_profile(&mut self) -> Result<Vec<f32>, &'static str> {
        Err("this source has no strip")
    }
}

/// The live kiosk screen. It owns the same long-lived capture backend as the reward reader, so
/// native Wayland sessions reuse their direct/KWin/portal session instead of renegotiating it on
/// every poll. A settle tick would otherwise pay for two captures back to back -- the strip look
/// and the full read -- so the last frame is kept briefly and reused when it is younger than one
/// poll interval; anything staler than that is simply captured again.
pub struct ScreenKioskSource {
    capture: reward_capture::GameCapture,
    recent: Option<(Instant, image::DynamicImage)>,
}

impl Default for ScreenKioskSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenKioskSource {
    pub fn new() -> Self {
        Self {
            capture: reward_capture::GameCapture::new(),
            recent: None,
        }
    }

    fn capture_frame(&mut self) -> Result<(image::DynamicImage, Vec<f32>), &'static str> {
        let candidates = self.capture.capture_candidates()?;
        let (selected, profile) = select_kiosk_strip(candidates)?;
        let frame = selected.image;
        self.recent = Some((Instant::now(), frame.clone()));
        Ok((frame, profile))
    }
}

fn kiosk_strip(frame: &image::DynamicImage) -> (Vec<f32>, u32) {
    let (x, y, w, h) = kiosk_geometry::grid_strip(frame.width(), frame.height());
    (kiosk_scroll::row_profiles(frame, x, y, w, h), y)
}

fn select_kiosk_strip(
    candidates: Vec<reward_capture::CapturedFrame>,
) -> Result<(reward_capture::CapturedFrame, Vec<f32>), &'static str> {
    candidates
        .into_iter()
        .find_map(|candidate| {
            let (profile, strip_top) = kiosk_strip(&candidate.image);
            let at = kiosk_geometry::label_anchors(profile.len());
            kiosk_scroll::label_offset(&profile, strip_top as i32, at.first_top, at.pitch, at.band)
                .map(|_| {
                    reward_ocr::publish_latest_matched_rect(candidate.rect);
                    (candidate, profile)
                })
        })
        .ok_or("the kiosk is not visible on any captured monitor")
}

impl KioskFrameSource for ScreenKioskSource {
    fn read_kiosk(
        &mut self,
        candidates: &[RewardCatalogEntry],
        dy: i32,
    ) -> Result<KioskRead, &'static str> {
        let frame = match &self.recent {
            Some((at, frame)) if at.elapsed() < KIOSK_POLL_INTERVAL => frame.clone(),
            _ => self.capture_frame()?.0,
        };
        self.recent = None;
        Ok(KioskRead {
            cells: kiosk_ocr::read_grid(&frame, candidates, dy),
            basket: kiosk_ocr::read_basket(&frame, candidates),
        })
    }

    fn strip_profile(&mut self) -> Result<Vec<f32>, &'static str> {
        let (_, profile) = self.capture_frame()?;
        Ok(profile)
    }
}

/// The match score that separates a true read from a lookalike. Clean reads of visible
/// labels score 0.94 or better (the kiosk fixture pins 0.85 as the floor of healthy);
/// lookalikes harvested from a misphased crop measured 0.60 to 0.76 (2026-09-25 frame).
const KIOSK_CONFIDENT_SCORE: f32 = 0.85;

/// How far the fine ladder walks: two eighth-pitch rungs to either side of the coarse
/// winner. The measured tie-run drag on the polluted pane was 36 rows of a 291 pitch --
/// one rung -- and two keeps a margin for a drag three times that strong.
const KIOSK_FINE_LADDER_RUNGS: i32 = 2;

/// Confident cells a read holds: matches at or above [`KIOSK_CONFIDENT_SCORE`]. A failed
/// read has none. This is the recovery referee -- sparse-but-true panes read weak in
/// count yet high in score, misphased panes the reverse.
fn confident_cells(read: &Result<KioskRead, &'static str>) -> usize {
    match read {
        Ok(frame) => confident_frame_cells(frame),
        Err(_) => 0,
    }
}

/// Confident cells in one frame. Split out so probes can be judged without wrapping
/// the frame in a `Result` first.
fn confident_frame_cells(frame: &KioskRead) -> usize {
    frame
        .cells
        .iter()
        .filter(|cell| cell.score >= KIOSK_CONFIDENT_SCORE)
        .count()
}

/// How many consecutive still looks before the screen counts as settled and the read runs. One
/// still look can be a flick's mid-detent hitch; two is a scroll that has actually stopped.
const KIOSK_SETTLE_LOOKS: u32 = 2;
/// How many consecutive unmeasurable-but-present looks the poller waits before reading anyway.
/// In-place animation (hover-card renders, dialog pulses) defeats frame-to-frame correlation
/// without moving the grid a pixel; the locator still names the bands, so a read after this
/// many looks publishes anchored on the locator alone instead of never publishing at all.
const KIOSK_UNMEASURED_READ_LOOKS: u32 = 3;

/// The kiosk poller's body, with the screen and the join as parameters.
///
/// This loop does not decide whether the kiosk is open -- EE.log does, on both edges, and it
/// says so ~50ms after the fact (see `kiosk_log`). That separation is the whole design: for a
/// while presence was inferred from the reader's own failures, and every hiccup that made a
/// frame unreadable -- a scroll the tracker could not measure, a locator that found no label
/// band, a capture that came back torn -- read as "the player left" and tore the overlay down
/// mid-session. Here an unreadable look costs nothing but that look: the last good view stays
/// up, and the next tick tries again.
///
/// So the loop only ever answers "is the grid moving, and if it has stopped, what does it
/// say". `emit_scroll` streams the tracker's frame-to-frame verdicts to the frontend:
/// `Some(delta)` is how far the grid moved since the previous look (the chips accumulate it),
/// `None` fades them until the next settled read re-anchors absolutely.
// The parameters are one dependency set (flags, timing, candidates) plus one hook each for the
// three things the loop does (join, publish, scroll); bundling them would hide that shape.
#[allow(clippy::too_many_arguments)]
pub fn spawn_kiosk_poller_with<S, J, P, E>(
    reanchor: &Arc<std::sync::atomic::AtomicBool>,
    gone: &Arc<std::sync::atomic::AtomicBool>,
    timing: KioskPollerTiming,
    candidates: Arc<Vec<RewardCatalogEntry>>,
    joiner: J,
    publish: P,
    emit_scroll: E,
    make_source: impl FnOnce() -> S + Send + 'static,
) -> std::thread::JoinHandle<()>
where
    S: KioskFrameSource + Send + 'static,
    J: Fn(u64, &KioskRead) -> KioskView + Send + 'static,
    P: Fn(KioskView) + Send + 'static,
    E: Fn(Option<i32>) + Send + 'static,
{
    let reanchor = Arc::clone(reanchor);
    let gone = Arc::clone(gone);
    std::thread::spawn(move || {
        let mut source = make_source();
        let mut epoch = 0_u64;
        let mut last_strip: Option<Vec<f32>> = None;
        let mut static_looks = 0_u32;
        let mut unmeasured_looks: u32 = 0;
        // Basket quantities flicker frame to frame on static content (measured
        // 4070<->470 totals on an unchanged 8-row basket): a quantity above 1 is only
        // adopted after two consecutive identical reads of the same basket slot and
        // name, otherwise the row publishes as 1. Keyed, so a reshuffled basket
        // restarts its own streaks without clearing anyone else's.
        let mut quantity_streaks: BTreeMap<(usize, String), (u32, u8)> = BTreeMap::new();
        // Per-look verdict counters, flushed with each publish and at session end: the
        // branches below are otherwise silent, which once hid a 48-second stretch with
        // a single settle behind "no evidence".
        let mut looks_motion = 0u64;
        let mut looks_blind = 0u64;
        let mut looks_unmeasured = 0u64;
        let mut looks_still = 0u64;
        let mut looks_reads = 0u64;
        // The last settle's proven correction to the located phase: content-proven, so it
        // rides the next settle directly instead of paying the ladder again. Retired the
        // moment it stops reading text.
        let mut phase_correction: i32 = 0;
        // Set when an unmeasured run falls through to a locator-anchored read, cleared at the
        // read so the normal two-look settle gate resumes afterwards.
        let mut unmeasured_fell_through = false;

        let deadline = Instant::now() + timing.lifetime;
        // The top anchor is the LOCATOR's on purpose -- deriving it from the OCR crop's rect
        // coupled the two, and growing the crop to catch three-line labels dragged the locator's
        // keying 22 rows down the screen with it.
        while Instant::now() < deadline {
            // The log said the screen went away (or the game did): stop looking at it.
            if gone.load(Ordering::Acquire) {
                break;
            }
            // A re-anchor request (open, filter change, basket edit) advances the epoch so the
            // frontend drops whatever it is showing.
            if reanchor.swap(false, Ordering::AcqRel) {
                epoch += 1;
            }
            // The cheap look first. Motion is frame to frame: only pixels that actually moved
            // between two looks count, so a still screen is still at ANY offset from anything.
            // (An anchor-relative "moved" verdict on a still screen once blocked every read
            // for a minute -- the undead session of 2026-08-23.)
            let current_strip = source.strip_profile();
            let reading = current_strip.as_ref().ok();
            let frame_delta = match (&last_strip, reading) {
                (Some(prev), Some(current)) => kiosk_scroll::estimate_dy(
                    prev,
                    current,
                    kiosk_scroll::FRAME_DELTA_MAX,
                    kiosk_scroll::MIN_PEAK_RATIO,
                ),
                // The first readable look establishes the baseline and counts as still. No
                // previous frame exists yet, so there is no failed measurement to fade over.
                (None, Some(_)) => Some(0),
                (_, None) => None,
            };
            last_strip = reading.cloned();
            let frame_delta = match frame_delta {
                Some(delta) => delta,
                // Unmeasurable but the strip itself read: the pane is there and the
                // locator below can still name its bands. What failed is only the
                // frame-to-frame correlation, which in-place animation (a hover
                // card's render, a dialog's pulse) breaks without moving the grid.
                // Treating that as blindness faded the chips through any animated
                // but still pane, so the overlay only ever appeared in rare truly
                // static moments. Count these looks separately instead: a short run
                // still reads -- the read re-anchors through the locator, so
                // in-place animation cannot misalign its crops. A strip that did not
                // even read is real blindness (a cinematic over the pane, the kiosk
                // gone) and keeps the fade verdict.
                None if reading.is_some() => {
                    unmeasured_looks = unmeasured_looks.saturating_add(1);
                    looks_unmeasured += 1;
                    if unmeasured_looks < KIOSK_UNMEASURED_READ_LOOKS {
                        emit_scroll(None);
                        std::thread::sleep(timing.motion_interval);
                        continue;
                    }
                    // Fall through to the locator read with no measured delta: the crop
                    // placement comes from the locator alone, and the emitted null keeps
                    // the frontend from treating this publish as a measured stillness.
                    emit_scroll(None);
                    looks_unmeasured += 1;
                    unmeasured_fell_through = true;
                    0
                }
                None => {
                    unmeasured_looks = 0;
                    looks_blind += 1;
                    static_looks = 0;
                    emit_scroll(None);
                    std::thread::sleep(timing.motion_interval);
                    continue;
                }
            };
            unmeasured_looks = 0;
            if frame_delta.abs() > 1 {
                // Stream the frame's movement; the frontend accumulates the deltas. Deltas
                // need no anchor and no range, so the chips follow a scroll of any length.
                looks_motion += 1;
                emit_scroll(Some(frame_delta));
                static_looks = 0;
                std::thread::sleep(timing.motion_interval);
                continue;
            }
            // Two readable, agreeing looks mean settled. The settled read locates itself -- the
            // grid's own label rows name the offset at any scroll position, so the crops land on
            // the text instead of the gaps. An unmeasured run that fell through above bypasses
            // the two-look gate for this one read; the locator gates it on its own.
            let read_now = unmeasured_fell_through;
            unmeasured_fell_through = false;
            if !read_now {
                static_looks = static_looks.saturating_add(1);
                if static_looks < KIOSK_SETTLE_LOOKS {
                    looks_still += 1;
                    std::thread::sleep(timing.motion_interval);
                    continue;
                }
            }
            looks_still += 1;
            looks_reads += 1;
            static_looks = 0;
            let located = reading.and_then(|strip| {
                let at = kiosk_geometry::label_anchors(strip.len());
                kiosk_scroll::label_offset(strip, at.strip_top, at.first_top, at.pitch, at.band)
                    .map(|located| (located.dy, at.pitch, located.bands))
            });
            let Some((dy, pitch, bands_present)) = located else {
                // No label band anywhere in the pane: an animation frame, a capture that came
                // back torn, or a grid the player has filtered down to nothing. None of those
                // is a closed kiosk, and none of them is worth publishing over a good view.
                log::debug!("[DEBUG-kiosk] no labels located: skipping this look");
                std::thread::sleep(timing.interval);
                continue;
            };
            // The fold answers a phase, and a live pane can make that answer wrong in two
            // different ways. A sparse pane (few populated rows after sales, a hover card
            // swallowing bands) can resolve the fold's tied run a whole pitch off: every
            // crop then lands between label bands. A polluted pane skews the tie-run
            // midpoint by a fraction of a pitch (measured on the 2026-09-25 live capture:
            // 36 profile-rows of the 291-row pitch -- the open hover card's bright title
            // votes in the fold, and the pane's first label row was dimmed past the
            // profile's white gate). Both end with the published page reading the gaps:
            // nothing at all, or a couple of lookalikes ("Corinth Prime Barrel" over a
            // Receiver) whose chips then sit half a band off their cards.
            //
            // Content is the referee the fold cannot be, but the referee needs to know
            // how much text to expect -- the fold already counted it. A sparse grid at
            // the right phase is complete at two confident cells out of one rendered
            // band; a populated grid that reads a single confident cell (measured 28
            // settles straight on 2026-09-25) is misphased, not sparse. So recovery fires
            // while confident cells lag rendered bands (capped at two, so a nearly-right
            // page with one occluded row does not churn). True reads measure 0.94-1.00,
            // misphased lookalikes top out at 0.76, so conviction -- not cell count --
            // tells them apart.
            //
            // Recovery then asks in two tiers: whole pitches for the which-band error,
            // then an eighth-pitch ladder around the tier-one winner for the midpoint
            // drag. Every tier adopts by (confidence, cells): a confident read always
            // beats lookalikes, and among lookalikes the widest read stands, as before.
            // The ladder stops at the first rung that reads every rendered band -- that
            // rung is on the text, further rungs only re-read it shifted. A correction
            // that proved itself rides the next settle directly -- pane pollution does
            // not move between two settles 400ms apart -- and one that stopped reading
            // is retired in place.
            let fullness = bands_present.min(2);
            let mut read_dy = dy + phase_correction;
            let mut read = source.read_kiosk(&candidates, read_dy);
            let mut read_confidence = confident_cells(&read);
            if phase_correction != 0 && read_confidence < fullness {
                phase_correction = 0;
                read = source.read_kiosk(&candidates, dy);
                read_dy = dy;
                read_confidence = confident_cells(&read);
            }
            // Best read so far, ordered by conviction first: (confident cells, all cells).
            let mut best = (
                read_confidence,
                read.as_ref().map_or(0, |frame| frame.cells.len()),
            );
            if read.is_ok() && read_confidence < fullness {
                for shifted in [read_dy - pitch, read_dy + pitch] {
                    match source.read_kiosk(&candidates, shifted) {
                        Ok(frame) => {
                            let probe = (confident_frame_cells(&frame), frame.cells.len());
                            if probe > best {
                                best = probe;
                                read = Ok(frame);
                                read_dy = shifted;
                            }
                        }
                        Err(_) => break,
                    }
                }
                read_confidence = confident_cells(&read);
                if read_confidence < fullness {
                    // The winner still sits on the fold's midpoint; the text is within a
                    // couple of eighth-pitch rungs of it. The ladder's centre is fixed so
                    // a weak-but-wider probe cannot drag the probes off the pitch family.
                    let anchor = read_dy;
                    let step = (pitch / 8).max(1);
                    'fine: for rung in 1..=KIOSK_FINE_LADDER_RUNGS {
                        for side in [-1_i32, 1] {
                            let shifted = anchor + side * rung * step;
                            match source.read_kiosk(&candidates, shifted) {
                                Ok(frame) => {
                                    let probe_confidence = confident_frame_cells(&frame);
                                    if probe_confidence >= fullness {
                                        read = Ok(frame);
                                        read_dy = shifted;
                                        read_confidence = probe_confidence;
                                        break 'fine;
                                    }
                                    if (probe_confidence, frame.cells.len()) > best {
                                        best = (probe_confidence, frame.cells.len());
                                        read = Ok(frame);
                                        read_dy = shifted;
                                    }
                                }
                                Err(_) => break 'fine,
                            }
                        }
                    }
                }
                if read_confidence > 0 {
                    phase_correction = read_dy - dy;
                }
            }
            let dy = read_dy;
            let final_confidence = confident_cells(&read);
            match read {
                Ok(mut frame) => {
                    // A read costs the better part of a second of OCR. A close that landed
                    // while it ran means this frame describes a screen that is already gone,
                    // and publishing it would refill the state the teardown just cleared.
                    if gone.load(Ordering::Acquire) {
                        break;
                    }
                    // Basket quantities flicker frame to frame on static content: tesseract
                    // alternates phantom digit strings ("720", "8040") that parse cleanly
                    // and multiply straight into the total (measured 4070<->470 on an
                    // unchanged 8-row basket). A quantity above 1 is only trusted after two
                    // consecutive identical reads of the same slot and name; anything else
                    // publishes as 1. Rows showing x1 adopt immediately, so the common case
                    // never lags a settle.
                    for row in &mut frame.basket {
                        let key = (row.index, row.name.clone());
                        let seen = quantity_streaks.get(&key).copied().unwrap_or((1, 0));
                        if seen.0 == row.quantity {
                            let streak = seen.1.saturating_add(1);
                            quantity_streaks.insert(key, (row.quantity, streak));
                            if row.quantity != 1 && streak < 2 {
                                row.quantity = 1;
                            }
                        } else {
                            quantity_streaks.insert(key, (row.quantity, 1));
                            if row.quantity != 1 {
                                row.quantity = 1;
                            }
                        }
                    }
                    // stays empty, which is the truthful state.
                    if final_confidence == 0 && !frame.cells.is_empty() {
                        log::debug!(
                            "[DEBUG-kiosk] grid page suppressed on {} weak cells dy={dy} (basket kept)",
                            frame.cells.len()
                        );
                        frame.cells.clear();
                    }
                    let mut view = joiner(epoch, &frame);
                    view.scroll_dy = dy;
                    let cell_detail: Vec<String> = frame
                        .cells
                        .iter()
                        .map(|cell| format!("{}:{:.2}", cell.name, cell.score))
                        .collect();
                    let basket_detail: Vec<String> = frame
                        .basket
                        .iter()
                        .map(|row| format!("{} x{}", row.name, row.quantity))
                        .collect();
                    log::debug!(
                        "[DEBUG-kiosk] publish epoch={epoch} cells={} confident={final_confidence} basket={} total={} dy={dy} correction={phase_correction} looks=still:{looks_still}/motion:{looks_motion}/blind:{looks_blind}/unmeasured:{looks_unmeasured}/reads:{looks_reads}",
                        view.cells.len(),
                        view.basket.len(),
                        view.total_plat
                    );
                    log::debug!(
                        "[DEBUG-kiosk] publish detail cells=[{}] basket=[{}]",
                        cell_detail.join(", "),
                        basket_detail.join(", ")
                    );
                    looks_motion = 0;
                    looks_blind = 0;
                    looks_unmeasured = 0;
                    looks_still = 0;
                    looks_reads = 0;
                    publish(view);
                }
                Err(reason) => log::warn!("[DEBUG-kiosk] read failed: {reason}"),
            }
            std::thread::sleep(timing.interval);
        }
        log::debug!(
            "[DEBUG-kiosk] poller exit epoch={epoch} looks=still:{looks_still}/motion:{looks_motion}/blind:{looks_blind}/unmeasured:{looks_unmeasured}/reads:{looks_reads}",
        );
    })
}

pub fn release_player_record_scan(identity: &str, active_scans: &Mutex<BTreeSet<String>>) {
    if let Ok(mut active) = active_scans.lock() {
        active.remove(identity);
    }
}

pub fn rotate_choices_to_local(choices: &mut [String], local_name: &str) {
    if let Some(index) = choices.iter().position(|name| name == local_name) {
        choices.rotate_left(index);
    }
}

pub fn assemble_player_record_choices(
    responders: &[&str],
    local_identity: Option<&str>,
    local_choice: Option<&str>,
    records: &std::collections::BTreeMap<String, String>,
) -> Option<Vec<String>> {
    let local_identity = local_identity?;
    let mut choices = vec![local_choice?.to_owned()];
    for identity in responders
        .iter()
        .copied()
        .filter(|identity| *identity != local_identity)
    {
        choices.push(records.get(identity)?.clone());
    }
    (choices.len() == responders.len()).then_some(choices)
}

pub fn store_player_record_if_current(
    expected_generation: u64,
    generation: &AtomicU64,
    identity: &str,
    resolution: warframe_acquisition::RewardResolution,
    records: &Mutex<BTreeMap<String, String>>,
) {
    if generation.load(Ordering::Acquire) != expected_generation {
        return;
    }
    let warframe_acquisition::RewardResolution::Confirmed { choices, .. } = resolution else {
        return;
    };
    let [choice] = choices.as_slice() else {
        return;
    };
    if let Ok(mut records) = records.lock()
        && generation.load(Ordering::Acquire) == expected_generation
    {
        records.insert(identity.to_owned(), choice.clone());
    }
}

pub fn scan_player_record_until_ready(
    expected_generation: u64,
    generation: &AtomicU64,
    timeout: Duration,
    mut scan: impl FnMut() -> warframe_acquisition::RewardResolution,
) -> warframe_acquisition::RewardResolution {
    let started = Instant::now();
    while generation.load(Ordering::Acquire) == expected_generation {
        let resolution = scan();
        if matches!(
            &resolution,
            warframe_acquisition::RewardResolution::Confirmed { choices, .. }
                if choices.len() == 1
        ) {
            return resolution;
        }
        if started.elapsed() >= timeout {
            break;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    warframe_acquisition::RewardResolution::Incomplete
}

fn trace_responder_reward_scan(
    identity: &str,
    elapsed: Duration,
    resolution: &warframe_acquisition::RewardResolution,
) {
    let suffix = identity
        .get(identity.len().saturating_sub(6)..)
        .unwrap_or(identity);
    log::debug!(
        "[DEBUG-responder] identity=…{suffix} elapsed_ms={} resolution={resolution:?}",
        elapsed.as_millis(),
    );
}

/// Fetch platinum prices without blocking the overlay.
///
/// Ducats cannot rank relic rewards on their own, since most commons share a value; platinum is
/// what separates them. But the cards matter more than their prices, and the reward screen only
/// lives for fifteen seconds, so the overlay goes up first and the prices land when they land. The
/// cards render an em dash until then.
// Seven thread inputs (choices, runtime, window, catalog, cache, clock and generation);
// bundling them would hide that shape behind a struct built once.
#[allow(clippy::too_many_arguments)]
fn spawn_market_price_fetch(
    choices: &[RewardObservation],
    shared: &SharedRuntime,
    app: &AppHandle,
    reward_catalog: &[RewardCatalogEntry],
    price_cache: &MarketPriceCache,
    now: u64,
    generation: &MonitorGeneration,
) {
    let names = choices.to_vec();
    let price_cache = price_cache.clone();
    let app = app.clone();
    spawn_market_price_worker(
        names,
        Arc::clone(shared),
        reward_catalog.to_vec(),
        now,
        generation.clone(),
        move |choices| {
            // Anything the pool warmed while the mission was still running is already here, so the
            // common case does no requests at all and the overlay never shows a dash. Only a reward
            // the warm pass missed -- a pool that never loaded, an API that was down then -- is
            // fetched now, and it is fetched with no gap because the screen is already up.
            let mut prices = choices
                .iter()
                .filter_map(|choice| Some((choice.name.clone(), price_cache.get(&choice.name)?)))
                .collect::<BTreeMap<_, _>>();
            let missing = choices
                .iter()
                .filter(|choice| !prices.contains_key(&choice.name))
                .map(|choice| choice.name.clone())
                .collect::<Vec<_>>();
            let mut outcome = WarmOutcome::default();
            if !missing.is_empty()
                && let Some(market) = warframe_acquisition::WarframeMarketHttp::new()
            {
                outcome = price_cache.warm(&market, &missing, Duration::ZERO);
                for name in missing {
                    if let Some(price) = price_cache.get(&name) {
                        prices.insert(name, price);
                    }
                }
            }
            // An oversize response is worth saying even when the cache carried the screen, because
            // it stops every future price and nothing else would report it. An empty screen with no
            // failure to name means no request was made at all.
            let failure = outcome.failure().or_else(|| {
                prices
                    .is_empty()
                    .then_some("warframe.market pricing is unavailable for these rewards")
            });
            (prices, failure)
        },
        move || {
            let _ = app.emit_to("reward-overlay", "reward-updated", ());
        },
    );
}

// Seven inputs plus the fetch/emit closures; see the fetch above.
#[allow(clippy::too_many_arguments)]
fn spawn_market_price_worker<Fetch, Emit>(
    choices: Vec<RewardObservation>,
    shared: SharedRuntime,
    reward_catalog: Vec<RewardCatalogEntry>,
    now: u64,
    generation: MonitorGeneration,
    fetch: Fetch,
    emit: Emit,
) -> std::thread::JoinHandle<()>
where
    Fetch: FnOnce(&[RewardObservation]) -> (BTreeMap<String, u32>, Option<&'static str>)
        + Send
        + 'static,
    Emit: FnOnce() + Send + 'static,
{
    std::thread::spawn(move || {
        let (prices, failure) = fetch(&choices);
        // Delayed prices are gated by the monitor generation alone, as before: the initial
        // overlay publication already consumed the single-use epoch token, so reusing it here
        // would decline every delayed price update.
        generation.publish(|| {
            if let Some(failure) = failure
                && let Ok(mut runtime) = shared.lock()
            {
                let _ = runtime.core.record_market_degraded(failure);
            }
            if prices.is_empty() {
                return;
            }
            apply_reward_observations(&shared, &reward_catalog, &choices, &prices);
            if let Ok(mut runtime) = shared.lock() {
                let _ = runtime
                    .core
                    .record_market_ready(prices.len(), now.to_string());
            }
            emit();
        });
    })
}

/// Price the whole relic pool while the mission is still being played.
///
/// The pool is known when the relics load and the reward screen is minutes away, so there is time
/// to be unhurried and polite about it. Doing this later -- when the cards are actually on screen
/// -- is what made every card show a dash for the first seconds of a fifteen-second window.
fn spawn_market_price_warm(pool: &[RewardCatalogEntry], price_cache: &MarketPriceCache) {
    let names = pool
        .iter()
        .map(|entry| entry.name.clone())
        .collect::<Vec<_>>();
    if names.is_empty() {
        return;
    }
    let price_cache = price_cache.clone();
    std::thread::spawn(move || {
        if let Some(market) = warframe_acquisition::WarframeMarketHttp::new() {
            price_cache.warm(&market, &names, warframe_acquisition::MARKET_MIN_GAP);
        }
    });
}

fn apply_reward_observations(
    shared: &SharedRuntime,
    catalog: &[RewardCatalogEntry],
    observations: &[RewardObservation],
    prices: &BTreeMap<String, u32>,
) {
    let Ok(mut runtime) = shared.lock() else {
        return;
    };
    let Ok(view) = runtime.core.current_view() else {
        return;
    };
    let candidates = observations
        .iter()
        .filter_map(|observation| {
            let ducats = catalog
                .iter()
                .find(|entry| {
                    warframe_acquisition::reward_name_matches(&entry.name, &observation.name)
                })
                .map_or(0, |entry| entry.ducats);
            let owned = view
                .collection()
                .items()
                .iter()
                .find(|item| {
                    warframe_acquisition::reward_name_matches(item.name(), &observation.name)
                })
                .map_or(0, |item| item.quantity());
            RewardCandidate::new(
                &observation.name,
                prices.get(&observation.name).copied().unwrap_or(0),
                ducats,
                owned,
                false,
                observation.confidence,
            )
            .ok()
        })
        .collect();
    let _ = runtime.core.apply_reward_candidates(candidates);
}

/// A string that stays the same while the log grows and changes when the log is replaced.
///
/// The monitor resumes at a byte offset, so it has to be able to tell "the same file, longer" from
/// "a new file that happens to be at the same path" -- getting that wrong either re-reads the whole
/// log or silently skips the start of a new one.
///
/// This was `dev:ino`, which is exactly the right answer and does not exist on Windows. Creation
/// time is the portable stand-in: the game rotates `EE.log` by writing a new file, which gets a new
/// creation time, while appending to the open one does not. Where the platform has no creation time
/// the path alone still distinguishes logs; only rotation-in-place goes unnoticed, and the length
/// check the caller already does catches the truncation that comes with it.
///
/// Seconds, not the full precision the platform offers: under Wine the reported creation time
/// jitters by a few hundred microseconds between reads of the same unmodified file, which would
/// make every poll look like a rotation and re-read the log from zero. A rotation and the append
/// before it cannot share a second and also matter -- the replacement log starts empty, so the
/// length check catches it either way.
pub fn log_identity(path: &Path, metadata: &fs::Metadata) -> String {
    let created = metadata
        .created()
        .ok()
        .and_then(|created| created.duration_since(UNIX_EPOCH).ok())
        .map(|since| since.as_secs());
    match created {
        Some(created) => format!("{}:{created}", path.display()),
        None => path.display().to_string(),
    }
}

/// Whether the log line about the followed EE.log path differs from the last one emitted.
///
/// The resolution debug line exists to explain which game the monitor is following (Wine prefix
/// surprises are the usual cause for confusion), so it must print when that state changes --
/// pid found, pid lost, another pid, another path -- and stay silent while the same path is
/// being polled at up to ten times a second.
fn monitor_path_changed(
    tracked: Option<&(u32, Option<PathBuf>)>,
    pid: u32,
    path: Option<&Path>,
) -> bool {
    match (tracked, path) {
        (None, _) => true,
        (Some((tracked_pid, _)), None) => *tracked_pid != pid,
        (Some((tracked_pid, Some(tracked))), Some(path)) => {
            *tracked_pid != pid || tracked.as_path() != path
        }
        (Some(_), Some(_)) => false,
    }
}

/// How much of EE.log to fold when recovering a kiosk that was already on screen at attach.
/// Enough to carry a visit's open marker through the game's ordinary chatter (the kiosk logs
/// nothing while the player reads, and the surrounding traffic is a few hundred KB a minute)
/// without replaying an evening of history into anything.
const KIOSK_TAIL_SCAN: u64 = 1024 * 1024;

/// The last stretch of a file, for one-shot state recovery. Any failure reads as "no tail":
/// the caller simply learns nothing, which is where it started.
fn log_tail(path: &Path, limit: u64) -> Vec<u8> {
    let Ok(metadata) = fs::metadata(path) else {
        return Vec::new();
    };
    let len = metadata.len();
    let start = len.saturating_sub(limit);
    let Ok(mut file) = fs::File::open(path) else {
        return Vec::new();
    };
    if file.seek(SeekFrom::Start(start)).is_err() {
        return Vec::new();
    }
    let mut bytes = Vec::with_capacity((len - start) as usize);
    if file.take(len - start).read_to_end(&mut bytes).is_err() {
        return Vec::new();
    }
    bytes
}

pub fn build_monitor_input(
    machine: &MonitorMachine,
    now: u64,
    pid: u32,
    path: Option<PathBuf>,
) -> (MonitorInput, Vec<u8>) {
    let Some(path) = path else {
        return (MonitorInput::running(now, pid, None), Vec::new());
    };
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) => {
            log::warn!("monitor: EE.log open failed: {error}");
            return (MonitorInput::running_with_log_error(now, pid), Vec::new());
        }
    };
    let identity = log_identity(&path, &metadata);
    if machine.process_pid() != Some(pid) {
        return (
            MonitorInput::running(
                now,
                pid,
                Some(LogObservation::new(identity, metadata.len(), Vec::new())),
            ),
            Vec::new(),
        );
    }
    let offset = if machine.log_identity() == Some(identity.as_str())
        && metadata.len() >= machine.log_offset()
    {
        machine.log_offset()
    } else {
        0
    };
    if metadata.len() == offset {
        return (
            MonitorInput::running(
                now,
                pid,
                Some(LogObservation::new(identity, offset, Vec::new())),
            ),
            Vec::new(),
        );
    }
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) => {
            log::warn!("monitor: EE.log open failed: {error}");
            return (MonitorInput::running_with_log_error(now, pid), Vec::new());
        }
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return (MonitorInput::running_with_log_error(now, pid), Vec::new());
    }
    let requested = (metadata.len() - offset).min(1024 * 1024);
    let mut bytes = Vec::with_capacity(requested as usize);
    if file.take(requested).read_to_end(&mut bytes).is_err() {
        return (MonitorInput::running_with_log_error(now, pid), Vec::new());
    }
    // A read from zero means the log changed identity under the same process: a rotation, or the
    // path resolution settling on a different Wine prefix's EE.log. Everything from before this
    // process was attached is not this session's events, and replaying it as if it were is the
    // whole of the 2026-08-22 ghost report -- an hours-old fissure armed the poller, ran the
    // reward pipeline against a screen that was not there, and left health degraded for a game
    // that was never running.
    let mut observation_len = offset + bytes.len() as u64;
    if offset == 0 {
        let created_unix = metadata
            .created()
            .ok()
            .and_then(|created| created.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|since| since.as_secs());
        match ee_log_rotation_keep_from(&bytes, created_unix, machine.attached_since()) {
            Some(0) => {}
            Some(keep) => {
                log::warn!(
                    "monitor: EE.log changed identity under the game process; skipping {} bytes \
                     that predate this session",
                    keep
                );
                bytes.drain(..keep);
            }
            None => {
                log::warn!(
                    "monitor: EE.log changed identity to a log from an earlier session; skipping \
                     all {} bytes",
                    bytes.len()
                );
                bytes.clear();
                // Past this read, not just up to it: a stale file larger than the read cap would
                // otherwise hand its remainder over one incremental chunk at a time.
                observation_len = metadata.len();
            }
        }
    }
    let log_bytes = bytes.clone();
    (
        MonitorInput::running(
            now,
            pid,
            Some(LogObservation::new(identity, observation_len, bytes)),
        ),
        log_bytes,
    )
}

/// A generation token captured by every worker and publication owned by one monitor run.
#[derive(Clone, Debug)]
pub struct MonitorGeneration {
    id: u64,
    current: Arc<AtomicU64>,
    stop: Arc<std::sync::atomic::AtomicBool>,
    publication: Arc<Mutex<()>>,
}

impl MonitorGeneration {
    pub fn new(id: u64, current: Arc<AtomicU64>) -> Self {
        current.store(id, Ordering::Release);
        Self {
            id,
            current,
            stop: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            publication: Arc::new(Mutex::new(())),
        }
    }

    pub const fn id(&self) -> u64 {
        self.id
    }

    pub fn request_stop(&self) {
        let _publication = self
            .publication
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        self.stop.store(true, Ordering::Release);
    }

    pub fn is_current(&self) -> bool {
        !self.stop.load(Ordering::Acquire) && self.current.load(Ordering::Acquire) == self.id
    }

    pub fn publish(&self, publication: impl FnOnce()) -> bool {
        let _publication = self
            .publication
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !self.is_current() {
            return false;
        }
        publication();
        true
    }
}

/// Minimal lifecycle body used by contract tests without constructing a Tauri application.
pub fn spawn_generation_worker(
    generation: MonitorGeneration,
    started: std::sync::mpsc::Sender<u64>,
    retired: std::sync::mpsc::Sender<u64>,
) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let _ = started.send(generation.id());
        while generation.is_current() {
            std::thread::park_timeout(Duration::from_millis(10));
        }
        let _ = retired.send(generation.id());
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use std::sync::atomic::AtomicBool;

    /// A scripted screen for the kiosk poller: each `pop` is one look, so a test can stage
    /// capture loss, occlusions and scrolls without playing the game.
    struct ScriptedKiosk {
        looks: StdMutex<Vec<Result<KioskRead, &'static str>>>,
        profiles: StdMutex<Vec<Option<Vec<f32>>>>,
        /// Set as a read begins: the log's close line landing while OCR is still running.
        closes_mid_read: Option<Arc<AtomicBool>>,
        /// When set, `read_kiosk` answers from this per-dy map instead of popping `looks`:
        /// the read at the map's key returns its value, any other dy returns an empty page.
        /// For pinning the poller's phase-retry against content.
        reads_by_dy: Option<StdMutex<std::collections::HashMap<i32, KioskRead>>>,
        /// Every dy a read was requested for, in order; lets a test count what the retry
        /// ladders probe rather than inferring it from outcomes.
        read_log: Option<Arc<StdMutex<Vec<i32>>>>,
    }

    impl ScriptedKiosk {
        fn new(looks: Vec<Result<KioskRead, &'static str>>) -> Self {
            Self {
                looks: StdMutex::new(looks),
                profiles: StdMutex::new(vec![None]),
                closes_mid_read: None,
                reads_by_dy: None,
                read_log: None,
            }
        }

        fn with_profiles(mut self, profiles: Vec<Option<Vec<f32>>>) -> Self {
            self.profiles = StdMutex::new(profiles);
            self
        }

        fn recorder_reads(mut self, log: &Arc<StdMutex<Vec<i32>>>) -> Self {
            self.read_log = Some(Arc::clone(log));
            self
        }

        fn closing_mid_read(mut self, gone: &Arc<AtomicBool>) -> Self {
            self.closes_mid_read = Some(Arc::clone(gone));
            self
        }

        fn with_reads_by_dy(mut self, reads: Vec<(i32, KioskRead)>) -> Self {
            self.reads_by_dy = Some(StdMutex::new(reads.into_iter().collect()));
            self
        }
    }

    impl KioskFrameSource for ScriptedKiosk {
        fn read_kiosk(
            &mut self,
            _candidates: &[RewardCatalogEntry],
            dy: i32,
        ) -> Result<KioskRead, &'static str> {
            if let Some(log) = &self.read_log {
                log.lock().expect("read log").push(dy);
            }
            if let Some(gone) = &self.closes_mid_read {
                gone.store(true, Ordering::Release);
            }
            if let Some(reads) = &self.reads_by_dy {
                // A dy-keyed script answers the mapped dy and an empty page otherwise, so a
                // test can stand in for a misphased locator: one dy holds the grid, the
                // neighbours read blank.
                return Ok(reads
                    .lock()
                    .expect("reads by dy")
                    .get(&dy)
                    .cloned()
                    .unwrap_or_default());
            }
            // An exhausted script is a vanished screen: the read fails instead of a panic
            // inside the poller thread. It is NOT a close -- only the log ends a session.
            self.looks
                .lock()
                .expect("looks")
                .pop()
                .unwrap_or(Err("script exhausted"))
        }

        fn strip_profile(&mut self) -> Result<Vec<f32>, &'static str> {
            match self.profiles.lock().expect("profiles").pop() {
                Some(Some(profile)) => Ok(profile),
                Some(None) => Err("strip unavailable"),
                None => Err("profile script exhausted"),
            }
        }
    }

    #[test]
    fn kiosk_capture_falls_through_to_the_candidate_with_content() {
        use reward_capture::{CapturedFrame, FrameBackend, RectOrigin};

        let frame = |x, image: image::DynamicImage| CapturedFrame {
            rect: overlay_window::WindowRect {
                x,
                y: 0,
                width: image.width(),
                height: image.height(),
            },
            image,
            rect_origin: RectOrigin::Wayland,
            frame_backend: FrameBackend::Portal,
        };
        let blank = image::DynamicImage::ImageRgba8(image::RgbaImage::new(1920, 1080));
        let kiosk = image::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/kiosk/kiosk-open.png"
        ))
        .expect("kiosk fixture");
        reward_ocr::clear_latest_matched_rect();

        let (selected, profile) = select_kiosk_strip(vec![frame(0, blank), frame(1920, kiosk)])
            .expect("second monitor contains the kiosk");

        assert_eq!(selected.rect.x, 1920);
        assert!(!profile.is_empty());
        assert_eq!(reward_ocr::latest_matched_rect(), Some(selected.rect));
    }

    fn scripted_cell(name: &str) -> GridCell {
        GridCell {
            col: 0,
            row: 0,
            name: name.to_owned(),
            score: 0.95,
        }
    }

    /// A pane profile at 1080p scale: 790 rows (the whole pane, 193 to 983), bands on the
    /// grid's pitch, two 11-row text lines per band at +6 and +27, the first band at the
    /// calibration position (strip row 150 = absolute 343) -- which `label_offset` reads as
    /// offset zero.
    fn label_strip() -> Vec<f32> {
        label_strip_at(0)
    }

    /// The label strip with its bands moved by `dy` rows, the way a scrolled grid's would be.
    fn label_strip_at(dy: i64) -> Vec<f32> {
        let mut rows = vec![2.0_f32; 790];
        let mut top = 150 + dy;
        while top < 790 {
            if top >= 0 {
                for line in [6_i64, 27] {
                    for row in 0..11 {
                        let at = top + line + row;
                        if at >= 0 && (at as usize) < 790 {
                            rows[at as usize] = 250.0;
                        }
                    }
                }
            }
            top += 222;
        }
        rows
    }

    fn run_poller_with<S>(
        source: S,
        candidates: Vec<RewardCatalogEntry>,
    ) -> (
        Arc<AtomicBool>,
        Arc<AtomicBool>,
        Arc<StdMutex<Vec<KioskView>>>,
    )
    where
        S: KioskFrameSource + Send + 'static,
    {
        let reanchor = Arc::new(AtomicBool::new(false));
        let gone = Arc::new(AtomicBool::new(false));
        let published: Arc<StdMutex<Vec<KioskView>>> = Arc::new(StdMutex::new(Vec::new()));
        let deltas: Arc<StdMutex<Vec<Option<i32>>>> = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::clone(&published);
        let delta_sink = Arc::clone(&deltas);
        let handle = spawn_kiosk_poller_with(
            &reanchor,
            &gone,
            KioskPollerTiming {
                interval: Duration::from_millis(1),
                motion_interval: Duration::from_millis(1),
                lifetime: Duration::from_millis(400),
            },
            Arc::new(candidates),
            |epoch, read| {
                // Price everything so the join keeps the scripted cells visible.
                crate::kiosk_view::build_view(epoch, &read.cells, &read.basket, |_| Some(1))
            },
            move |view| sink.lock().expect("published").push(view),
            move |delta| delta_sink.lock().expect("deltas").push(delta),
            move || source,
        );
        handle.join().expect("poller thread");
        (reanchor, gone, published)
    }

    fn run_poller<S>(
        source: S,
    ) -> (
        Arc<AtomicBool>,
        Arc<AtomicBool>,
        Arc<StdMutex<Vec<KioskView>>>,
    )
    where
        S: KioskFrameSource + Send + 'static,
    {
        run_poller_with(source, Vec::new())
    }

    #[test]
    fn a_failed_read_is_followed_by_a_good_one() {
        // The first settle read fails, the next publishes: one miss has never meant the screen
        // closed. The script runs dry afterwards, which is just more failed reads -- what
        // matters is that the bad read in between did not stop the publish.
        // Pops come off the back: the first settle read fails, the second is the good frame.
        let source = ScriptedKiosk::new(vec![
            Ok(KioskRead {
                cells: vec![scripted_cell("Titania Prime Systems Blueprint")],
                basket: vec![],
            }),
            Err("window gone"),
        ])
        .with_profiles(vec![Some(label_strip()); 8]);
        let (_, _, published) = run_poller(source);
        let published = published.lock().expect("published");
        assert_eq!(published.len(), 1, "the good read after the miss published");
        assert_eq!(
            published[0].cells[0].name,
            "Titania Prime Systems Blueprint"
        );
    }

    /// The poller does not decide whether the kiosk is there. A pane it cannot read costs that
    /// look and nothing else -- no close, no cleared view -- because EE.log owns presence and
    /// says so within ~150ms. (Inferring presence from the reader's failures is what tore the
    /// overlay down mid-session all through 2026-08-23.)
    #[test]
    fn an_unreadable_pane_does_not_end_the_session() {
        let source = ScriptedKiosk::new(vec![]).with_profiles(vec![Some(vec![2.0; 790]); 40]);
        let (_, gone, published) = run_poller(source);
        assert!(!gone.load(Ordering::Acquire), "no labels is not a close");
        assert!(published.lock().expect("published").is_empty());
    }

    /// A capture that fails outright is the same kind of nothing: the last good view stands
    /// and the next look tries again.
    #[test]
    fn failed_captures_do_not_end_the_session() {
        let source = ScriptedKiosk::new(vec![Err("window gone"), Err("window gone")])
            .with_profiles(vec![Some(label_strip()); 40]);
        let (_, gone, _) = run_poller(source);
        assert!(!gone.load(Ordering::Acquire));
    }

    /// A close that lands while the reader is inside tesseract must not publish what it was
    /// holding. The stop is checked at the top of a tick, but a read costs the better part of a
    /// second, so the frame in flight describes a screen that is already gone: publishing it
    /// refills the state the teardown just cleared, and the next visit opens on the previous
    /// grid's chips until its own first read lands.
    #[test]
    fn a_close_during_a_read_publishes_nothing() {
        let reanchor = Arc::new(AtomicBool::new(false));
        let gone = Arc::new(AtomicBool::new(false));
        let published: Arc<StdMutex<Vec<KioskView>>> = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::clone(&published);
        let source = ScriptedKiosk::new(vec![Ok(KioskRead {
            cells: vec![scripted_cell("Stale by the time it lands")],
            basket: vec![],
        })])
        .with_profiles(vec![Some(label_strip()); 6])
        .closing_mid_read(&gone);
        let handle = spawn_kiosk_poller_with(
            &reanchor,
            &gone,
            KioskPollerTiming {
                interval: Duration::from_millis(1),
                motion_interval: Duration::from_millis(1),
                lifetime: Duration::from_millis(200),
            },
            Arc::new(Vec::new()),
            |epoch, read| {
                crate::kiosk_view::build_view(epoch, &read.cells, &read.basket, |_| Some(1))
            },
            move |view| sink.lock().expect("published").push(view),
            |_| (),
            move || source,
        );
        handle.join().expect("poller thread");
        assert!(
            published.lock().expect("published").is_empty(),
            "a frame read across the close belongs to a screen that is gone"
        );
    }

    #[test]
    fn a_reanchor_accepts_a_partially_filled_grid() {
        // The player narrowed a filter: the epoch advanced and the next read holds fewer
        // cells. That is not an occlusion -- publish what is there.
        let reanchor = Arc::new(AtomicBool::new(true));
        let gone = Arc::new(AtomicBool::new(false));
        let published: Arc<StdMutex<Vec<KioskView>>> = Arc::new(StdMutex::new(Vec::new()));
        let sink = Arc::clone(&published);
        let source = ScriptedKiosk::new(vec![Ok(KioskRead {
            cells: vec![scripted_cell("Only survivor")],
            basket: vec![],
        })])
        .with_profiles(vec![Some(label_strip()); 6]);
        let handle = spawn_kiosk_poller_with(
            &reanchor,
            &gone,
            KioskPollerTiming {
                interval: Duration::from_millis(1),
                motion_interval: Duration::from_millis(1),
                lifetime: Duration::from_millis(100),
            },
            Arc::new(Vec::new()),
            |epoch, read| {
                crate::kiosk_view::build_view(epoch, &read.cells, &read.basket, |_| Some(1))
            },
            move |view| sink.lock().expect("published").push(view),
            |_| {},
            move || source,
        );
        handle.join().expect("poller thread");
        let published = published.lock().expect("published");
        assert_eq!(
            published.len(),
            1,
            "the narrowed grid published on the fresh anchor"
        );
        assert_eq!(published[0].cells.len(), 1);
    }
    /// A sparse pane can resolve the locator's fold one whole pitch off, and the crops
    /// then land between label bands: the located read sees nothing, while the
    /// dy-independent basket reads fine underneath. Content is the referee: the located
    /// read scores no confident match, the neighbouring phases are tried, and whichever
    /// reads the grid with conviction wins -- and the published offset follows it.
    #[test]
    fn a_misphased_locator_recovers_by_reading_neighbouring_phases() {
        // The strip's true phase is 0, so the locator names dy=0; the grid's text, though,
        // sits one pitch down (the fold misphased), so only dy=+222 reads cells.
        let full_page = KioskRead {
            cells: vec![
                scripted_cell("Recovered row one"),
                scripted_cell("Recovered row two"),
                scripted_cell("Recovered row three"),
                scripted_cell("Recovered row four"),
                scripted_cell("Recovered row five"),
            ],
            basket: vec![],
        };
        // The locator's answer carries its plateau slack, so key the recovery phase on
        // whatever it actually names for this strip rather than assuming 0.
        let anchors = kiosk_geometry::label_anchors(label_strip().len());
        let located = kiosk_scroll::label_offset(
            &label_strip(),
            anchors.strip_top,
            anchors.first_top,
            anchors.pitch,
            anchors.band,
        )
        .expect("the calibration strip locates")
        .dy;
        let source = ScriptedKiosk::new(vec![])
            .with_profiles(vec![Some(label_strip()); 6])
            .with_reads_by_dy(vec![(located + anchors.pitch, full_page)]);
        let (_, _, published) = run_poller(source);
        let published = published.lock().expect("published");
        assert!(
            !published.is_empty(),
            "the poller publishes once the content names the phase"
        );
        assert_eq!(published[0].cells.len(), 5);
        assert_eq!(
            published[0].scroll_dy,
            located + anchors.pitch,
            "the published offset follows the phase the text was found at"
        );
    }

    /// An animated-but-present pane defeats frame-to-frame correlation without moving
    /// the grid: hover-card renders, dialog pulses. Those looks are unmeasurable, not
    /// blind -- the strip read and the locator can still name the bands -- so after a
    /// short run the poller reads anyway, anchored on the locator.
    #[test]
    fn an_unmeasurable_but_present_strip_publishes_after_a_run() {
        // The same locatable strip with a large hover-card block rendered into it: the
        // block's uniform brightness drowns the frame-to-frame correlation (the estimate
        // answers None), exactly as an in-place animation does, while the label bands
        // stay intact for the locator.
        let hover_block = |with: bool| {
            let mut strip = label_strip();
            if with {
                for row in strip.iter_mut().take(500).skip(300) {
                    *row = 100.0;
                }
            }
            strip
        };
        // Pops come off the back: alternate absent/block looks, unmeasurable every pair.
        let source = ScriptedKiosk::new(vec![
            Ok(KioskRead {
                cells: vec![scripted_cell("Animated hover row")],
                basket: vec![],
            }),
            Err("script exhausted"),
        ])
        .with_profiles(vec![
            Some(hover_block(false)),
            Some(hover_block(true)),
            Some(hover_block(false)),
            Some(hover_block(true)),
            Some(hover_block(false)),
            Some(hover_block(true)),
        ]);
        let (_, _, published) = run_poller(source);
        let published = published.lock().expect("published");
        assert!(
            !published.is_empty(),
            "an unmeasurable-but-present strip still publishes anchored on the locator"
        );
    }

    /// The fold answers a phase by vote, and rows that are not label text can outvote it:
    /// a hover card's bright title inside the strip, a top row dimmed past the profile's
    /// white threshold. On the 2026-09-24 live frame its answer sat ~36 rows off the text --
    /// a fraction of the 291-row pitch, so no whole-pitch retry could ever reach it, and the
    /// published page was a couple of near-miss lookalikes (0.6-0.76 score) floating beside
    /// their cards. Content referees again, on a finer ladder: while nothing reads with
    /// conviction, step an eighth of a pitch to either side of the winner.
    #[test]
    fn a_fractionally_misphased_locator_is_fine_tuned_by_content() {
        let full_page = KioskRead {
            cells: vec![
                scripted_cell("Fine-tuned row one"),
                scripted_cell("Fine-tuned row two"),
                scripted_cell("Fine-tuned row three"),
                scripted_cell("Fine-tuned row four"),
                scripted_cell("Fine-tuned row five"),
            ],
            basket: vec![],
        };
        let weak_page = KioskRead {
            cells: (0..3)
                .map(|i| GridCell {
                    col: 0,
                    row: 0,
                    name: format!("near-miss lookalike {i}"),
                    score: 0.70,
                })
                .collect(),
            basket: vec![],
        };
        let anchors = kiosk_geometry::label_anchors(label_strip().len());
        let located = kiosk_scroll::label_offset(
            &label_strip(),
            anchors.strip_top,
            anchors.first_top,
            anchors.pitch,
            anchors.band,
        )
        .expect("the calibration strip locates")
        .dy;
        let step = anchors.pitch / 8;
        let source = ScriptedKiosk::new(vec![])
            .with_profiles(vec![Some(label_strip()); 6])
            .with_reads_by_dy(vec![
                (located, weak_page),
                // Whole-pitch neighbours hold nothing, as they must: they share the
                // misphase. The text lives one fine step away.
                (located - step, full_page),
            ]);
        let (_, _, published) = run_poller(source);
        let published = published.lock().expect("published");
        assert!(
            !published.is_empty(),
            "the fine ladder publishes the read the text was found at"
        );
        assert_eq!(published[0].cells.len(), 5);
        assert_eq!(
            published[0].scroll_dy,
            located - step,
            "the published offset follows the fine-tuned phase"
        );
    }

    /// The 2026-09-25 live failure behind the gate: a populated grid that reads a single
    /// confident cell, settle after settle. One lucky slot (or one confident lookalike)
    /// must not certify the phase -- the fold saw three bands here, so one confident
    /// cell is evidence of misphase, and the ladder must run until the page reads.
    #[test]
    fn a_lone_confident_cell_on_a_full_pane_still_recovers() {
        let full_page = KioskRead {
            cells: vec![
                scripted_cell("Recovered row one"),
                scripted_cell("Recovered row two"),
                scripted_cell("Recovered row three"),
                scripted_cell("Recovered row four"),
                scripted_cell("Recovered row five"),
            ],
            basket: vec![],
        };
        let lone_page = KioskRead {
            cells: vec![GridCell {
                col: 0,
                row: 0,
                name: "lone lookalike-or-lucky-slot".to_owned(),
                score: 0.90,
            }],
            basket: vec![],
        };
        let anchors = kiosk_geometry::label_anchors(label_strip().len());
        let located = kiosk_scroll::label_offset(
            &label_strip(),
            anchors.strip_top,
            anchors.first_top,
            anchors.pitch,
            anchors.band,
        )
        .expect("the calibration strip locates")
        .dy;
        let step = anchors.pitch / 8;
        let source = ScriptedKiosk::new(vec![])
            .with_profiles(vec![Some(label_strip()); 6])
            .with_reads_by_dy(vec![(located, lone_page), (located - step, full_page)]);
        let (_, _, published) = run_poller(source);
        let published = published.lock().expect("published");
        assert!(
            !published.is_empty(),
            "a full pane reading one confident cell must still recover"
        );
        assert_eq!(published[0].cells.len(), 5);
        assert_eq!(
            published[0].scroll_dy,
            located - step,
            "the published offset follows the fine-tuned phase"
        );
    }

    fn basket_page(quantities: &[(usize, &str, u32)]) -> KioskRead {
        KioskRead {
            cells: vec![],
            basket: quantities
                .iter()
                .map(|(index, name, quantity)| BasketRow {
                    index: *index,
                    name: (*name).to_owned(),
                    score: 0.99,
                    quantity: *quantity,
                })
                .collect(),
        }
    }

    /// Basket quantities flicker frame to frame on static content (measured 4070<->470
    /// totals on an unchanged 8-row basket): a quantity above 1 is only trusted after two
    /// consecutive identical reads, so phantom digit strings can never reach the total.
    #[test]
    fn a_flapping_quantity_never_reaches_the_total() {
        let flap_a = basket_page(&[(0usize, "Ninkondi Prime Handle", 720u32)]);
        let flap_b = basket_page(&[(0usize, "Ninkondi Prime Handle", 8040u32)]);
        // Pops come off the back, so this alternates A,B,A,B... per read. Every settle
        // burns up to 7 pops (located + pitch pair + ladder) before publishing once.
        let source = ScriptedKiosk::new(vec![
            Ok(flap_b.clone()),
            Ok(flap_a.clone()),
            Ok(flap_b.clone()),
            Ok(flap_a.clone()),
            Ok(flap_b.clone()),
            Ok(flap_a.clone()),
            Ok(flap_b.clone()),
            Ok(flap_a.clone()),
        ])
        .with_profiles(vec![Some(label_strip()); 4]);
        let (_, _, published) = run_poller(source);
        let published = published.lock().expect("published");
        assert_eq!(published.len(), 2, "two settles publish: {published:?}");
        assert!(
            published.iter().all(|view| view.total_plat <= 1),
            "no phantom multiplier may reach the total: {published:?}"
        );
    }

    /// A stable multi-stack is trusted on its second identical sighting: the first
    /// publish conservatively shows x1, the next shows the proven count.
    #[test]
    fn a_stable_quantity_is_adopted_on_repeat() {
        let steady = basket_page(&[(3usize, "Ninkondi Prime Handle", 4u32)]);
        // Settle 1 burns 7 pops finding nothing better than the located read; settle 2
        // spends its single remaining pop on the located read itself.
        let source =
            ScriptedKiosk::new(vec![Ok(steady); 8]).with_profiles(vec![Some(label_strip()); 4]);
        let (_, _, published) = run_poller(source);
        let published = published.lock().expect("published");
        assert_eq!(published.len(), 2, "two settles publish: {published:?}");
        assert_eq!(published[0].total_plat, 1);
        assert_eq!(published[1].total_plat, 4);
    }

    /// A misphased pane stays misphased the same way until the content moves: once a settle
    /// has paid the ladder for a correction, the next settle must not pay it again. The first
    /// read tries the corrected phase directly, and the publish cadence keeps its shape.
    #[test]
    fn a_proven_phase_correction_is_reused_while_it_keeps_reading() {
        let full_page = KioskRead {
            cells: vec![
                scripted_cell("Corrected row one"),
                scripted_cell("Corrected row two"),
                scripted_cell("Corrected row three"),
                scripted_cell("Corrected row four"),
                scripted_cell("Corrected row five"),
            ],
            basket: vec![],
        };
        let weak_page = KioskRead {
            cells: vec![GridCell {
                col: 0,
                row: 0,
                name: "near-miss lookalike".to_owned(),
                score: 0.70,
            }],
            basket: vec![],
        };
        let anchors = kiosk_geometry::label_anchors(label_strip().len());
        let located = kiosk_scroll::label_offset(
            &label_strip(),
            anchors.strip_top,
            anchors.first_top,
            anchors.pitch,
            anchors.band,
        )
        .expect("the calibration strip locates")
        .dy;
        let step = anchors.pitch / 8;
        let asked: Arc<StdMutex<Vec<i32>>> = Arc::new(StdMutex::new(Vec::new()));
        let source = ScriptedKiosk::new(vec![])
            .with_profiles(vec![Some(label_strip()); 4])
            .with_reads_by_dy(vec![(located, weak_page), (located - step, full_page)])
            .recorder_reads(&asked);
        let (_, _, published) = run_poller(source);
        let published = published.lock().expect("published");
        assert_eq!(published.len(), 2, "two settles, two publishes");
        let asked = asked.lock().expect("read log");
        assert_eq!(
            asked.len(),
            5,
            "first settle pays the ladder ({located}, ±pitch, then -step wins and the ladder stops); the second settle reads once: {asked:?}"
        );
        assert_eq!(
            asked[4],
            located - step,
            "the second settle's first read is the corrected phase"
        );
    }

    /// Two genuine items on an otherwise empty page read with conviction at the right phase;
    /// that is a healthy sparse pane, never a misphase. No pitch probes, no fine ladder:
    /// every settle is one read.
    #[test]
    fn a_sparse_confident_read_skips_recovery_probes() {
        let page = KioskRead {
            cells: vec![
                scripted_cell("Sparse item one"),
                scripted_cell("Sparse item two"),
            ],
            basket: vec![],
        };
        let anchors = kiosk_geometry::label_anchors(label_strip().len());
        let located = kiosk_scroll::label_offset(
            &label_strip(),
            anchors.strip_top,
            anchors.first_top,
            anchors.pitch,
            anchors.band,
        )
        .expect("the calibration strip locates")
        .dy;
        let asked: Arc<StdMutex<Vec<i32>>> = Arc::new(StdMutex::new(Vec::new()));
        let source = ScriptedKiosk::new(vec![])
            .with_profiles(vec![Some(label_strip()); 4])
            .with_reads_by_dy(vec![(located, page)])
            .recorder_reads(&asked);
        let (_, _, published) = run_poller(source);
        let published = published.lock().expect("published");
        assert_eq!(published.len(), 2, "two settles, two publishes");
        assert_eq!(published[0].cells.len(), 2);
        let asked = asked.lock().expect("read log");
        assert_eq!(
            asked.as_slice(),
            &[located, located],
            "each settle is exactly one read, at the located phase: {asked:?}"
        );
    }

    /// When no probed phase reads with conviction, the page on screen is only lookalikes:
    /// names that half-match the crop's partial text. Publishing them puts real-looking
    /// prices on the wrong cards (the 2026-09-25 report), which is strictly worse than
    /// showing no chips -- so the publish strips the grid cells. The basket still publishes:
    /// it does not share the phase, so the misphase never wrongs it.
    #[test]
    fn an_unproven_page_is_published_without_its_lookalike_cells() {
        let always_weak = |name: &str| GridCell {
            col: 0,
            row: 0,
            name: name.to_owned(),
            score: 0.70,
        };
        let weak_page = KioskRead {
            cells: vec![always_weak("lookalike A"), always_weak("lookalike B")],
            basket: vec![],
        };
        let good_page = KioskRead {
            cells: vec![scripted_cell("True Label")],
            basket: vec![],
        };
        let anchors = kiosk_geometry::label_anchors(label_strip().len());
        let located = kiosk_scroll::label_offset(
            &label_strip(),
            anchors.strip_top,
            anchors.first_top,
            anchors.pitch,
            anchors.band,
        )
        .expect("the calibration strip locates")
        .dy;
        // First settle reads weak everywhere; second settle's correction retry at the same
        // located phase still reads weak, and the surviving ladder rung finds the true page.
        let step = anchors.pitch / 8;
        let source = ScriptedKiosk::new(vec![])
            .with_profiles(vec![Some(label_strip()); 4])
            .with_reads_by_dy(vec![
                (located, weak_page.clone()),
                (located - step, weak_page),
                (located + step, good_page),
            ]);
        let (_, _, published) = run_poller(source);
        let published = published.lock().expect("published");
        assert!(
            published
                .first()
                .is_some_and(|view| view.cells.iter().all(|cell| cell.name == "True Label")),
            "the first publish must already be the proven page: {published:?}"
        );
        assert!(
            published.iter().all(|view| view
                .cells
                .iter()
                .all(|cell| cell.name != "lookalike A" && cell.name != "lookalike B")),
            "no publish may ever contain a lookalike cell"
        );
    }

    /// The total-failure tail of that contract: when no rung of any tier ever reads a
    /// confident cell (a pane too degraded to read at all), the publish still happens --
    /// an empty grid keeps the basket's prices visible -- but its cell list is empty,
    /// never garbage.
    #[test]
    fn a_never_confident_read_publishes_with_no_cells() {
        let weak_page = KioskRead {
            cells: vec![GridCell {
                col: 0,
                row: 0,
                name: "lookalike C".to_owned(),
                score: 0.70,
            }],
            basket: vec![BasketRow {
                index: 0,
                name: "Ninkondi Prime Handle".to_owned(),
                score: 0.99,
                quantity: 4,
            }],
        };
        let source =
            ScriptedKiosk::new(vec![Ok(weak_page)]).with_profiles(vec![Some(label_strip()); 4]);
        let (_, _, published) = run_poller(source);
        let published = published.lock().expect("published");
        assert!(
            !published.is_empty(),
            "a readable pane with an unreadable grid still publishes its basket"
        );
        assert!(
            published.iter().all(|view| view.cells.is_empty()),
            "no publish may carry the weak cells: {published:?}"
        );
        assert!(
            published.iter().all(|view| view.basket.len() == 1),
            "the basket read is dy-independent and must survive: {published:?}"
        );
    }

    /// End-to-end against the real polluted capture from the 2026-09-25 incident: an open
    /// hover card's title text votes in the fold while the pane's top label row is dimmed
    /// out of the profile's white gate, so the bare locator answers a midpoint that is 36
    /// profile rows off the text (dy=266) and the read comes back as three lookalikes.
    /// Recovery must land on a readable phase and keep the chips on their cards.
    #[test]
    fn the_polluted_live_frame_publishes_the_readable_phase() {
        let frame = image::open(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/tests/fixtures/kiosk/kiosk-hover-polluted.png"
        ))
        .expect("live polluted frame fixture");
        // True labels on the frame, plus decoys that a misphased crop actually produced
        // (the lookalikes the broken publish shipped on 2026-09-25).
        let candidates: Vec<RewardCatalogEntry> = [
            "Corinth Prime Receiver",
            "Dual Zoren Prime Blueprint",
            "Epitaph Prime Barrel",
            "Equinox Prime Neuroptics Blueprint",
            "Equinox Prime Blueprint",
            "Euphona Prime Barrel",
            "Frost Prime Systems Blueprint",
            "Fulmin Prime Blueprint",
            "Grendel Prime Chassis Blueprint",
            "Guandao Prime Blueprint",
            "Harrow Prime Blueprint",
            "Hydroid Prime Blueprint",
            "Karyst Prime Handle",
            "Kavasa Prime Kubrow Collar Blueprint",
            "Nekros Prime Blueprint",
            "Nekros Prime Neuroptics Blueprint",
            "Lex Prime Blueprint",
            "Corinth Prime Barrel",
            "Grendel Prime Systems",
            "Karyst Prime Blade",
        ]
        .into_iter()
        .map(|name| RewardCatalogEntry {
            name: name.to_owned(),
            ducats: 45,
        })
        .collect();

        struct LiveFrame(image::DynamicImage);
        impl KioskFrameSource for LiveFrame {
            fn strip_profile(&mut self) -> Result<Vec<f32>, &'static str> {
                let (x, y, w, h) = kiosk_geometry::grid_strip(self.0.width(), self.0.height());
                Ok(kiosk_scroll::row_profiles(&self.0, x, y, w, h))
            }

            fn read_kiosk(
                &mut self,
                candidates: &[RewardCatalogEntry],
                dy: i32,
            ) -> Result<KioskRead, &'static str> {
                Ok(KioskRead {
                    cells: crate::kiosk_ocr::read_grid(&self.0, candidates, dy),
                    basket: vec![],
                })
            }
        }

        // The naked fold on this frame: still locked at its dragged midpoint. (The day's
        // publish showed exactly the lookalikes this read produces.)
        let strip = {
            let mut probe = LiveFrame(frame.clone());
            probe.strip_profile().expect("strip")
        };
        let anchors = kiosk_geometry::label_anchors(strip.len());
        let located = kiosk_scroll::label_offset(
            &strip,
            anchors.strip_top,
            anchors.first_top,
            anchors.pitch,
            anchors.band,
        )
        .expect("the polluted fold still answers a phase")
        .dy;
        let naked = crate::kiosk_ocr::read_grid(&frame, &candidates, located);
        assert!(
            naked
                .iter()
                .all(|cell| cell.score < crate::monitor::KIOSK_CONFIDENT_SCORE),
            "the fold's own answer must be visibly weak on this frame, else the test pins nothing: {naked:?}"
        );

        let (_, _, published) = run_poller_with(LiveFrame(frame), candidates);

        let published = published.lock().expect("published");
        let view = published.first().expect("polluted frame must publish");
        eprintln!(
            "published dy={} located={located} cells={} {:?}",
            view.scroll_dy,
            view.cells.len(),
            view.cells
                .iter()
                .map(|cell| (cell.name.as_str(), cell.col, cell.row))
                .collect::<Vec<_>>()
        );
        assert!(
            view.cells.len() >= 10,
            "the corrected phase must read the visible page, got {} cells",
            view.cells.len()
        );
        assert!(
            view.cells
                .iter()
                .any(|cell| cell.name == "Karyst Prime Handle")
                && view
                    .cells
                    .iter()
                    .any(|cell| cell.name == "Frost Prime Systems Blueprint"),
            "the published page must carry the true labels, not lookalikes: {:?}",
            view.cells
        );
        assert!(
            view.cells.iter().all(
                |cell| cell.name != "Corinth Prime Barrel" && cell.name != "Karyst Prime Blade"
            ),
            "a lookalike read must never survive recovery: {:?}",
            view.cells
        );
    }

    /// Motion streams frame-to-frame deltas -- not offsets against some anchor -- and the
    /// settled read publishes the grid's self-located offset, so the chips land on the rows
    /// wherever the scroll stopped. (2026-08-23: a stop at -142 read the gaps and died.)
    #[test]
    fn motion_streams_deltas_and_the_settle_publishes_the_located_offset() {
        let source = ScriptedKiosk::new(vec![Ok(KioskRead {
            cells: vec![scripted_cell("Scrolled row")],
            basket: vec![],
        })])
        // Pops come off the back: the first look is the unscrolled strip, then the scroll.
        .with_profiles(vec![
            Some(label_strip_at(-30)),
            Some(label_strip_at(-30)),
            Some(label_strip_at(-30)),
            Some(label_strip_at(-30)),
            Some(label_strip()),
        ]);
        let (_, gone, published) = run_poller(source);
        let published = published.lock().expect("published");
        assert_eq!(published.len(), 1);
        // The view carries the label phase. These synthetic bands are flat blocks, so the
        // locator's containment plateau is at its widest: any offset that still lands row
        // 0's crop over the anchor band's whole text (within 8 rows above its top) is right.
        let drift = (published[0].scroll_dy - -30).rem_euclid(222);
        assert!(
            drift == 0 || drift + 8 >= 222,
            "the view carries the label-located phase: {}",
            published[0].scroll_dy
        );
        // Whether the script's exhaustion later closed the session is beside the point.
        let _ = gone.load(Ordering::Acquire);
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn portal_close_is_requested_once_when_the_game_exits() {
        assert!(should_close_portal(Some(42), None));
        assert!(!should_close_portal(None, None));
        assert!(!should_close_portal(None, Some(42)));
        assert!(!should_close_portal(Some(42), Some(42)));
        assert!(!should_close_portal(Some(42), Some(43)));
    }

    #[test]
    fn a_direct_process_replacement_retires_the_previous_kiosk_session() {
        assert!(process_was_replaced(Some(42), Some(43)));
        assert!(!process_was_replaced(None, Some(43)));
        assert!(!process_was_replaced(Some(42), None));
        assert!(!process_was_replaced(Some(42), Some(42)));
    }

    #[test]
    fn monitor_path_changed_fires_on_the_first_observation() {
        assert!(monitor_path_changed(None, 42, None));
    }

    #[test]
    fn monitor_path_changed_is_silent_while_the_same_path_is_tracked() {
        let path = Path::new("/prefix/drive_c/EE.log");
        let tracked = Some((42, Some(path.to_path_buf())));
        assert!(!monitor_path_changed(tracked.as_ref(), 42, Some(path)));
    }

    #[test]
    fn monitor_path_changed_silent_when_the_log_disappears() {
        let path = Some((42u32, Some(PathBuf::from("/p/EE.log"))));
        assert!(!monitor_path_changed(path.as_ref(), 42, None));
    }

    #[test]
    fn monitor_path_changed_fires_when_the_pid_changes() {
        let tracked = Some((42u32, Some(PathBuf::from("/p/EE.log"))));
        assert!(monitor_path_changed(
            tracked.as_ref(),
            43,
            Some(Path::new("/p/EE.log"))
        ));
    }

    #[test]
    fn monitor_path_changed_fires_when_the_path_moves() {
        let tracked = Some((42u32, Some(PathBuf::from("/p/EE.log"))));
        assert!(monitor_path_changed(
            tracked.as_ref(),
            42,
            Some(Path::new("/q/EE.log"))
        ));
    }
    #[test]
    fn resetting_record_scans_waits_for_memory_workers_to_retire() {
        let scans = Arc::new(RecordScans::default());
        let (started_tx, started_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            started_tx.send(()).expect("report worker start");
            release_rx.recv().expect("release memory worker");
        });
        scans.track_worker(worker);
        started_rx.recv().expect("memory worker starts");

        let retiring = Arc::clone(&scans);
        let (retired_tx, retired_rx) = std::sync::mpsc::channel();
        let resetter = std::thread::spawn(move || {
            retiring.reset();
            retired_tx.send(()).expect("report retirement");
        });
        assert!(
            retired_rx.recv_timeout(Duration::from_millis(20)).is_err(),
            "reset returned while a process-memory worker was still active"
        );

        release_tx.send(()).expect("release worker");
        retired_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("reset returns after memory worker retires");
        resetter.join().expect("resetter exits");
    }
    #[test]

    fn resetting_record_scans_invalidates_workers_and_clears_both_indexes() {
        let scans = RecordScans::default();
        scans.generation.store(41, Ordering::Release);
        scans
            .records
            .lock()
            .expect("records")
            .insert("squadmate".to_owned(), "Forma Blueprint".to_owned());
        scans
            .active
            .lock()
            .expect("active scans")
            .insert("squadmate".to_owned());

        scans.reset();

        assert_eq!(scans.generation.load(Ordering::Acquire), 42);
        assert!(scans.records.lock().expect("records").is_empty());
        assert!(scans.active.lock().expect("active scans").is_empty());
    }

    /// Delayed market prices are gated by the monitor generation alone: a retired generation
    /// must not mutate reward candidates, market health, or emit updates.
    #[test]
    fn retired_generation_rejects_detached_price_worker_effects() {
        let directory = tempfile::tempdir().expect("temporary runtime");
        let shared = crate::tests::test_runtime(directory.path());
        let current = Arc::new(AtomicU64::new(0));
        let generation = MonitorGeneration::new(16, current);
        let (fetch_started_tx, fetch_started_rx) = std::sync::mpsc::channel();
        let (release_fetch_tx, release_fetch_rx) = std::sync::mpsc::channel();
        let emitted = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let worker_emitted = Arc::clone(&emitted);

        let worker = spawn_market_price_worker(
            vec![RewardObservation::certain("Forma Blueprint")],
            Arc::clone(&shared),
            vec![RewardCatalogEntry {
                name: "Forma Blueprint".to_owned(),
                ducats: 25,
            }],
            123,
            generation.clone(),
            move |_| {
                fetch_started_tx.send(()).expect("report fetch completion");
                release_fetch_rx.recv().expect("release price worker");
                (
                    BTreeMap::from([("Forma Blueprint".to_owned(), 12)]),
                    Some("delayed market failure"),
                )
            },
            move || {
                worker_emitted.fetch_add(1, Ordering::AcqRel);
            },
        );
        fetch_started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("price worker reaches delayed publication");

        generation.request_stop();
        release_fetch_tx.send(()).expect("release price worker");
        worker.join().expect("price worker exits");

        let runtime = shared.lock().expect("runtime");
        let view = runtime.core.current_view().expect("view builds");
        assert!(
            view.reward().cards().is_empty(),
            "a retired price worker mutated the reward candidates"
        );
        assert_eq!(
            view.health().market().message(),
            "warframe.market pricing idle; nothing to price yet",
            "a retired price worker mutated market health"
        );
        assert_eq!(
            emitted.load(Ordering::Acquire),
            0,
            "a retired price worker emitted reward-updated"
        );
    }
    #[test]
    fn retiring_generation_waits_for_in_flight_publication() {
        let current = Arc::new(AtomicU64::new(0));
        let generation = MonitorGeneration::new(16, current);
        let worker_generation = generation.clone();
        let (entered_tx, entered_rx) = std::sync::mpsc::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            worker_generation.publish(|| {
                entered_tx.send(()).expect("report publication start");
                release_rx.recv().expect("release publication");
            })
        });
        entered_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("publication starts");

        let retiring_generation = generation.clone();
        let (retired_tx, retired_rx) = std::sync::mpsc::channel();
        let retire = std::thread::spawn(move || {
            retiring_generation.request_stop();
            retired_tx.send(()).expect("report retirement");
        });
        assert!(
            retired_rx.recv_timeout(Duration::from_millis(20)).is_err(),
            "retirement returned while a reward publication was still mutating runtime state"
        );

        release_tx.send(()).expect("release publication");
        retired_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("retirement completes after publication");
        assert!(worker.join().expect("publication worker exits"));
        retire.join().expect("retirement worker exits");
    }

    #[test]
    fn retired_generation_rejects_delayed_reward_publication() {
        let current = Arc::new(AtomicU64::new(0));
        let generation = MonitorGeneration::new(17, Arc::clone(&current));
        let published = std::cell::Cell::new(false);

        assert!(generation.publish(|| published.set(true)));
        assert!(published.replace(false));

        current.fetch_add(1, Ordering::AcqRel);
        generation.request_stop();
        assert!(!generation.publish(|| published.set(true)));
        assert!(
            !published.get(),
            "a retired monitor must not publish a delayed price fetch"
        );
    }
}
