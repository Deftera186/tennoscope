#![forbid(unsafe_code)]

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use app_core::{AcquisitionPort, AppCore, AppView, InventoryRefreshOutcome};
use local_store::SnapshotMeta;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use warframe_acquisition::{
    CatalogCache, CatalogIndex, GameProcess, InventoryAcquirer, InventoryHttpTransport, LinuxProc,
    MarketPriceSource, MemoryReader, ProcessDiscovery, RelicCatalogCache, RelicRewardIndex,
    RewardCatalogEntry, RewardMemoryScanner, WfcdCatalogHttp, WfcdRelicCatalogHttp,
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
/// Upper bound on how long a single fissure mission is worth watching for.
const POLLER_LIFETIME: Duration = Duration::from_secs(45 * 60);

mod monitor;
mod overlay_window;
mod reward_log;
mod reward_observer;
mod reward_ocr;
mod reward_source;
pub use monitor::{
    LogMonitorDiagnostic, LogObservation, MonitorInput, MonitorMachine, MonitorResult,
};
pub use overlay_window::{
    OverlayGeometry, WindowRect, reward_overlay_geometry, warframe_window_rect_from_sway_tree,
};
pub use reward_log::{RewardLogEvent, RewardLogMachine};
pub use reward_observer::{
    RewardObservation, RewardObserverState, match_reward_text, normalize_ocr,
};
pub use reward_ocr::{ScreenRewardSource, best_match, read_cards};
pub use reward_source::{
    BoundMemoryRewardSource, LiveMemoryRewardState, MemoryRewardSource, RewardChoiceSet,
    RewardChoiceSource, RewardSourceCoordinator, RewardSourceDiagnostic, RewardSourceResult,
    VisualRewardSource,
};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SetupStatus {
    pub risk_accepted: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LocalPaths {
    pub setup: PathBuf,
    pub database: PathBuf,
}

pub fn resolve_local_paths(app_data: &Path) -> LocalPaths {
    let legacy_setup = app_data.join("setup.json");
    let legacy_database = app_data.join("warframe-helper.sqlite3");
    LocalPaths {
        setup: if legacy_setup.exists() {
            legacy_setup
        } else {
            app_data.join("tennoscope-setup.json")
        },
        database: if legacy_database.exists() {
            legacy_database
        } else {
            app_data.join("tennoscope.sqlite3")
        },
    }
}

pub fn read_setup_status(path: &Path) -> Result<SetupStatus, String> {
    match fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes).unwrap_or_default()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(SetupStatus::default()),
        Err(_) => Err("setup status could not be read".to_owned()),
    }
}

pub fn accept_setup_risk(path: &Path) -> Result<SetupStatus, String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "setup status could not be saved")?;
    }
    let status = SetupStatus {
        risk_accepted: true,
    };
    let temporary = path.with_extension("tmp");
    fs::write(
        &temporary,
        serde_json::to_vec(&status).map_err(|_| "setup status could not be saved")?,
    )
    .map_err(|_| "setup status could not be saved")?;
    fs::rename(temporary, path).map_err(|_| "setup status could not be saved")?;
    Ok(status)
}

pub fn contains_inventory_sync_trigger(bytes: &[u8]) -> bool {
    bytes.split(|byte| *byte == b'\n').any(|line| {
        line.windows(b"Inventory sync done".len())
            .any(|window| window == b"Inventory sync done")
    })
}

struct Runtime {
    core: AppCore,
    app_data: PathBuf,
    setup_path: PathBuf,
    setup: SetupStatus,
    last_refresh_started: Option<Instant>,
    refresh_in_flight: bool,
    overlay_preview_until: Option<Instant>,
    monitor_started: bool,
}
type SharedRuntime = Arc<Mutex<Runtime>>;

#[tauri::command]
async fn get_view(state: State<'_, SharedRuntime>) -> Result<AppView, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?
            .core
            .current_view()
            .map_err(|_| "application view is unavailable".to_owned())
    })
    .await
    .map_err(|_| "application view task failed".to_owned())?
}

