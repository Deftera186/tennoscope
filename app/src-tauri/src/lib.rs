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
    CatalogCache, CatalogIndex, CollectionPriceCache, GameProcess, InventoryAcquirer,
    InventoryHttpTransport, LinuxProc, MarketPriceCache, MemoryReader, PriceDumpError,
    ProcessDiscovery, RelicCatalogCache, RelicRewardIndex, RelicsRunHttp, RewardCatalogEntry,
    RewardMemoryScanner, WarmOutcome, WfcdCatalogHttp, WfcdRelicCatalogHttp, dump_is_current,
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
pub use overlay_window::{OverlayGeometry, WindowRect, reward_overlay_geometry};
pub use reward_log::{RewardLogEvent, RewardLogMachine};
pub use reward_observer::{
    RewardObservation, RewardObserverState, match_reward_text, normalize_ocr,
};
pub use reward_ocr::{
    MAX_CARDS, ScreenRewardSource, best_match, card_block_left, card_block_width, ocr_crop,
    read_cards, warframe_window_from_xwininfo_tree,
};
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
    LocalPaths {
        setup: app_data.join("tennoscope-setup.json"),
        database: app_data.join("tennoscope.sqlite3"),
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
    /// One relic sweep at a time; see `spawn_owned_relic_sweep`.
    relic_sweep_in_flight: bool,
    overlay_preview_until: Option<Instant>,
    monitor_started: bool,
    // Survives across missions on purpose: the same relic pools recur all evening, so a price
    // fetched two runs ago is one this run does not have to make. Shared with the collection, so
    // a pool warmed mid-mission also prices those items in the browser.
    live_prices: MarketPriceCache,
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
        start_collection_prices(Arc::clone(state.inner()));
        start_monitor(Arc::clone(state.inner()), app);
    }
    result
}

#[tauri::command]
async fn refresh_inventory(state: State<'_, SharedRuntime>) -> Result<AppView, String> {
    refresh_shared(Arc::clone(state.inner())).await
}

