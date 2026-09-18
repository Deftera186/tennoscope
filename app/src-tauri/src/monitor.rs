use super::{
    AccessPolicy, SharedRuntime, apply_outcome, kiosk_geometry, kiosk_log,
    kiosk_ocr::{self, BasketRow, GridCell},
    kiosk_scroll,
    kiosk_view::{self, KioskState, KioskView},
    overlay_window, refresh_blocking, reward_capture,
    reward_log::{RewardLogEvent, RewardLogMachine},
    reward_observer::{RewardObservation, RewardObserverState},
    reward_ocr::{self, ScreenRewardSource},
    reward_source::{
        LiveMemoryRewardState, RewardChoiceSet, RewardChoiceSource, RewardSourceCoordinator,
        RewardSourceDiagnostic, RewardSourceResult, VisualRewardSource,
    },
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

/// How long to keep re-reading the reward screen before giving up. The cards appear a few
/// milliseconds after the log announces them and the screen lives for fifteen seconds, so this is
/// generous enough to cover a slow paint while still leaving the overlay useful.
const VISUAL_READ_DEADLINE: Duration = Duration::from_secs(8);

/// Gap between screen polls while a fissure mission is running. A poll costs about 160ms, almost
/// all of it process startup rather than OCR, so the interval is the only real lever on cost. Two
/// seconds keeps it near 8% of one core while still giving roughly seven attempts at a screen that
/// lives for fifteen.
const POLLER_INTERVAL: Duration = Duration::from_secs(2);
/// Once the cards are up the screen only lives fifteen seconds, so the question changes from "is
/// it here yet" to "has it gone", and that wants answering quickly.
const POLLER_WATCH_INTERVAL: Duration = Duration::from_millis(400);
/// Consecutive failed reads before the screen counts as closed. Cards read blank often enough
/// mid-screen that one miss is not evidence.
const POLLER_GONE_STREAK: u32 = 2;
/// Consecutive routine misses before one is worth a warning.
///
/// Before cards are found the poller runs every two seconds, so fifteen uninterrupted routine
/// misses represent roughly thirty seconds without a usable reward read.
const ROUTINE_MISS_WARNING_STREAK: u32 = 15;
/// Upper bound on how long a single fissure mission is worth watching for.
const POLLER_LIFETIME: Duration = Duration::from_secs(45 * 60);
/// The kiosk poller's steady cadence: the kiosk stays up while the player browses, so there is
/// no "found it, watch faster" split like the reward screen's -- one rate fast enough to feel
/// live against basket edits and slow enough to keep OCR off the CPU.
const KIOSK_POLL_INTERVAL: Duration = Duration::from_millis(400);
/// The kiosk poller's cadence while the grid is drifting: a grim capture costs ~25ms, so a 60ms
/// tick streams scroll offsets in near real time and still leaves the CPU alone.
const KIOSK_MOTION_INTERVAL: Duration = Duration::from_millis(60);

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

    /// Feed incremental EE.log bytes into the kiosk lifecycle. `spawn_poller` receives the
    /// session identity and shared flags: `reanchor` requests a read and `gone` permanently stops
    /// the worker. State is retired on the close event itself, before a later open in the same
    /// byte batch can re-arm.
    pub fn observe(
        &mut self,
        bytes: &[u8],
        kiosk_view: &KioskState,
        show: &dyn Fn(),
        spawn_poller: &SpawnKioskPoller<'_>,
    ) -> bool {
        let mut state_retired = false;
        for event in self.machine.observe_bytes(bytes) {
            log::debug!("[DEBUG-kiosk] ee event {event:?}");
            match event {
                kiosk_log::KioskLogEvent::KioskOpened => {
                    if self.poller_active {
                        // The machine already de-duplicates, but both open markers can land in
                        // one batch and a second poller would race the first for the same flags.
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
                    // One event, two readers that must both get it: the poller thread stops
                    // looking, and the monitor takes the window down on its next tick. They
                    // shared a single consuming flag once, and the monitor always won it --
                    // set here, swapped back by `take_close` a few lines later in the same
                    // tick, while the thread was still asleep or parked inside tesseract. It
                    // never saw the stop and ran out its 45-minute lifetime instead, so every
                    // visit leaked a live poller that went on capturing, publishing views and
                    // streaming scroll deltas across later sessions (nineteen in one evening,
                    // two at once, double-counting the scroll the overlay accumulates).
                    self.gone.store(true, Ordering::Release);
                    if let Some(poller) = self.poller.take() {
                        self.retired_pollers.push(poller);
                    }
                    if let Some(session) = self.active_session.take() {
                        kiosk_view.end_session(session);
                        state_retired = true;
                    }
                    // The session is over *here*, not when the monitor gets round to the
                    // window: a close and the next open arrive in one batch whenever the
                    // player reopens inside a tick, and leaving this set until `take_close`
                    // ran made the open look like a duplicate marker and dropped it -- a kiosk
                    // with no overlay for the whole visit.
                    self.poller_active = false;
                    self.close_pending = true;
                }
            }
        }
        state_retired
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
        self.poller_active = true;
        self.overlay_up = true;
        // A teardown armed earlier in this same batch belongs to the visit that just ended;
        // the player is back on the screen, so the overlay stays up instead of blinking.
        self.close_pending = false;
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

    /// Did the session end? Consumed once; the teardown is `close_overlay`'s. The verdict comes
    /// from the log (`KioskClosed`) -- the poller only ever stops looking, it does not judge.
    pub fn take_close(&mut self, kiosk_view: &KioskState, hide: &dyn Fn()) -> bool {
        std::mem::take(&mut self.close_pending) && {
            self.close_overlay(kiosk_view, hide);
            true
        }
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

/// The shared state of the reward-screen poller.
///
/// The pool, delivered read, worker flag and screen-gone signal describe one watch. Keeping them
/// together prevents a caller from arming with one pool while draining another watch's signals.
#[derive(Default)]
pub struct ScreenWatch {
    pool: SharedRelicPool,
    reads: Arc<Mutex<Option<Vec<String>>>>,
    polling: Arc<std::sync::atomic::AtomicBool>,
    gone: Arc<std::sync::atomic::AtomicBool>,
    poller: Mutex<Option<std::thread::JoinHandle<()>>>,
}

impl ScreenWatch {
    pub fn adopt(&self, relics: &[String], entries: Vec<RewardCatalogEntry>) {
        if let Ok(mut pool) = self.pool.lock() {
            pool.adopt(relics, entries);
        }
    }

    pub fn take_read(&self) -> Option<Vec<String>> {
        self.reads.lock().ok().and_then(|mut slot| slot.take())
    }

    pub fn take_gone(&self) -> bool {
        self.gone.swap(false, Ordering::AcqRel)
    }

    pub fn stop(&self) {
        self.polling.store(false, Ordering::Release);
        if let Ok(mut poller) = self.poller.lock()
            && let Some(poller) = poller.take()
        {
            let _ = poller.join();
        }
    }

    pub fn running(&self) -> bool {
        self.polling.load(Ordering::Acquire)
    }

    fn arm(&self) {
        if let Some(poller) =
            spawn_reward_screen_poller_with(self, PollerTiming::live(), ScreenRewardSource::new)
            && let Ok(mut slot) = self.poller.lock()
        {
            *slot = Some(poller);
        }
    }

    fn gone_signal(&self) -> &std::sync::atomic::AtomicBool {
        &self.gone
    }

    fn trace_published(&self, names: &[String]) {
        if let Ok(pool) = self.pool.lock() {
            pool.trace_published(names);
        }
    }
}

/// What the monitor knows about the squad whose reward screen is approaching.
#[derive(Default)]
struct SquadProgress {
    resolved: bool,
    pending: Option<PendingRewardSquad>,
}

impl SquadProgress {
    fn remember(&mut self, squad: PendingRewardSquad) {
        self.pending = Some(squad);
    }

    fn squad(&self, expected: usize) -> Option<&PendingRewardSquad> {
        self.pending
            .as_ref()
            .filter(|squad| squad.screen_order.len() == expected)
    }

    fn resolved(&self) -> bool {
        self.resolved
    }

    fn resolve(&mut self) {
        self.resolved = true;
    }

    fn reset(&mut self) {
        self.resolved = false;
        self.pending = None;
    }
}

/// The catalogs used to turn a squad's relic paths into the reward names visible on its screen.
struct RewardReference {
    catalog: Option<CatalogIndex>,
    relics: Option<RelicRewardIndex>,
    rewards: Vec<RewardCatalogEntry>,
}

impl RewardReference {
    fn candidates_for(&self, relic_paths: &[String]) -> Vec<warframe_acquisition::RewardNeedle> {
        self.catalog
            .as_ref()
            .zip(self.relics.as_ref())
            .map(|(catalog, relics)| relics.candidates_for_projection_paths(relic_paths, catalog))
            .unwrap_or_default()
    }

    fn pool_entries(
        &self,
        candidates: &[warframe_acquisition::RewardNeedle],
    ) -> Vec<RewardCatalogEntry> {
        relic_pool_entries(candidates, &self.rewards)
    }
}

/// What one observed log event may do: the effective access policy, the monitor generation
/// it belongs to, and the second it was observed in. Bundled so handler signatures stay small.
#[derive(Clone, Copy)]
struct EventScope<'a> {
    policy: AccessPolicy,
    generation: &'a MonitorGeneration,
    now: u64,
}

/// Everything whose lifetime is one monitored Warframe process's reward stream.
///
/// The monitor supplies events and application side effects; this session owns the coupled
/// observation, responder-scan and screen-watch state that must turn over together.
struct RewardSession {
    reference: RewardReference,
    memory: LiveMemoryRewardState,
    coordinator: RewardSourceCoordinator,
    observer: RewardObserverState,
    progress: SquadProgress,
    scans: RecordScans,
    watch: ScreenWatch,
    screen: Option<ScreenRewardSource>,
    price_cache: MarketPriceCache,
}

impl RewardSession {
    fn new(
        catalog: Option<CatalogIndex>,
        relics: Option<RelicRewardIndex>,
        rewards: Vec<RewardCatalogEntry>,
        price_cache: MarketPriceCache,
    ) -> Self {
        Self {
            reference: RewardReference {
                catalog,
                relics,
                rewards,
            },
            memory: LiveMemoryRewardState::new(RewardMemoryScanner::new(
                256 * 1024,
                768 * 1024 * 1024,
                Duration::from_millis(1_500),
            )),
            coordinator: RewardSourceCoordinator::new(cfg!(debug_assertions)),
            observer: RewardObserverState::new(1, 1),
            progress: SquadProgress::default(),
            scans: RecordScans::default(),
            watch: ScreenWatch::default(),
            screen: None,
            price_cache,
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
            RewardLogEvent::ResponsesComplete {
                screen_order,
                local_reward_path,
                ..
            } => {
                let squad = PendingRewardSquad {
                    screen_order,
                    local_reward_path,
                };
                self.progress.remember(squad.clone());
                // The screen read needs a window, not a process handle, but a dead game has neither:
                // requiring the process keeps a vanished game from burning the retry deadline.
                if process.is_some()
                    && self
                        .try_publish(&squad, shared, app, scope.now, scope.generation)
                        .is_ok()
                {
                    self.progress.resolve();
                }
            }
            RewardLogEvent::BaselineRequested { relic_paths } => {
                self.progress.reset();
                self.scans.reset();
                let candidates = self.reference.candidates_for(&relic_paths);
                let Some(_process) = process else {
                    self.memory.clear();
                    return;
                };
                if scope.policy.read_process_memory {
                    self.memory.prepare_candidates(&candidates);
                } else {
                    self.memory.clear();
                }
                // Publish the pool before arming, and on every baseline rather than only the first.
                // A running poller reads this cell each poll, so later relic loads still reach it.
                let entries = self.reference.pool_entries(&candidates);
                self.watch.adopt(&relic_paths, entries.clone());
                spawn_market_price_warm(&entries, &self.price_cache);
                self.watch.arm();
            }
            RewardLogEvent::ChoicesReady {
                expected_choices, ..
            } => {
                if self.progress.resolved() || process.is_none() {
                    return;
                }
                let Some(squad) = self.progress.squad(expected_choices).cloned() else {
                    if let Ok(mut runtime) = shared.lock() {
                        let _ = runtime
                            .core
                            .record_capture_degraded("Structured reward records were incomplete");
                    }
                    return;
                };
                match self.try_publish(&squad, shared, app, scope.now, scope.generation) {
                    Ok(()) => self.progress.resolve(),
                    // Name the subsystem that actually failed. Reporting structured records here
                    // sends an investigation toward EE.log parsing even when capture is the fault.
                    Err(reason) => {
                        if let Ok(mut runtime) = shared.lock() {
                            let _ = runtime.core.record_capture_degraded(format!(
                                "Screen capture failed: {reason}"
                            ));
                        }
                    }
                }
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
        if self.progress.resolved() || !policy.read_process_memory {
            return;
        }
        let Some(process) = process else {
            return;
        };
        self.scans.scan(
            identity,
            process,
            self.memory.candidates(),
            generation.clone(),
        );
    }

    fn try_publish(
        &mut self,
        squad: &PendingRewardSquad,
        shared: &SharedRuntime,
        app: &AppHandle,
        now: u64,
        generation: &MonitorGeneration,
    ) -> Result<(), &'static str> {
        let result = read_squad_cards(
            squad,
            &self.memory,
            &self.coordinator,
            self.screen.get_or_insert_with(ScreenRewardSource::new),
            &self.reference.rewards,
            self.watch.gone_signal(),
        )?;
        self.publish(result, shared, app, now, generation);
        Ok(())
    }

    fn publish(
        &mut self,
        result: RewardSourceResult,
        shared: &SharedRuntime,
        app: &AppHandle,
        now: u64,
        generation: &MonitorGeneration,
    ) {
        let observations = result
            .choices
            .names
            .into_iter()
            .map(RewardObservation::certain)
            .collect::<Vec<_>>();
        let transition = self.observer.observe(observations);
        let mut overlay_notice = None;
        if transition.publish {
            apply_reward_observations(
                shared,
                &self.reference.rewards,
                &transition.choices,
                &BTreeMap::new(),
            );
            overlay_notice = overlay_window::show_reward_overlay(app, transition.choices.len());
            let _ = app.emit_to("reward-overlay", "reward-updated", ());
            spawn_market_price_fetch(
                &transition.choices,
                shared,
                app,
                &self.reference.rewards,
                &self.price_cache,
                now,
                generation,
            );
        }
        if let Ok(mut runtime) = shared.lock() {
            let source = match result.choices.source {
                RewardChoiceSource::Memory => "memory",
                RewardChoiceSource::Ocr => "ocr",
            };
            let _ = runtime.core.record_capture_source_ready(
                source,
                result.choices.elapsed.as_millis(),
                now.to_string(),
            );
            // Read the cards but could not find the window to draw over: on Windows that is
            // exclusive fullscreen, and the player is the only one who can fix it.
            if let Some(notice) = overlay_notice {
                let _ = runtime.core.record_capture_degraded(notice);
            }
            if result.diagnostic == RewardSourceDiagnostic::Disagreement {
                let _ = runtime
                    .core
                    .record_capture_degraded("memory and OCR reward recognition disagreed");
            }
        }
    }

    fn drain_screen_watch(
        &mut self,
        shared: &SharedRuntime,
        app: &AppHandle,
        now: u64,
        generation: &MonitorGeneration,
    ) {
        if let Some(names) = self.watch.take_read()
            && !self.progress.resolved()
        {
            // The poller's closed-set match is the only evidence on this path, so retain the exact
            // candidate pool alongside the published names in the capture trace.
            self.watch.trace_published(&names);
            self.publish(
                RewardSourceResult {
                    choices: RewardChoiceSet {
                        names,
                        source: RewardChoiceSource::Ocr,
                        elapsed: Duration::ZERO,
                    },
                    diagnostic: RewardSourceDiagnostic::MemoryFallback,
                },
                shared,
                app,
                now,
                generation,
            );
            self.progress.resolve();
        }
        // The capture signal beats EE.log's delayed shutdown line and prevents a stale overlay.
        if self.watch.take_gone() && self.observer.miss().hide {
            overlay_window::hide_reward_overlay(app);
        }
    }

    fn game_gone(&mut self, app: &AppHandle) {
        self.memory.clear();
        self.scans.reset();
        if self.observer.miss().hide {
            overlay_window::hide_reward_overlay(app);
        }
        // Release the game-session screen cast; each later read locates Warframe again.
        if self.screen.take().is_some() {
            log::debug!("[DEBUG-capture] game gone; released the monitor thread's capture");
        }
    }

    fn close(&mut self, shared: &SharedRuntime, app: &AppHandle) {
        self.watch.stop();
        self.progress.reset();
        self.scans.reset();
        self.memory.clear();
        self.observer.miss();
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
                    cache
                        .get(name)
                        .or_else(|| table.as_ref().and_then(|table| table.price_for(name)))
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
                        now,
                    },
                );
            }
        }
        // Same bytes, second machine: the kiosk's lifecycle is independent of the reward screen's
        // (the two never occur at once in practice, but neither knows about the other).
        if let Some(kiosk_view_cell) = app.try_state::<KioskState>() {
            let state_retired = kiosk_session.observe(
                &log_bytes,
                kiosk_view_cell.inner(),
                &kiosk_show,
                &kiosk_spawn,
            );
            // A same-batch reopen deliberately leaves the window up. Publish the resulting
            // lifecycle identity before starting an IPC read, so the webview retires the old
            // visit synchronously and rejects its already-queued scroll events.
            if state_retired {
                emit_kiosk_update(&app, kiosk_view_cell.active_session());
            }
            // The log's close line and the game process dying are the only closes there are;
            // both land here.
            kiosk_session.take_close(kiosk_view_cell.inner(), &kiosk_hide);
        }
        reward_session.drain_screen_watch(&shared, &app, now, &generation);
        if process.is_none() {
            reward_session.game_gone(&app);
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

/// The squad roster in screen order, plus the one reward EE.log states outright. `local_identity`
/// used to ride along for the memory scan's per-player attribution; the screen read needs only the
/// local player's reward name, as a check that the four cards it read include the one the log
/// already confirmed.
#[derive(Clone, Debug)]
struct PendingRewardSquad {
    screen_order: Vec<String>,
    local_reward_path: Option<String>,
}

/// Read the squad's cards off the screen, against the pool their own relics resolve to.
///
/// Kept as a narrow seam so tests can prove the caller-owned visual source and squad-specific
/// candidate pool are used without constructing an `AppHandle`.
fn read_squad_cards(
    squad: &PendingRewardSquad,
    memory_state: &LiveMemoryRewardState,
    coordinator: &RewardSourceCoordinator,
    visual: &mut dyn VisualRewardSource,
    reward_catalog: &[RewardCatalogEntry],
    visual_screen_gone: &std::sync::atomic::AtomicBool,
) -> Result<RewardSourceResult, &'static str> {
    let local_choice = squad.local_reward_path.as_deref().and_then(|path| {
        memory_state
            .candidates()
            .iter()
            .find(|needle| {
                needle.internal_paths().iter().any(|candidate| {
                    reward_path_matches(path, std::str::from_utf8(candidate).unwrap_or(""))
                })
            })
            .map(|needle| needle.choice_name().to_owned())
    });
    // Matching a card against the squad's own relic pool rather than the whole catalog is what
    // keeps a garbled read on the right item; a few dozen names, not a few thousand.
    let pool = relic_pool_entries(memory_state.candidates(), reward_catalog);
    coordinator.visual_choices(
        visual,
        &pool,
        squad.screen_order.len(),
        local_choice.as_deref(),
        VISUAL_READ_DEADLINE,
        visual_screen_gone,
    )
}

/// The names the poller matches a card against, and the relics they came from.
///
/// The relics ride along because they are what says *which fissure* a pool describes. Length
/// cannot: a pool is not better for being bigger, it is right or wrong depending on whose relics
/// are on screen.
#[derive(Clone, Debug, Default)]
pub struct RelicPool {
    relics: Vec<String>,
    entries: Vec<RewardCatalogEntry>,
}

impl RelicPool {
    /// Take on the pool this fissure's relics resolve to, replacing whatever was here.
    ///
    /// This used to keep whichever pool was longer, which is safe within a fissure and wrong
    /// between them. `loaded_relics` is append-only until the reward screen shuts down and the
    /// catalog is resolved once before the monitor loop, so a later baseline in the same fissure
    /// can only ever resolve a superset -- the length test never did anything there. Across
    /// fissures it did the only thing it could: kept the older, bigger pool.
    ///
    /// 2026-08-20 is what that cost. A 38-name pool from a fissure two hours earlier outlived the
    /// application restart between them and displaced a 16-name one, and the closed-set match has
    /// no way to say "not in the pool" -- it returns the nearest name it was given. All four cards
    /// were published wrong, above the match floor, without a single failed read to show for it.
    pub fn adopt(&mut self, relics: &[String], entries: Vec<RewardCatalogEntry>) {
        self.relics = relics.to_vec();
        self.entries = entries;
    }

    pub fn entries(&self) -> &[RewardCatalogEntry] {
        &self.entries
    }

    /// Record what a published read was matched against.
    ///
    /// At Info, not Debug, and that is the whole reason it exists. The pool already announced
    /// itself at `[DEBUG-poller] arm pool=38`, but the stable build's file target keeps `<= Info`
    /// -- so on 2026-08-20 a player sent a report in which all four cards were wrong, all four
    /// were above the match floor, nothing had failed, and there was no line anywhere saying the
    /// pool belonged to a fissure two hours earlier. It had to be reconstructed afterwards by
    /// reading the squad's relics out of the cached catalog by hand.
    ///
    /// The relic paths are trimmed to their names because the prefix is the same on every one and
    /// four of them do not fit a log line otherwise.
    pub fn trace_published(&self, names: &[String]) {
        let relics = self
            .relics
            .iter()
            .map(|path| path.rsplit('/').next().unwrap_or(path))
            .collect::<Vec<_>>();
        log::info!(
            "reward: published cards={names:?} pool={} relics={relics:?}",
            self.entries.len(),
        );
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// The relic pool the poller matches against, shared because it is still growing when the poller
/// starts.
///
/// Each squad member's relic is logged as it loads, and the baseline fires on the second one --
/// long before the other two arrive. The pool was passed to the poller by value at that moment, so
/// the later relics were only ever seen by the arming call that the "already running" guard then
/// declined. The poller spent the rest of the fissure matching a screen of four rewards against a
/// pool that only knew two relics' worth, and one unmatched card fails the whole read, so the
/// overlay never appeared. Observed live on 2026-07-27: armed at 11 names, the 17-name pool
/// declined, and `Banshee Prime Neuroptics Blueprint` -- on screen, in the newer pool, not in the
/// older one -- failed every attempt.
pub type SharedRelicPool = Arc<Mutex<RelicPool>>;

/// Whether this poll failure deserves a warning, given how many times its reason has repeated.
///
/// Blank cards and pool misses are routine away from the reward screen, so only a sustained streak
/// warns. Other failures indicate broken capture and warn immediately. Each class warns only once
/// per uninterrupted streak.
fn poll_failure_is_worth_warning(reason: &str, consecutive: u32) -> bool {
    let routine = matches!(
        reason,
        "a reward card read as blank" | "reward card text did not match the relic pool"
    );
    if routine {
        consecutive == ROUTINE_MISS_WARNING_STREAK
    } else {
        consecutive == 1
    }
}

/// How often the poller looks, before and after it has found the cards.
///
/// Two rates because the poller does two jobs. Before the cards it may wait minutes, so it looks
/// slowly. Once they are up the screen only lives fifteen seconds and the question becomes when it
/// disappears, which wants a fast answer -- a miss costs one crop, since the read stops at the
/// first card that will not match.
#[derive(Clone, Copy, Debug)]
pub struct PollerTiming {
    pub interval: Duration,
    pub watch_interval: Duration,
    pub lifetime: Duration,
}

impl PollerTiming {
    pub const fn live() -> Self {
        Self {
            interval: POLLER_INTERVAL,
            watch_interval: POLLER_WATCH_INTERVAL,
            lifetime: POLLER_LIFETIME,
        }
    }
}

/// The body of the poller, with the screen and the clock as parameters.
///
/// Four live runs produced no overlay and no way to tell arming from polling from reading, because
/// the only way to reach this loop was to play a fissure. Taking the source as an argument lets a
/// test drive it against a scripted screen in milliseconds, which is how the retry, the stop flag,
/// and the four-name guard below are actually checked rather than argued about.
///
/// Returns the join handle so a test can wait for the thread instead of sleeping, and `None` when
/// arming was declined.
pub fn spawn_reward_screen_poller_with<S, F>(
    watch: &ScreenWatch,
    timing: PollerTiming,
    make_source: F,
) -> Option<std::thread::JoinHandle<()>>
where
    F: FnOnce() -> S + Send + 'static,
    // The source is constructed inside the spawned thread, then born, read and dropped there.
    // Keeping the factory `Send` is sufficient; requiring `S: Send` would impose a constraint the
    // ownership model does not need.
    S: VisualRewardSource + 'static,
{
    // Claim the flag only once this call is definitely going to spawn. Taking it first and then
    // bailing on an empty pool leaves it set with no thread behind it, and since only a running
    // poller or the screen shutting down ever clears it, every later relic load in that fissure is
    // declined as a duplicate. The first relic pair is exactly when the pool can still be empty --
    // a vaulted relic resolves to no candidates -- so the poller was being poisoned before the
    // fissure that needed it had even started.
    let pool_size = watch.pool.lock().map(|pool| pool.len()).unwrap_or(0);
    if pool_size == 0 {
        log::debug!("[DEBUG-poller] arm declined: empty pool");
        return None;
    }
    let already_running = watch.polling.swap(true, Ordering::AcqRel);
    log::debug!("[DEBUG-poller] arm pool={pool_size} already_running={already_running}");
    if already_running {
        return None;
    }
    let pool = Arc::clone(&watch.pool);
    let visual_reads = Arc::clone(&watch.reads);
    let visual_polling = Arc::clone(&watch.polling);
    let visual_screen_gone = Arc::clone(&watch.gone);
    Some(std::thread::spawn(move || {
        let mut source = make_source();
        let deadline = Instant::now() + timing.lifetime;
        // Keep polling after the cards are found, to see the screen go away. The shutdown line in
        // EE.log arrives with the same flush delay as everything else, so hiding on it leaves the
        // overlay up for seconds after the screen it describes has gone.
        let mut found = false;
        let mut misses = 0_u32;
        let mut last_reason: Option<&'static str> = None;
        let mut repeated = 0_u32;
        while visual_polling.load(Ordering::Acquire) && Instant::now() < deadline {
            // Re-read the pool every poll rather than capturing it at arm time. Squadmates' relics
            // are still loading when this thread starts, and a card missing from the pool fails the
            // whole screen.
            let current = pool
                .lock()
                .map(|pool| pool.entries().to_vec())
                .unwrap_or_default();
            if current.is_empty() {
                std::thread::sleep(timing.interval);
                continue;
            }
            let outcome = VisualRewardSource::choices(&mut source, &current);
            if let Err(reason) = &outcome {
                if last_reason == Some(*reason) {
                    repeated = repeated.saturating_add(1);
                } else {
                    last_reason = Some(*reason);
                    repeated = 1;
                }
                if poll_failure_is_worth_warning(reason, repeated) {
                    log::warn!("[DEBUG-poller] poll failed: {reason}");
                } else {
                    log::debug!("[DEBUG-poller] poll failed: {reason} (x{repeated})");
                }
            } else {
                last_reason = None;
                repeated = 0;
            }
            match outcome {
                // However many cards the screen has -- the reader reports the layout it found, and
                // a squad of three is three cards, not a failed read of four. Requiring four here
                // is what threw away a good three-card read even after the crops were looking in
                // the right place. Two is the floor because one reward is not a choice.
                Ok(names) if names.len() >= 2 => {
                    if !found && let Ok(mut slot) = visual_reads.lock() {
                        *slot = Some(names);
                        found = true;
                    }
                    misses = 0;
                }
                // A card reads blank often enough mid-screen that one miss cannot mean the screen
                // closed; require a streak before taking the overlay down.
                _ if found => {
                    misses += 1;
                    if misses >= POLLER_GONE_STREAK {
                        log::debug!("[DEBUG-poller] reward screen gone");
                        visual_screen_gone.store(true, Ordering::Release);
                        break;
                    }
                }
                _ => {}
            }
            std::thread::sleep(if found {
                timing.watch_interval
            } else {
                timing.interval
            });
        }
        visual_polling.store(false, Ordering::Release);
    }))
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

/// How many consecutive still looks before the screen counts as settled and the read runs. One
/// still look can be a flick's mid-detent hitch; two is a scroll that has actually stopped.
const KIOSK_SETTLE_LOOKS: u32 = 2;

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
            let Some(frame_delta) = frame_delta else {
                // Blindness is not stillness. Fade stale chips and restart settling; otherwise
                // two torn/flat looks can launch OCR against a displacement we never measured.
                emit_scroll(None);
                static_looks = 0;
                std::thread::sleep(timing.motion_interval);
                continue;
            };
            if frame_delta.abs() > 1 {
                // Stream the frame's movement; the frontend accumulates the deltas. Deltas
                // need no anchor and no range, so the chips follow a scroll of any length.
                emit_scroll(Some(frame_delta));
                static_looks = 0;
                std::thread::sleep(timing.motion_interval);
                continue;
            }
            // Two readable, agreeing looks mean settled. The settled read locates itself -- the
            // grid's own label rows name the offset at any scroll position, so the crops land on
            // the text instead of the gaps.
            static_looks += 1;
            if static_looks < KIOSK_SETTLE_LOOKS {
                std::thread::sleep(timing.motion_interval);
                continue;
            }
            static_looks = 0;
            let located = reading.and_then(|strip| {
                let at = kiosk_geometry::label_anchors(strip.len());
                kiosk_scroll::label_offset(strip, at.strip_top, at.first_top, at.pitch, at.band)
            });
            let Some(dy) = located else {
                // No label band anywhere in the pane: an animation frame, a capture that came
                // back torn, or a grid the player has filtered down to nothing. None of those
                // is a closed kiosk, and none of them is worth publishing over a good view.
                log::debug!("[DEBUG-kiosk] no labels located: skipping this look");
                std::thread::sleep(timing.interval);
                continue;
            };
            match source.read_kiosk(&candidates, dy) {
                Ok(frame) => {
                    // A read costs the better part of a second of OCR. A close that landed
                    // while it ran means this frame describes a screen that is already gone,
                    // and publishing it would refill the state the teardown just cleared.
                    if gone.load(Ordering::Acquire) {
                        break;
                    }
                    let mut view = joiner(epoch, &frame);
                    view.scroll_dy = dy;
                    log::debug!(
                        "[DEBUG-kiosk] publish epoch={epoch} cells={} basket={} total={} dy={dy}",
                        view.cells.len(),
                        view.basket.len(),
                        view.total_plat
                    );
                    publish(view);
                }
                Err(reason) => log::warn!("[DEBUG-kiosk] read failed: {reason}"),
            }
            std::thread::sleep(timing.interval);
        }
    })
}

/// The relic pool as catalog entries, so the visual source can match against exactly the rewards
/// this squad's relics can produce.
fn relic_pool_entries(
    candidates: &[warframe_acquisition::RewardNeedle],
    reward_catalog: &[RewardCatalogEntry],
) -> Vec<RewardCatalogEntry> {
    candidates
        .iter()
        .map(|needle| RewardCatalogEntry {
            name: needle.choice_name().to_owned(),
            ducats: reward_catalog
                .iter()
                .find(|entry| {
                    warframe_acquisition::reward_name_matches(&entry.name, needle.choice_name())
                })
                .map_or(0, |entry| entry.ducats),
        })
        .collect()
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

pub fn reward_path_matches(log_path: &str, catalog_path: &str) -> bool {
    log_path == catalog_path
        || log_path
            .strip_prefix("/Lotus/StoreItems")
            .is_some_and(|suffix| catalog_path == format!("/Lotus{suffix}"))
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
    }

    impl ScriptedKiosk {
        fn new(looks: Vec<Result<KioskRead, &'static str>>) -> Self {
            Self {
                looks: StdMutex::new(looks),
                profiles: StdMutex::new(vec![None]),
                closes_mid_read: None,
            }
        }

        fn with_profiles(mut self, profiles: Vec<Option<Vec<f32>>>) -> Self {
            self.profiles = StdMutex::new(profiles);
            self
        }

        fn closing_mid_read(mut self, gone: &Arc<AtomicBool>) -> Self {
            self.closes_mid_read = Some(Arc::clone(gone));
            self
        }
    }

    impl KioskFrameSource for ScriptedKiosk {
        fn read_kiosk(
            &mut self,
            _candidates: &[RewardCatalogEntry],
            _dy: i32,
        ) -> Result<KioskRead, &'static str> {
            if let Some(gone) = &self.closes_mid_read {
                gone.store(true, Ordering::Release);
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

    fn run_poller_with(
        source: ScriptedKiosk,
    ) -> (
        Arc<AtomicBool>,
        Arc<AtomicBool>,
        Arc<StdMutex<Vec<KioskView>>>,
    ) {
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
            Arc::new(Vec::new()),
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

    fn run_poller(
        source: ScriptedKiosk,
    ) -> (
        Arc<AtomicBool>,
        Arc<AtomicBool>,
        Arc<StdMutex<Vec<KioskView>>>,
    ) {
        run_poller_with(source)
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
    fn a_discovery_error_is_not_a_confirmed_game_exit() {
        let failure: Result<Option<u32>, &str> = Err("procfs was temporarily unreadable");
        assert_eq!(confirmed_process_observation(&failure), None);
        assert_eq!(
            confirmed_process_observation::<u32, &str>(&Ok(None)),
            Some(None)
        );
    }

    /// Mutation caught: treating routine blank and pool misses like capture failures would warn on
    /// every ordinary gameplay poll again.
    #[test]
    fn a_single_routine_miss_is_not_a_warning() {
        assert!(!poll_failure_is_worth_warning(
            "a reward card read as blank",
            1
        ));
        assert!(!poll_failure_is_worth_warning(
            "reward card text did not match the relic pool",
            1
        ));
    }

    /// Mutation caught: using an off-by-one or `>=` threshold would either miss the one warning or
    /// repeat it after roughly 30 seconds of uninterrupted pre-detection routine misses.
    #[test]
    fn a_persistent_routine_miss_warns_once_at_the_threshold() {
        assert!(poll_failure_is_worth_warning(
            "a reward card read as blank",
            15
        ));
        assert!(!poll_failure_is_worth_warning(
            "a reward card read as blank",
            16
        ));
    }

    /// Mutation caught: applying routine-miss handling to a capture failure would delay its first
    /// warning, while accepting every occurrence would bury the report in repeats.
    #[test]
    fn a_missing_window_warns_immediately_but_does_not_repeat() {
        assert!(poll_failure_is_worth_warning("no Warframe window found", 1));
        assert!(!poll_failure_is_worth_warning(
            "no Warframe window found",
            2
        ));
        assert!(!poll_failure_is_worth_warning(
            "no Warframe window found",
            249
        ));
    }

    #[test]
    fn monitor_path_changed_fires_on_the_first_observation() {
        assert!(monitor_path_changed(None, 42, None));
    }

    /// A screen reader that records reads on the injected instance.
    ///
    /// Reads landing on this instance prove the event path reuses its caller-owned source instead
    /// of constructing an unrelated reader for each event.
    struct CountedScreen {
        cards: Vec<String>,
        /// The pool of the last read, so a test can check what the read was matched against.
        seen: Vec<String>,
        /// Reads that landed on *this* instance. The discriminating counter: a
        /// `read_squad_cards` that built a source of its own would leave this at zero however many
        /// times it was called, because the reads would land on its private instance instead.
        reads: u32,
    }

    impl CountedScreen {
        fn new(cards: &[&str]) -> Self {
            Self {
                cards: cards.iter().map(|name| (*name).to_owned()).collect(),
                seen: Vec::new(),
                reads: 0,
            }
        }
    }

    impl VisualRewardSource for CountedScreen {
        fn choices(
            &mut self,
            candidates: &[RewardCatalogEntry],
        ) -> Result<Vec<String>, &'static str> {
            self.reads += 1;
            self.seen = candidates
                .iter()
                .map(|entry| entry.name.clone())
                .collect::<Vec<_>>();
            Ok(self.cards.clone())
        }
    }

    fn squad_of(names: &[&str], local_reward_path: Option<&str>) -> PendingRewardSquad {
        PendingRewardSquad {
            screen_order: names.iter().map(|name| (*name).to_owned()).collect(),
            local_reward_path: local_reward_path.map(str::to_owned),
        }
    }

    fn memory_state_for(needles: Vec<warframe_acquisition::RewardNeedle>) -> LiveMemoryRewardState {
        let mut state = LiveMemoryRewardState::new(RewardMemoryScanner::new(
            4096,
            1024 * 1024,
            Duration::from_millis(1),
        ));
        state.prepare_candidates(&needles);
        state
    }

    /// The read must land on the source it was handed, not on one built inside.
    ///
    /// The assertion is on reads landing on this instance. Counting test-side constructions would
    /// not catch an implementation that quietly built a private source.
    #[test]
    fn the_screen_read_lands_on_the_source_it_was_given() {
        let mut screen = CountedScreen::new(&["A", "B"]);
        let state = memory_state_for(vec![
            warframe_acquisition::RewardNeedle::new("A", ["/Lotus/A"]).expect("needle"),
            warframe_acquisition::RewardNeedle::new("B", ["/Lotus/B"]).expect("needle"),
        ]);

        let result = read_squad_cards(
            &squad_of(&["one", "two"], None),
            &state,
            &RewardSourceCoordinator::new(false),
            &mut screen,
            &[],
            &std::sync::atomic::AtomicBool::new(false),
        )
        .expect("the injected screen's cards are published");

        assert_eq!(result.choices.names, vec!["A".to_owned(), "B".to_owned()]);
        assert_eq!(
            screen.reads, 1,
            "the read went somewhere other than the source it was given"
        );
    }

    /// A whole fissure run's worth of reward events must reuse its caller-owned source.
    ///
    /// `RewardSession::try_publish` is reached twice per reward screen -- from
    /// `ResponsesComplete` and again from `ChoicesReady`. Eight reward events must therefore
    /// produce eight reads on the same instance. A count of zero here means the event path ignored
    /// the source it was given and built an unrelated reader instead.
    #[test]
    fn a_run_of_reward_screens_reads_through_the_same_source_every_time() {
        let mut screen = CountedScreen::new(&["A", "B"]);
        let state = memory_state_for(vec![
            warframe_acquisition::RewardNeedle::new("A", ["/Lotus/A"]).expect("needle"),
            warframe_acquisition::RewardNeedle::new("B", ["/Lotus/B"]).expect("needle"),
        ]);
        let coordinator = RewardSourceCoordinator::new(false);
        let squad = squad_of(&["one", "two"], None);

        // Four fissures, both reward events each.
        for _ in 0..8 {
            assert!(
                read_squad_cards(
                    &squad,
                    &state,
                    &coordinator,
                    &mut screen,
                    &[],
                    &std::sync::atomic::AtomicBool::new(false),
                )
                .is_ok(),
                "every read should publish"
            );
        }

        assert_eq!(
            screen.reads, 8,
            "eight reward events did not all read through the one source they were given"
        );
    }

    /// The pool a card is matched against is the squad's own relics, not the whole catalog.
    ///
    /// Pinned because `read_squad_cards` builds that pool for `RewardSession::try_publish`.
    /// A read handed the full catalog is the 2026-08-20 failure mode: the closed-set match cannot
    /// say "not in the pool", it returns
    /// the nearest name it was given, so a too-wide pool publishes confident nonsense.
    #[test]
    fn the_read_is_matched_against_the_squads_own_relic_pool() {
        let mut screen = CountedScreen::new(&["A", "B"]);
        let state = memory_state_for(vec![
            warframe_acquisition::RewardNeedle::new("A", ["/Lotus/A"]).expect("needle"),
            warframe_acquisition::RewardNeedle::new("B", ["/Lotus/B"]).expect("needle"),
        ]);
        // The catalog knows a reward this squad's relics cannot drop.
        let catalog = ["A", "B", "Elsewhere Prime Blueprint"]
            .into_iter()
            .map(|name| RewardCatalogEntry {
                name: name.to_owned(),
                ducats: 0,
            })
            .collect::<Vec<_>>();

        read_squad_cards(
            &squad_of(&["one", "two"], None),
            &state,
            &RewardSourceCoordinator::new(false),
            &mut screen,
            &catalog,
            &std::sync::atomic::AtomicBool::new(false),
        )
        .expect("the cards are published");

        assert_eq!(
            screen.seen,
            vec!["A".to_owned(), "B".to_owned()],
            "the read was matched against something other than the squad's own relic pool"
        );
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

    #[test]
    fn screen_watch_drains_signals_once_and_stops_the_poller() {
        let watch = ScreenWatch::default();
        watch.polling.store(true, Ordering::Release);
        *watch.reads.lock().expect("visual reads") = Some(vec!["Forma Blueprint".to_owned()]);
        watch.gone.store(true, Ordering::Release);

        assert_eq!(watch.take_read(), Some(vec!["Forma Blueprint".to_owned()]));
        assert_eq!(watch.take_read(), None);
        assert!(watch.take_gone());
        assert!(!watch.take_gone());

        watch.stop();
        assert!(!watch.running());
    }

    #[test]
    fn squad_progress_accepts_only_the_expected_roster_and_resets_as_one_unit() {
        let mut progress = SquadProgress::default();
        progress.remember(squad_of(&["one", "two"], Some("/Lotus/A")));

        assert!(progress.squad(1).is_none());
        assert_eq!(
            progress
                .squad(2)
                .expect("matching squad")
                .screen_order
                .len(),
            2
        );
        assert!(!progress.resolved());

        progress.resolve();
        assert!(progress.resolved());

        progress.reset();
        assert!(!progress.resolved());
        assert!(progress.squad(2).is_none());
    }
}