#[tauri::command]
fn get_setup_status(state: State<'_, SharedRuntime>) -> Result<SetupStatus, String> {
    Ok(state
        .lock()
        .map_err(|_| "application state is unavailable".to_owned())?
        .setup
        .clone())
}

#[tauri::command]
async fn accept_risk_disclosure(
    app: AppHandle,
    state: State<'_, SharedRuntime>,
) -> Result<SetupStatus, String> {
    let shared = Arc::clone(state.inner());
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut runtime = shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?;
        let status = accept_setup_risk(&runtime.setup_path)?;
        runtime.setup = status.clone();
        Ok(status)
    })
    .await
    .map_err(|_| "setup task failed".to_owned())?;
    if result.is_ok() {
        start_monitor(Arc::clone(state.inner()), app);
    }
    result
}

#[tauri::command]
async fn refresh_inventory(state: State<'_, SharedRuntime>) -> Result<AppView, String> {
    refresh_shared(Arc::clone(state.inner())).await
}

#[tauri::command]
async fn load_fake_session(state: State<'_, SharedRuntime>) -> Result<AppView, String> {
    if !cfg!(debug_assertions) {
        return Err("fake session is unavailable in release builds".to_owned());
    }
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?
            .core
            .load_fake_session()
            .map_err(|_| "fake session could not be loaded".to_owned())
    })
    .await
    .map_err(|_| "fake session task failed".to_owned())?
}

async fn refresh_shared(shared: SharedRuntime) -> Result<AppView, String> {
    tauri::async_runtime::spawn_blocking(move || refresh_blocking(&shared))
        .await
        .map_err(|_| "inventory refresh task failed".to_owned())?
}

fn refresh_blocking(shared: &SharedRuntime) -> Result<AppView, String> {
    let app_data = {
        let mut runtime = shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?;
        if !runtime.setup.risk_accepted {
            return Err(
                "accept the read-only process-memory risk disclosure during setup first".to_owned(),
            );
        }
        if runtime.refresh_in_flight
            || runtime
                .last_refresh_started
                .is_some_and(|started| started.elapsed() < Duration::from_secs(15))
        {
            return runtime
                .core
                .current_view()
                .map_err(|_| "application view is unavailable".to_owned());
        }
        runtime.refresh_in_flight = true;
        runtime.last_refresh_started = Some(Instant::now());
        runtime.app_data.clone()
    };
    let port = ProductionAcquisition { app_data };
    let outcome = port.refresh();
    let result = apply_outcome(shared, outcome);
    if let Ok(mut runtime) = shared.lock() {
        runtime.refresh_in_flight = false;
    }
    result
}

