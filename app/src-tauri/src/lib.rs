#![forbid(unsafe_code)]

use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use app_core::{AcquisitionPort, AppCore, AppView, InventoryRefreshOutcome};
use local_store::SnapshotMeta;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use warframe_acquisition::{
    CatalogCache, InventoryAcquirer, InventoryHttpTransport, LinuxProc, ProcessDiscovery,
    RewardCatalogEntry, WfcdCatalogHttp,
};
use warframe_domain::RewardCandidate;

mod monitor;
mod overlay_window;
mod reward_observer;
pub use monitor::{
    LogMonitorDiagnostic, LogObservation, MonitorInput, MonitorMachine, MonitorResult,
};
pub use reward_observer::{
    RewardObservation, RewardObserverState, match_reward_text, normalize_ocr,
};
pub use overlay_window::{OverlayGeometry, reward_overlay_geometry};

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
        if runtime
            .last_refresh_started
            .is_some_and(|started| started.elapsed() < Duration::from_secs(15))
        {
            return runtime
                .core
                .current_view()
                .map_err(|_| "application view is unavailable".to_owned());
        }
        runtime.last_refresh_started = Some(Instant::now());
        runtime.app_data.clone()
    };
    let port = ProductionAcquisition { app_data };
    let outcome = port.refresh();
    apply_outcome(shared, outcome)
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
    let mut reward_state = RewardObserverState::new(2, 2);
    let reward_catalog = shared
        .lock()
        .ok()
        .and_then(|runtime| load_reward_catalog(&runtime.app_data))
        .unwrap_or_default();
    loop {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let input = match procfs.discover() {
            Ok(None) => MonitorInput::absent(now),
            Err(error) => MonitorInput::error(now, error),
            Ok(Some(process)) => build_monitor_input(&machine, now, process.pid()),
        };
        let result = machine.tick(input);
        if result.refresh {
            let _ = refresh_blocking(&shared);
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
        observe_rewards(
            &shared,
            &app,
            now,
            machine.process_pid().is_some(),
            &reward_catalog,
            &mut reward_state,
        );
        std::thread::sleep(Duration::from_secs(5));
    }
}

fn load_reward_catalog(app_data: &Path) -> Option<Vec<RewardCatalogEntry>> {
    let source = WfcdCatalogHttp::new().ok()?;
    let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?.as_secs();
    CatalogCache::new(app_data.join("catalog"))
        .load(&source, now)
        .ok()
        .map(|catalog| catalog.index().reward_entries())
}

fn observe_rewards(
    shared: &SharedRuntime,
    app: &AppHandle,
    now: u64,
    game_running: bool,
    catalog: &[RewardCatalogEntry],
    state: &mut RewardObserverState,
) {
    if !game_running || catalog.is_empty() {
        let transition = state.miss();
        if transition.hide {
            overlay_window::hide_reward_overlay(app);
        }
        return;
    }
    match reward_observer::observe_live_rewards(catalog) {
        Ok(choices) if choices.len() == 4 => {
            let transition = state.observe(choices);
            if transition.show {
                apply_reward_observations(shared, catalog, &transition.choices);
                overlay_window::show_reward_overlay(app);
            }
            if let Ok(mut runtime) = shared.lock() {
                let _ = runtime.core.record_capture_ready(now.to_string());
            }
        }
        Ok(_) => {
            let transition = state.miss();
            if transition.hide {
                overlay_window::hide_reward_overlay(app);
                if let Ok(mut runtime) = shared.lock() {
                    let _ = runtime.core.apply_reward_candidates(Vec::new());
                }
            }
        }
        Err(message) => {
            let transition = state.miss();
            if transition.hide {
                overlay_window::hide_reward_overlay(app);
            }
            if let Ok(mut runtime) = shared.lock() {
                let _ = runtime.core.record_capture_degraded(message);
            }
        }
    }
}

fn apply_reward_observations(
    shared: &SharedRuntime,
    catalog: &[RewardCatalogEntry],
    observations: &[RewardObservation],
) {
    let Ok(mut runtime) = shared.lock() else { return };
    let Ok(view) = runtime.core.current_view() else { return };
    let candidates = observations
        .iter()
        .filter_map(|observation| {
            let ducats = catalog.iter().find(|entry| entry.name == observation.name)
                .map_or(0, |entry| entry.ducats);
            let owned = view.collection().items().iter()
                .find(|item| item.name() == observation.name)
                .map_or(0, |item| item.quantity());
            RewardCandidate::new(
                &observation.name,
                0,
                ducats,
                owned,
                false,
                observation.confidence,
            ).ok()
        })
        .collect();
    let _ = runtime.core.apply_reward_candidates(candidates);
}

fn build_monitor_input(machine: &MonitorMachine, now: u64, pid: u32) -> MonitorInput {
    let Some(path) = inventory_log_path(pid) else {
        return MonitorInput::running(now, pid, None);
    };
    let metadata = match fs::metadata(&path) {
        Ok(metadata) => metadata,
        Err(_) => return MonitorInput::running_with_log_error(now, pid),
    };
    let identity = format!("{}:{}", metadata.dev(), metadata.ino());
    if machine.process_pid() != Some(pid) {
        return MonitorInput::running(
            now,
            pid,
            Some(LogObservation::new(identity, metadata.len(), Vec::new())),
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
        return MonitorInput::running(
            now,
            pid,
            Some(LogObservation::new(identity, offset, Vec::new())),
        );
    }
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(_) => return MonitorInput::running_with_log_error(now, pid),
    };
    if file.seek(SeekFrom::Start(offset)).is_err() {
        return MonitorInput::running_with_log_error(now, pid);
    }
    let requested = (metadata.len() - offset).min(1024 * 1024);
    let mut bytes = Vec::with_capacity(requested as usize);
    if file.take(requested).read_to_end(&mut bytes).is_err() {
        return MonitorInput::running_with_log_error(now, pid);
    }
    MonitorInput::running(
        now,
        pid,
        Some(LogObservation::new(
            identity,
            offset + bytes.len() as u64,
            bytes,
        )),
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
            load_fake_session
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