/// Price the named items live, because the player asked about them.
///
/// Paced at the documented three requests a second, so a full page of forty-eight takes about
/// sixteen seconds. It runs to completion rather than returning early: the frontend's own poll
/// surfaces each price as it lands, so the wait is visible as prices appearing rather than as a
/// button that does nothing.
#[tauri::command]
async fn refresh_prices(
    item_ids: Vec<String>,
    state: State<'_, SharedRuntime>,
) -> Result<AppView, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let (names, cache) = {
            let runtime = shared
                .lock()
                .map_err(|_| "application state is unavailable".to_owned())?;
            (
                runtime
                    .core
                    .market_names_for(&item_ids)
                    .map_err(|_| "collection items could not be resolved".to_owned())?,
                runtime.live_prices.clone(),
            )
        };
        if let Some(market) = warframe_acquisition::WarframeMarketHttp::new() {
            let outcome = cache.warm(&market, &names, warframe_acquisition::MARKET_MIN_GAP);
            // The live path shares the overlay's row, since both answer "could we reach
            // warframe.market just now". The dump's date lives in its own row and is not
            // disturbed by this.
            if let Some(failure) = outcome.failure()
                && let Ok(mut runtime) = shared.lock()
            {
                let _ = runtime.core.record_market_degraded(failure);
            }
        }
        shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?
            .core
            .current_view()
            .map_err(|_| "application view is unavailable".to_owned())
    })
    .await
    .map_err(|_| "price refresh task failed".to_owned())?
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
    // A refresh is the only thing that can add a relic to the collection, including the very first
    // one, which is what turns an empty first-install snapshot into 65 relics the startup sweep
    // never saw. The lock is released above on purpose: the sweep takes about 22 seconds.
    if result.is_ok() {
        spawn_owned_relic_sweep(Arc::clone(shared));
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
    let mut core = AppCore::open(&paths.database)?;
    let live_prices = MarketPriceCache::new();
    core.set_live_prices(live_prices.clone());
    Ok(Arc::new(Mutex::new(Runtime {
        core,
        app_data,
        setup_path: paths.setup,
        setup,
        last_refresh_started: None,
        refresh_in_flight: false,
        relic_sweep_in_flight: false,
        overlay_preview_until: None,
        monitor_started: false,
        live_prices,
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
    // Survives across missions on purpose: the same relic pools recur all evening, so a price
    // fetched two runs ago is one this run does not have to make.
    let price_cache = shared
        .lock()
        .map(|runtime| runtime.live_prices.clone())
        .unwrap_or_default();
    let visual_pool: SharedRelicPool = Arc::new(Mutex::new(Vec::new()));
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
                &visual_pool,
                &price_cache,
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
                &price_cache,
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
    visual_pool: &SharedRelicPool,
    price_cache: &MarketPriceCache,
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
            ..
        } => {
            *pending_reward_squad = Some(PendingRewardSquad {
                screen_order,
                local_reward_path,
            });
            // The screen read needs a window, not a process handle, but a dead game has neither:
            // requiring the process keeps a vanished game from burning the retry deadline.
            if process.is_some()
                && let Some(squad) = pending_reward_squad.as_ref()
                && try_publish_player_records(
                    squad,
                    memory_state,
                    coordinator,
                    observer,
                    shared,
                    app,
                    reward_catalog,
                    price_cache,
                    visual_screen_gone,
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
            // Publish the pool before arming, and on every baseline rather than only the first.
            // A running poller reads this cell each poll, so a relic that loads after it started
            // still reaches it -- which is the common case, since the baseline fires on the second
            // of four relics.
            let entries = relic_pool_entries(&candidates, reward_catalog);
            if let Ok(mut pool) = visual_pool.lock()
                && entries.len() > pool.len()
            {
                *pool = entries.clone();
            }
            spawn_market_price_warm(&entries, price_cache);
            spawn_reward_screen_poller(
                visual_pool,
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
            if process.is_none() {
                return;
            }
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
                memory_state,
                coordinator,
                observer,
                shared,
                app,
                reward_catalog,
                price_cache,
                visual_screen_gone,
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

/// The squad roster in screen order, plus the one reward EE.log states outright. `local_identity`
/// used to ride along for the memory scan's per-player attribution; the screen read needs only the
/// local player's reward name, as a check that the four cards it read include the one the log
/// already confirmed.
#[derive(Clone, Debug)]
struct PendingRewardSquad {
    screen_order: Vec<String>,
    local_reward_path: Option<String>,
}

/// Publish the four cards, read off the screen.
///
/// Memory used to be tried first here and the screen kept as a fallback. It never once answered on
/// a live run: ten reward events across host and client sessions on 2026-07-27 all resolved
/// `Incomplete`, and the only per-player record ever confirmed belongs to the local player, whose
/// reward EE.log already states exactly and which arrives here as `local_choice`. Hosting was
/// expected to be the case that worked and was measured doing the same thing, so the scan bought
/// nothing but 130-200MB of reads per reward screen. The scanner and its fixtures stay in
/// `warframe-acquisition` for the attribution question to be reopened against evidence.
#[allow(clippy::too_many_arguments)]
fn try_publish_player_records(
    squad: &PendingRewardSquad,
    memory_state: &LiveMemoryRewardState,
    coordinator: &RewardSourceCoordinator,
    observer: &mut RewardObserverState,
    shared: &SharedRuntime,
    app: &AppHandle,
    reward_catalog: &[RewardCatalogEntry],
    price_cache: &MarketPriceCache,
    visual_screen_gone: &std::sync::atomic::AtomicBool,
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
    // Matching a card against the squad's own relic pool rather than the whole catalog is what
    // keeps a garbled read on the right item; a few dozen names, not a few thousand.
    let pool = relic_pool_entries(memory_state.candidates(), reward_catalog);
    let Some(result) = coordinator.visual_choices(
        &mut ScreenRewardSource::new(),
        &pool,
        squad.screen_order.len(),
        local_choice.as_deref(),
        VISUAL_READ_DEADLINE,
        visual_screen_gone,
    ) else {
        return false;
    };
    publish_reward_result(
        result,
        observer,
        shared,
        app,
        reward_catalog,
        price_cache,
        now,
    );
    true
}

/// Watch for the reward screen instead of waiting to be told about it.
///
/// EE.log is flushed by the game seconds after the fact, so the announcement can arrive after the
/// fifteen-second screen has closed. Relic loading is logged minutes earlier, which is early enough
/// to survive any flush delay, so that is what arms this. Each poll is a capture plus four crops,
/// roughly 150ms; the interval keeps it to about a tenth of a core while a fissure is running.
fn spawn_reward_screen_poller(
    pool: &SharedRelicPool,
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
pub type SharedRelicPool = Arc<Mutex<Vec<RewardCatalogEntry>>>;

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
    pool: &SharedRelicPool,
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
    let pool_size = pool.lock().map(|pool| pool.len()).unwrap_or(0);
    if pool_size == 0 {
        #[cfg(debug_assertions)]
        warframe_acquisition::append_debug_line("[DEBUG-poller] arm declined: empty pool");
        return None;
    }
    let already_running = visual_polling.swap(true, Ordering::AcqRel);
    #[cfg(debug_assertions)]
    warframe_acquisition::append_debug_line(&format!(
        "[DEBUG-poller] arm pool={pool_size} already_running={already_running}"
    ));
    if already_running {
        return None;
    }
    let pool = Arc::clone(pool);
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
            // Re-read the pool every poll rather than capturing it at arm time. Squadmates' relics
            // are still loading when this thread starts, and a card missing from the pool fails the
            // whole screen.
            let current = pool.lock().map(|pool| pool.clone()).unwrap_or_default();
            if current.is_empty() {
                std::thread::sleep(timing.interval);
                continue;
            }
            let outcome = VisualRewardSource::choices(&mut source, &current);
            #[cfg(debug_assertions)]
            if let Err(reason) = &outcome {
                warframe_acquisition::append_debug_line(&format!(
                    "[DEBUG-poller] poll failed: {reason}"
                ));
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

#[allow(clippy::too_many_arguments)]
fn publish_reward_result(
    result: RewardSourceResult,
    observer: &mut RewardObserverState,
    shared: &SharedRuntime,
    app: &AppHandle,
    reward_catalog: &[RewardCatalogEntry],
    price_cache: &MarketPriceCache,
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
        overlay_window::show_reward_overlay(app, transition.choices.len());
        let _ = app.emit_to("reward-overlay", "reward-updated", ());
        spawn_market_price_fetch(
            &transition.choices,
            shared,
            app,
            reward_catalog,
            price_cache,
            now,
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
    price_cache: &MarketPriceCache,
    now: u64,
) {
    let names = choices.to_vec();
    let shared = Arc::clone(shared);
    let app = app.clone();
    let reward_catalog = reward_catalog.to_vec();
    let price_cache = price_cache.clone();
    std::thread::spawn(move || {
        // Anything the pool warmed while the mission was still running is already here, so the
        // common case does no requests at all and the overlay never shows a dash. Only a reward
        // the warm pass missed -- a pool that never loaded, an API that was down then -- is
        // fetched now, and it is fetched with no gap because the screen is already up.
        let mut prices = names
            .iter()
            .filter_map(|choice| Some((choice.name.clone(), price_cache.get(&choice.name)?)))
            .collect::<BTreeMap<_, _>>();
        let missing = names
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
        // An oversize response is worth saying even when the cache carried the screen, because it
        // is the failure that stops every future price and nothing else would report it. An empty
        // screen with no failure to name means no request was made at all.
        let failure = outcome.failure().or_else(|| {
            prices
                .is_empty()
                .then_some("warframe.market pricing is unavailable for these rewards")
        });
        if let Some(failure) = failure
            && let Ok(mut runtime) = shared.lock()
        {
            let _ = runtime.core.record_market_degraded(failure);
        }
        if prices.is_empty() {
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

/// Price the collection: cached table first so items are priced before any request is made, then
/// at most one download for the day's dump, then a live sweep of the relics the player owns.
///
/// There is nothing to schedule for the dump itself. The whole collection is priced by a single
/// file, so that part runs once at start and is done -- no queue, no worker, no rate limiting,
/// because there are no per-item requests to pace. A cached table that is already as new as
/// anything published skips the download entirely; the file is 3.9 MB and it changes once a day.
/// Relics are the exception: the dump's relic prices run up to 1.5x high, so they are priced live
/// instead, which is where the per-item pacing in `sweep_owned_relics` comes from.
///
/// The dumps lag, so the usual launch re-downloads the same file it already had. The refreshed
/// table adopts the cached one's swept relic prices when the date matches, or the download would
/// throw away every price the last sweep spent 22 seconds learning.
fn start_collection_prices(shared: SharedRuntime) {
    std::thread::spawn(move || {
        let Some(app_data) = shared.lock().ok().map(|runtime| runtime.app_data.clone()) else {
            return;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        let cache = CollectionPriceCache::new(&app_data);
        let cached = cache.load_cached();
        if let Some(table) = cached.clone()
            && let Ok(mut runtime) = shared.lock()
        {
            let priced = table.len();
            let date = table.dump_date().to_owned();
            runtime.core.set_collection_prices(Arc::new(table));
            let _ = runtime.core.record_collection_prices_ready(priced, date);
        }
        if !cached
            .as_ref()
            .is_some_and(|table| dump_is_current(table.dump_date(), now))
        {
            let Some(source) = RelicsRunHttp::new() else {
                return;
            };
            match cache.refresh(&source, now, cached.as_ref()) {
                Ok(table) => {
                    if let Ok(mut runtime) = shared.lock() {
                        let priced = table.len();
                        let date = table.dump_date().to_owned();
                        runtime.core.set_collection_prices(Arc::new(table));
                        let _ = runtime.core.record_collection_prices_ready(priced, date);
                    }
                }
                Err(error) => {
                    // A dump that could not be read and a disk that could not be written are different
                    // problems with different fixes, and only one of them is warframe.market's.
                    let message = match error {
                        PriceDumpError::Malformed => "No warframe.market price dump could be read",
                        PriceDumpError::CacheWrite => {
                            "Prices loaded but could not be saved for the next start"
                        }
                    };
                    if let Ok(mut runtime) = shared.lock() {
                        let _ = runtime.core.record_collection_prices_degraded(message);
                    }
                    return;
                }
            }
        }
        spawn_owned_relic_sweep(shared);
    });
}

/// Sweep whatever relics are not priced yet, off the calling thread.
///
/// A first-ever launch prices nothing: the snapshot does not exist yet, so the startup sweep finds
/// no owned relics and returns having done nothing anybody can fix without a restart. This is what
/// fires after an inventory refresh has produced a snapshot -- and after any later refresh that
/// adds a relic the player did not own before. It costs nothing when there is nothing new, because
/// the sweep's filter is per-relic.
///
/// One sweep at a time. Two overlapping sweeps would spend the same requests twice against an API
/// with a documented rate limit, and a first run is exactly where refreshes and the startup sweep
/// collide: the sweep takes about 22 seconds and a refresh may be repeated after 15.
fn spawn_owned_relic_sweep(shared: SharedRuntime) {
    let Ok(mut runtime) = shared.lock() else {
        return;
    };
    if runtime.relic_sweep_in_flight {
        return;
    }
    runtime.relic_sweep_in_flight = true;
    let app_data = runtime.app_data.clone();
    let live_prices = runtime.live_prices.clone();
    drop(runtime);
    std::thread::spawn(move || {
        sweep_owned_relics(&shared, &CollectionPriceCache::new(&app_data), &live_prices);
        if let Ok(mut runtime) = shared.lock() {
            runtime.relic_sweep_in_flight = false;
        }
    });
}

/// Prices the player's owned relics live and writes the results back into the persisted table, so
/// they outlive the 15-minute live cache and survive a restart.
///
/// Never holds the runtime lock across the sweep: a real collection is 65 relics, about 22 seconds
/// at the 3-requests/second floor `MarketPriceCache::warm` enforces, and the view is rebuilt every
/// 2.5 seconds. Everything the sweep needs is copied out under one short lock before `warm` -- which
/// paces the requests itself -- is ever called; the table is only written back under a second short
/// lock once the sweep has finished.
///
/// Bounded by `owned_relic_market_names`, never by every relic the dump lists, and then by the one
/// question worth asking per relic: does this one already have a swept price? A swept price lives
/// exactly as long as the dump it arrived with (see `PriceTable::adopt_swept`), so that single
/// filter is the whole freshness rule -- there is no second cadence to keep in step with the first.
fn sweep_owned_relics(
    shared: &SharedRuntime,
    cache: &CollectionPriceCache,
    live_prices: &MarketPriceCache,
) {
    let Ok((owned, table)) = shared.lock().map(|runtime| {
        (
            runtime.core.owned_relic_market_names().unwrap_or_default(),
            runtime.core.collection_prices(),
        )
    }) else {
        return;
    };
    let Some(table) = table else {
        return;
    };
    let to_sweep: Vec<String> = owned
        .into_iter()
        .filter(|name| !table.has_swept_price(name))
        .collect();
    if to_sweep.is_empty() {
        return;
    }
    let Some(market) = warframe_acquisition::WarframeMarketHttp::new() else {
        return;
    };
    if let Ok(mut runtime) = shared.lock() {
        let _ = runtime
            .core
            .record_collection_prices_sweeping(to_sweep.len());
    }
    let outcome = live_prices.warm(&market, &to_sweep, warframe_acquisition::MARKET_MIN_GAP);
    // ponytail: read-modify-write against a table this thread does not hold a lock on. Safe because
    // this sweep is the only writer of relic prices and `spawn_owned_relic_sweep` admits one at a
    // time; if a second writer is ever added, move the mutation under the runtime lock.
    let mut updated = (*table).clone();
    for name in &to_sweep {
        if let Some(price) = live_prices.get(name) {
            updated.insert_live(name, price);
        }
    }
    let store_result = cache.store_table(&updated);
    let priced = updated.len();
    let date = updated.dump_date().to_owned();
    let Ok(mut runtime) = shared.lock() else {
        return;
    };
    runtime.core.set_collection_prices(Arc::new(updated));
    if let Some(failure) = outcome.failure() {
        let _ = runtime.core.record_collection_prices_degraded(failure);
    } else if store_result.is_err() {
        let _ = runtime.core.record_collection_prices_degraded(
            "Swept relic prices could not be saved for the next start",
        );
    } else {
        let _ = runtime.core.record_collection_prices_ready(priced, date);
    }
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
    // The preview has no screen to measure, so it shows the full-squad strip.
    overlay_window::show_reward_overlay(&app, reward_ocr::MAX_CARDS);
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
    // Run the whole app on X11, including under Wayland. The game is a Wine/Proton client and so is
    // always an X11 window, and X11 is the only display server that will tell a program where
    // another application's window is, or let it place a window above that application's fullscreen
    // surface. Wayland exposes neither by design: `wlr-layer-shell` covers the second half but is
    // absent on GNOME, and no protocol covers the first. Sharing the game's display server is what
    // makes the overlay land in the right place on every window manager rather than on some of them.
    //
    // Left alone if there is no X server to run on, so a session without one still gets the app
    // itself; only the overlay degrades.
    //
    // ponytail: this puts the *main* window on XWayland too, which a compositor doing fractional
    // scaling will render blurry. Split the overlay into its own X11 process if that ever matters
    // more than having one.
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_some() {
        gtk::gdk::set_allowed_backends("x11");
    }
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
                start_collection_prices(Arc::clone(app.state::<SharedRuntime>().inner()));
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
            refresh_prices,
            load_fake_session,
            show_reward_overlay,
            hide_reward_overlay
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