struct ProductionAcquisition {
    app_data: PathBuf,
}
impl AcquisitionPort for ProductionAcquisition {
    fn refresh(&self) -> InventoryRefreshOutcome {
        let catalog_http = match WfcdCatalogHttp::new() {
            Ok(client) => client,
            Err(_) => return InventoryRefreshOutcome::catalog_failed(),
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let catalog =
            match CatalogCache::new(self.app_data.join("catalog")).load(&catalog_http, now) {
                Ok(catalog) => catalog,
                Err(_) => return InventoryRefreshOutcome::catalog_failed(),
            };
        let procfs = LinuxProc::new();
        let transport = match InventoryHttpTransport::new() {
            Ok(transport) => transport,
            Err(error) => {
                return InventoryRefreshOutcome::acquisition_failed(
                    warframe_acquisition::AcquisitionFailure::from_error(error),
                );
            }
        };
        let attempt = InventoryAcquirer::new(&procfs, &procfs, transport).acquire(catalog.index());
        match attempt {
            Ok(result) => {
                let meta = SnapshotMeta::new(
                    now.to_string(),
                    "unknown".to_owned(),
                    "warframe-memory".to_owned(),
                )
                .expect("nonblank production snapshot metadata");
                InventoryRefreshOutcome::success(
                    result,
                    meta,
                    catalog.source(),
                    catalog.fetched_unix(),
                )
            }
            Err(failure) => InventoryRefreshOutcome::acquisition_failed(failure),
        }
    }
}

struct CompletedOutcome(InventoryRefreshOutcome);
impl AcquisitionPort for CompletedOutcome {
    fn refresh(&self) -> InventoryRefreshOutcome {
        self.0.clone()
    }
}

fn apply_outcome(
    shared: &SharedRuntime,
    outcome: InventoryRefreshOutcome,
) -> Result<AppView, String> {
    shared
        .lock()
        .map_err(|_| "application state is unavailable".to_owned())?
        .core
        .refresh_from(&CompletedOutcome(outcome))
        .map_err(|_| "inventory health could not be applied".to_owned())
}

fn initialize_runtime(app: &AppHandle) -> Result<SharedRuntime, Box<dyn std::error::Error>> {
    let app_data = app.path().app_data_dir()?;
    fs::create_dir_all(&app_data)?;
    let paths = resolve_local_paths(&app_data);
    let setup = read_setup_status(&paths.setup).map_err(std::io::Error::other)?;
    let core = AppCore::open(&paths.database)?;
    Ok(Arc::new(Mutex::new(Runtime {
        core,
        app_data,
        setup_path: paths.setup,
        setup,
        last_refresh_started: None,
        refresh_in_flight: false,
        overlay_preview_until: None,
        monitor_started: false,
    })))
}

fn inventory_log_path(pid: u32) -> Option<PathBuf> {
    inventory_log_path_at(Path::new("/proc"), pid)
}

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

fn monitor_game(shared: SharedRuntime, app: AppHandle) {
    let procfs = LinuxProc::new();
    let mut machine = MonitorMachine::new(15);
    let mut reward_state = RewardObserverState::new(1, 1);
    let mut reward_log = RewardLogMachine::default();
    let mut announced_process = None;
    let mut early_reward_resolved = false;
    let mut pending_reward_squad = None::<PendingRewardSquad>;
    let incremental_reward_records = Arc::new(Mutex::new(BTreeMap::<String, String>::new()));
    let active_reward_scans = Arc::new(Mutex::new(BTreeSet::<String>::new()));
    let reward_generation = Arc::new(AtomicU64::new(0));
    let mut reward_memory = LiveMemoryRewardState::new(RewardMemoryScanner::new(
        256 * 1024,
        768 * 1024 * 1024,
        Duration::from_millis(1_500),
    ));
    let coordinator = RewardSourceCoordinator::new(cfg!(debug_assertions));
    let catalog = shared
        .lock()
        .ok()
        .and_then(|runtime| load_catalog(&runtime.app_data));
    if let (Some(catalog), Ok(mut runtime)) = (catalog.as_ref(), shared.lock()) {
        let _ = runtime.core.enrich_collection_from_catalog(catalog);
    }
    let reward_catalog = catalog
        .as_ref()
        .map(CatalogIndex::reward_entries)
        .unwrap_or_default();
    let relic_catalog = shared
        .lock()
        .ok()
        .and_then(|runtime| load_relic_catalog(&runtime.app_data));
    // EE.log reaches us seconds after the events it describes -- measured at ~7.5s on 2026-07-27,
    // by which time the fifteen-second reward screen can already be gone. The relic-load signal
    // arrives minutes ahead of the screen though, so it can arm a poller that watches for the cards
    // directly. The closed-set match is its own detector: only the reward screen yields four names
    // from this squad's relic pool.
    let visual_reads = Arc::new(Mutex::new(None::<Vec<String>>));
    let visual_polling = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let visual_screen_gone = Arc::new(std::sync::atomic::AtomicBool::new(false));

    loop {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let discovered = procfs.discover();
        let process = discovered.as_ref().ok().and_then(|process| *process);
        if process != announced_process {
            if process.is_some()
                && let Ok(mut runtime) = shared.lock()
            {
                let _ = runtime.core.record_game_process_ready();
            }
            announced_process = process;
        }
        let (input, log_bytes) = match discovered {
            Ok(None) => (MonitorInput::absent(now), Vec::new()),
            Err(error) => (MonitorInput::error(now, error), Vec::new()),
            Ok(Some(process)) => build_monitor_input(&machine, now, process.pid()),
        };
        let result = machine.tick(input);
        if result.refresh {
            let refresh = Arc::clone(&shared);
            spawn_monitor_refresh_task(move || {
                let _ = refresh_blocking(&refresh);
            });
        }
        if let Some(error) = result.acquisition_health {
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
                    LogMonitorDiagnostic::Unavailable => runtime
                        .core
                        .record_log_monitor_degraded("EE.log not found; retrying"),
                    LogMonitorDiagnostic::ReadFailed => runtime
                        .core
                        .record_log_monitor_failure("EE.log could not be read"),
                };
            }
        }
        for event in reward_log.observe_bytes(&log_bytes) {
            handle_reward_event(
                event,
                process,
                &procfs,
                catalog.as_ref(),
                relic_catalog.as_ref(),
                &reward_catalog,
                &mut reward_memory,
                &coordinator,
                &mut reward_state,
                &mut early_reward_resolved,
                &mut pending_reward_squad,
                &incremental_reward_records,
                &active_reward_scans,
                &reward_generation,
                &shared,
                &app,
                now,
                &visual_reads,
                &visual_polling,
                &visual_screen_gone,
            );
        }
        if let Some(names) = visual_reads.lock().ok().and_then(|mut slot| slot.take())
            && !early_reward_resolved
        {
            publish_reward_result(
                RewardSourceResult {
                    choices: RewardChoiceSet {
                        names,
                        source: RewardChoiceSource::Ocr,
                        elapsed: Duration::ZERO,
                    },
                    diagnostic: RewardSourceDiagnostic::MemoryFallback,
                },
                &mut reward_state,
                &shared,
                &app,
                &reward_catalog,
                now,
            );
            early_reward_resolved = true;
        }
        // The poller saw the screen disappear. Taking the overlay down here rather than waiting for
        // the shutdown line in EE.log saves the same flush delay that used to make the overlay miss
        // the screen entirely -- it is why the overlay used to linger for seconds after the window
        // it describes was gone. `Closed` still arrives later and does the rest of the teardown.
        if visual_screen_gone.swap(false, Ordering::AcqRel) && reward_state.miss().hide {
            overlay_window::hide_reward_overlay(&app);
        }
        if process.is_none() {
            reward_memory.clear();
            reward_generation.fetch_add(1, Ordering::AcqRel);
            if let Ok(mut records) = incremental_reward_records.lock() {
                records.clear();
            }
            if reward_state.miss().hide {
                overlay_window::hide_reward_overlay(&app);
            }
        }
        let poll_interval = if reward_log.reward_window_open() {
            Duration::from_millis(10)
        } else {
            Duration::from_millis(100)
        };
        std::thread::sleep(poll_interval);
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

#[allow(clippy::too_many_arguments)]
fn handle_reward_event(
    event: RewardLogEvent,
    process: Option<GameProcess>,
    procfs: &LinuxProc,
    catalog: Option<&CatalogIndex>,
    relic_catalog: Option<&RelicRewardIndex>,
    reward_catalog: &[RewardCatalogEntry],
    memory_state: &mut LiveMemoryRewardState,
    coordinator: &RewardSourceCoordinator,
    observer: &mut RewardObserverState,
    early_reward_resolved: &mut bool,
    pending_reward_squad: &mut Option<PendingRewardSquad>,
    incremental_reward_records: &Arc<Mutex<BTreeMap<String, String>>>,
    active_reward_scans: &Arc<Mutex<BTreeSet<String>>>,
    reward_generation: &Arc<AtomicU64>,
    shared: &SharedRuntime,
    app: &AppHandle,
    now: u64,
    visual_reads: &Arc<Mutex<Option<Vec<String>>>>,
    visual_polling: &Arc<std::sync::atomic::AtomicBool>,
    visual_screen_gone: &Arc<std::sync::atomic::AtomicBool>,
) {
    match event {
        RewardLogEvent::RewardWindowOpened => {
            if let Some(process) = process {
                let _ = procfs.reset_recent_writes(&process);
            }
        }
        RewardLogEvent::ResponderExpected { identity } => {
            if *early_reward_resolved {
                return;
            }
            let Some(process) = process else {
                return;
            };
            spawn_player_record_scan(
                identity,
                process,
                memory_state.candidates(),
                incremental_reward_records,
                active_reward_scans,
                reward_generation,
            );
        }
        RewardLogEvent::ResponderReceived { identity, is_local } => {
            if is_local || *early_reward_resolved {
                return;
            }
            let Some(process) = process else {
                return;
            };
            spawn_player_record_scan(
                identity,
                process,
                memory_state.candidates(),
                incremental_reward_records,
                active_reward_scans,
                reward_generation,
            );
        }
        RewardLogEvent::ResponsesComplete {
            screen_order,
            local_reward_path,
            local_identity,
            ..
        } => {
            *pending_reward_squad = Some(PendingRewardSquad {
                screen_order,
                local_reward_path,
                local_identity,
            });
            if let (Some(process), Some(squad)) = (process, pending_reward_squad.as_ref())
                && try_publish_player_records(
                    squad,
                    process,
                    procfs,
                    memory_state,
                    coordinator,
                    observer,
                    shared,
                    app,
                    reward_catalog,
                    now,
                )
            {
                *early_reward_resolved = true;
            }
        }
        RewardLogEvent::BaselineRequested { relic_paths } => {
            *early_reward_resolved = false;
            *pending_reward_squad = None;
            reward_generation.fetch_add(1, Ordering::AcqRel);
            if let Ok(mut records) = incremental_reward_records.lock() {
                records.clear();
            }
            if let Ok(mut scans) = active_reward_scans.lock() {
                scans.clear();
            }
            let candidates = catalog
                .zip(relic_catalog)
                .map(|(catalog, relics)| {
                    relics.candidates_for_projection_paths(&relic_paths, catalog)
                })
                .unwrap_or_default();
            let Some(_process) = process else {
                memory_state.clear();
                return;
            };
            memory_state.prepare_candidates(&candidates);
            spawn_reward_screen_poller(
                relic_pool_entries(&candidates, reward_catalog),
                visual_reads,
                visual_polling,
                visual_screen_gone,
            );
        }
        RewardLogEvent::ChoicesReady {
            expected_choices, ..
        } => {
            if *early_reward_resolved {
                return;
            }
            let Some(process) = process else {
                return;
            };
            let Some(squad) = pending_reward_squad
                .as_ref()
                .filter(|squad| squad.screen_order.len() == expected_choices)
            else {
                if let Ok(mut runtime) = shared.lock() {
                    let _ = runtime
                        .core
                        .record_capture_degraded("Structured reward records were incomplete");
                }
                return;
            };
            if try_publish_player_records(
                squad,
                process,
                procfs,
                memory_state,
                coordinator,
                observer,
                shared,
                app,
                reward_catalog,
                now,
            ) {
                *early_reward_resolved = true;
            } else if let Ok(mut runtime) = shared.lock() {
                let _ = runtime
                    .core
                    .record_capture_degraded("Structured reward records were incomplete");
            }
        }
        RewardLogEvent::Closed => {
            visual_polling.store(false, Ordering::Release);
            *early_reward_resolved = false;
            *pending_reward_squad = None;
            reward_generation.fetch_add(1, Ordering::AcqRel);
            if let Ok(mut records) = incremental_reward_records.lock() {
                records.clear();
            }
            if let Ok(mut scans) = active_reward_scans.lock() {
                scans.clear();
            }
            memory_state.clear();
            observer.miss();
            overlay_window::hide_reward_overlay(app);
            if let Ok(mut runtime) = shared.lock() {
                let _ = runtime.core.apply_reward_candidates(Vec::new());
            }
        }
    }
}

#[derive(Clone, Debug)]
struct PendingRewardSquad {
    screen_order: Vec<String>,
    local_reward_path: Option<String>,
    local_identity: Option<String>,
}

#[allow(clippy::too_many_arguments)]
fn try_publish_player_records(
    squad: &PendingRewardSquad,
    process: GameProcess,
    procfs: &LinuxProc,
    memory_state: &mut LiveMemoryRewardState,
    coordinator: &RewardSourceCoordinator,
    observer: &mut RewardObserverState,
    shared: &SharedRuntime,
    app: &AppHandle,
    reward_catalog: &[RewardCatalogEntry],
    now: u64,
) -> bool {
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
    let responders = squad
        .screen_order
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    // Matching a card against the squad's own relic pool rather than the whole catalog is what
    // keeps a garbled read on the right item; a few dozen names, not a few thousand.
    let pool = relic_pool_entries(memory_state.candidates(), reward_catalog);
    let mut memory = memory_state.bind(procfs, process);
    let result = coordinator
        .player_record_choices(
            &mut memory,
            &responders,
            squad.local_identity.as_deref(),
            local_choice.as_deref(),
        )
        .or_else(|| {
            coordinator.visual_choices(
                &mut ScreenRewardSource::new(),
                &pool,
                squad.screen_order.len(),
                local_choice.as_deref(),
                VISUAL_READ_DEADLINE,
            )
        });
    let Some(result) = result else {
        return false;
    };
    publish_reward_result(result, observer, shared, app, reward_catalog, now);
    true
}

/// Watch for the reward screen instead of waiting to be told about it.
///
/// EE.log is flushed by the game seconds after the fact, so the announcement can arrive after the
/// fifteen-second screen has closed. Relic loading is logged minutes earlier, which is early enough
/// to survive any flush delay, so that is what arms this. Each poll is a capture plus four crops,
/// roughly 150ms; the interval keeps it to about a tenth of a core while a fissure is running.
fn spawn_reward_screen_poller(
    pool: Vec<RewardCatalogEntry>,
    visual_reads: &Arc<Mutex<Option<Vec<String>>>>,
    visual_polling: &Arc<std::sync::atomic::AtomicBool>,
    visual_screen_gone: &Arc<std::sync::atomic::AtomicBool>,
) {
    spawn_reward_screen_poller_with(
        pool,
        visual_reads,
        visual_polling,
        visual_screen_gone,
        PollerTiming::live(),
        ScreenRewardSource::new,
    );
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
    pool: Vec<RewardCatalogEntry>,
    visual_reads: &Arc<Mutex<Option<Vec<String>>>>,
    visual_polling: &Arc<std::sync::atomic::AtomicBool>,
    visual_screen_gone: &Arc<std::sync::atomic::AtomicBool>,
    timing: PollerTiming,
    make_source: F,
) -> Option<std::thread::JoinHandle<()>>
where
    F: FnOnce() -> S + Send + 'static,
    S: VisualRewardSource + Send + 'static,
{
    // Claim the flag only once this call is definitely going to spawn. Taking it first and then
    // bailing on an empty pool leaves it set with no thread behind it, and since only a running
    // poller or the screen shutting down ever clears it, every later relic load in that fissure is
    // declined as a duplicate. The first relic pair is exactly when the pool can still be empty --
    // a vaulted relic resolves to no candidates -- so the poller was being poisoned before the
    // fissure that needed it had even started.
    if pool.is_empty() {
        #[cfg(debug_assertions)]
        warframe_acquisition::append_debug_line("[DEBUG-poller] arm declined: empty pool");
        return None;
    }
    let already_running = visual_polling.swap(true, Ordering::AcqRel);
    #[cfg(debug_assertions)]
    warframe_acquisition::append_debug_line(&format!(
        "[DEBUG-poller] arm pool={} already_running={already_running}",
        pool.len()
    ));
    if already_running {
        return None;
    }
    let visual_reads = Arc::clone(visual_reads);
    let visual_polling = Arc::clone(visual_polling);
    let visual_screen_gone = Arc::clone(visual_screen_gone);
    Some(std::thread::spawn(move || {
        let mut source = make_source();
        let deadline = Instant::now() + timing.lifetime;
        // Keep polling after the cards are found, to see the screen go away. The shutdown line in
        // EE.log arrives with the same flush delay as everything else, so hiding on it leaves the
        // overlay up for seconds after the screen it describes has gone.
        let mut found = false;
        let mut misses = 0_u32;
        while visual_polling.load(Ordering::Acquire) && Instant::now() < deadline {
            let outcome = VisualRewardSource::choices(&mut source, &pool);
            #[cfg(debug_assertions)]
            if let Err(reason) = &outcome {
                warframe_acquisition::append_debug_line(&format!(
                    "[DEBUG-poller] poll failed: {reason}"
                ));
            }
            match outcome {
                Ok(names) if names.len() == 4 => {
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
                        #[cfg(debug_assertions)]
                        warframe_acquisition::append_debug_line(
                            "[DEBUG-poller] reward screen gone",
                        );
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
                .find(|entry| entry.name == needle.choice_name())
                .map_or(0, |entry| entry.ducats),
        })
        .collect()
}

fn spawn_player_record_scan(
    identity: String,
    process: GameProcess,
    candidates: &[warframe_acquisition::RewardNeedle],
    records: &Arc<Mutex<BTreeMap<String, String>>>,
    active_scans: &Arc<Mutex<BTreeSet<String>>>,
    generation: &Arc<AtomicU64>,
) {
    if candidates.is_empty() {
        return;
    }
    let Ok(mut active) = active_scans.lock() else {
        return;
    };
    if !active.insert(identity.clone()) {
        return;
    }
    drop(active);

    let candidates = candidates.to_vec();
    let records = Arc::clone(records);
    let active_scans = Arc::clone(active_scans);
    let generation = Arc::clone(generation);
    let expected_generation = generation.load(Ordering::Acquire);
    std::thread::spawn(move || {
        let started = Instant::now();
        let procfs = LinuxProc::new();
        let scanner =
            RewardMemoryScanner::new(256 * 1024, 768 * 1024 * 1024, Duration::from_millis(1_500));
        let resolution = scan_player_record_until_ready(
            expected_generation,
            &generation,
            Duration::from_millis(750),
            || {
                scanner
                    .resolve_live_player_record(&procfs, &process, &candidates, &identity)
                    .unwrap_or(warframe_acquisition::RewardResolution::Incomplete)
            },
        );
        #[cfg(debug_assertions)]
        trace_responder_reward_scan(&identity, started.elapsed(), &resolution);
        store_player_record_if_current(
            expected_generation,
            &generation,
            &identity,
            resolution,
            &records,
        );
        release_player_record_scan(&identity, &active_scans);
    });
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

#[cfg(debug_assertions)]
fn trace_responder_reward_scan(
    identity: &str,
    elapsed: Duration,
    resolution: &warframe_acquisition::RewardResolution,
) {
    let suffix = identity
        .get(identity.len().saturating_sub(6)..)
        .unwrap_or(identity);
    warframe_acquisition::append_debug_line(&format!(
        "[DEBUG-responder] identity=…{suffix} elapsed_ms={} resolution={resolution:?}",
        elapsed.as_millis(),
    ));
}

fn publish_reward_result(
    result: RewardSourceResult,
    observer: &mut RewardObserverState,
    shared: &SharedRuntime,
    app: &AppHandle,
    reward_catalog: &[RewardCatalogEntry],
    now: u64,
) {
    let observations = result
        .choices
        .names
        .into_iter()
        .map(RewardObservation::certain)
        .collect::<Vec<_>>();
    let transition = observer.observe(observations);
    if transition.publish {
        apply_reward_observations(
            shared,
            reward_catalog,
            &transition.choices,
            &BTreeMap::new(),
        );
        overlay_window::show_reward_overlay(app);
        let _ = app.emit_to("reward-overlay", "reward-updated", ());
        spawn_market_price_fetch(&transition.choices, shared, app, reward_catalog, now);
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
        if result.diagnostic == RewardSourceDiagnostic::Disagreement {
            let _ = runtime
                .core
                .record_capture_degraded("memory and OCR reward recognition disagreed");
        }
    }
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
    now: u64,
) {
    let names = choices.to_vec();
    let shared = Arc::clone(shared);
    let app = app.clone();
    let reward_catalog = reward_catalog.to_vec();
    std::thread::spawn(move || {
        let Some(market) = warframe_acquisition::WarframeMarketHttp::new() else {
            return;
        };
        let prices = names
            .iter()
            .filter_map(|choice| Some((choice.name.clone(), market.lowest_sell(&choice.name)?)))
            .collect::<BTreeMap<_, _>>();
        if prices.is_empty() {
            if let Ok(mut runtime) = shared.lock() {
                let _ = runtime
                    .core
                    .record_market_degraded("No live warframe.market sellers for these rewards");
            }
            return;
        }
        apply_reward_observations(&shared, &reward_catalog, &names, &prices);
        if let Ok(mut runtime) = shared.lock() {
            let _ = runtime
                .core
                .record_market_ready(prices.len(), now.to_string());
        }
        let _ = app.emit_to("reward-overlay", "reward-updated", ());
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
                .find(|entry| entry.name == observation.name)
                .map_or(0, |entry| entry.ducats);
            let owned = view
                .collection()
                .items()
                .iter()
                .find(|item| item.name() == observation.name)
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

fn build_monitor_input(machine: &MonitorMachine, now: u64, pid: u32) -> (MonitorInput, Vec<u8>) {
    let Some(path) = inventory_log_path(pid) else {
        return (MonitorInput::running(now, pid, None), Vec::new());
    };
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(_) => return (MonitorInput::running_with_log_error(now, pid), Vec::new()),
    };
    let identity = format!("{}:{}", metadata.dev(), metadata.ino());
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
        Err(_) => return (MonitorInput::running_with_log_error(now, pid), Vec::new()),
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return (MonitorInput::running_with_log_error(now, pid), Vec::new());
    }
    let requested = (metadata.len() - offset).min(1024 * 1024);
    let mut bytes = Vec::with_capacity(requested as usize);
    if file.take(requested).read_to_end(&mut bytes).is_err() {
        return (MonitorInput::running_with_log_error(now, pid), Vec::new());
    }
    let log_bytes = bytes.clone();
    (
        MonitorInput::running(
            now,
            pid,
            Some(LogObservation::new(
                identity,
                offset + bytes.len() as u64,
                bytes,
            )),
        ),
        log_bytes,
    )
}

fn start_monitor(shared: SharedRuntime, app: AppHandle) {
    let should_start = shared
        .lock()
        .map(|mut runtime| {
            if runtime.monitor_started {
                false
            } else {
                runtime.monitor_started = true;
                true
            }
        })
        .unwrap_or(false);
    if should_start {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(3));
            monitor_game(shared, app);
        });
    }
}

#[tauri::command]
fn show_reward_overlay(app: AppHandle, state: State<'_, SharedRuntime>) {
    if let Ok(mut runtime) = state.lock() {
        runtime.overlay_preview_until = Some(Instant::now() + Duration::from_secs(30));
    }
    overlay_window::show_reward_overlay(&app);
}

#[tauri::command]
fn hide_reward_overlay(app: AppHandle, state: State<'_, SharedRuntime>) {
    if let Ok(mut runtime) = state.lock() {
        runtime.overlay_preview_until = None;
    }
    overlay_window::hide_reward_overlay(&app);
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            if cfg!(debug_assertions) {
                app.handle().plugin(
                    tauri_plugin_log::Builder::default()
                        .level(log::LevelFilter::Info)
                        .build(),
                )?;
            }
            let runtime = initialize_runtime(app.handle())?;
            let should_refresh = runtime
                .lock()
                .map(|state| state.setup.risk_accepted)
                .unwrap_or(false);
            app.manage(runtime);
            if should_refresh {
                start_monitor(
                    Arc::clone(app.state::<SharedRuntime>().inner()),
                    app.handle().clone(),
                );
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_view,
            get_setup_status,
            accept_risk_disclosure,
            refresh_inventory,
            load_fake_session,
            show_reward_overlay,
            hide_reward_overlay
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
