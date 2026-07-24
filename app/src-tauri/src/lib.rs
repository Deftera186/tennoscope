#![forbid(unsafe_code)]

use std::{
    fs,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use app_core::{AppCore, AppView};
use local_store::SnapshotMeta;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use warframe_acquisition::{
    CatalogCache, InventoryAcquirer, InventoryHttpTransport, LinuxProc, ProcessDiscovery,
    WfcdCatalogHttp,
};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SetupStatus {
    pub risk_accepted: bool,
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
    setup: SetupStatus,
    last_refresh_started: Option<Instant>,
    monitor_started: bool,
}
type SharedRuntime = Arc<Mutex<Runtime>>;

#[tauri::command]
fn get_view(state: State<'_, SharedRuntime>) -> Result<AppView, String> {
    state
        .lock()
        .map_err(|_| "application state is unavailable".to_owned())?
        .core
        .current_view()
        .map_err(|_| "application view is unavailable".to_owned())
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
async fn accept_risk_disclosure(state: State<'_, SharedRuntime>) -> Result<SetupStatus, String> {
    let shared = Arc::clone(state.inner());
    let result = tauri::async_runtime::spawn_blocking(move || {
        let mut runtime = shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?;
        let status = accept_setup_risk(&runtime.app_data.join("setup.json"))?;
        runtime.setup = status.clone();
        Ok(status)
    })
    .await
    .map_err(|_| "setup task failed".to_owned())?;
    if result.is_ok() {
        start_monitor(Arc::clone(state.inner()));
    }
    result
}

#[tauri::command]
async fn refresh_inventory(state: State<'_, SharedRuntime>) -> Result<AppView, String> {
    refresh_shared(Arc::clone(state.inner())).await
}

#[tauri::command]
fn load_fake_session(state: State<'_, SharedRuntime>) -> Result<AppView, String> {
    if !cfg!(debug_assertions) {
        return Err("fake session is unavailable in release builds".to_owned());
    }
    state
        .lock()
        .map_err(|_| "application state is unavailable".to_owned())?
        .core
        .load_fake_session()
        .map_err(|_| "fake session could not be loaded".to_owned())
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
    let catalog_http =
        WfcdCatalogHttp::new().map_err(|_| "catalog client could not be initialized".to_owned())?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    let catalog = match CatalogCache::new(app_data.join("catalog")).load(&catalog_http, now) {
        Ok(catalog) => catalog,
        Err(_) => {
            return shared
                .lock()
                .map_err(|_| "application state is unavailable".to_owned())?
                .core
                .record_catalog_failure("No valid WFCD catalog is available")
                .map_err(|_| "catalog failure could not be recorded".to_owned());
        }
    };
    let procfs = LinuxProc::new();
    let transport = InventoryHttpTransport::new()
        .map_err(|_| "inventory client could not be initialized".to_owned())?;
    let attempt = InventoryAcquirer::new(&procfs, &procfs, transport).acquire(catalog.index());
    let attempt = match attempt {
        Ok(result) => {
            let meta = SnapshotMeta::new(
                now.to_string(),
                "unknown".to_owned(),
                "warframe-memory".to_owned(),
            )
            .map_err(|_| "snapshot metadata could not be created".to_owned())?;
            Ok((result, meta))
        }
        Err(failure) => Err(failure),
    };
    shared
        .lock()
        .map_err(|_| "application state is unavailable".to_owned())?
        .core
        .finish_inventory_refresh(attempt, Some((catalog.source(), catalog.fetched_unix())))
        .map_err(|_| "inventory refresh could not be applied".to_owned())
}

fn initialize_runtime(app: &AppHandle) -> Result<SharedRuntime, Box<dyn std::error::Error>> {
    let app_data = app.path().app_data_dir()?;
    fs::create_dir_all(&app_data)?;
    let setup = read_setup_status(&app_data.join("setup.json")).map_err(std::io::Error::other)?;
    let core = AppCore::open(&app_data.join("warframe-helper.sqlite3"))?;
    Ok(Arc::new(Mutex::new(Runtime {
        core,
        app_data,
        setup,
        last_refresh_started: None,
        monitor_started: false,
    })))
}

fn inventory_log_path(pid: u32) -> Option<PathBuf> {
    let environment = fs::read(format!("/proc/{pid}/environ")).ok()?;
    let prefix = environment
        .split(|byte| *byte == 0)
        .find_map(|entry| entry.strip_prefix(b"WINEPREFIX="))?;
    let prefix = PathBuf::from(String::from_utf8(prefix.to_vec()).ok()?);
    [
        prefix.join("drive_c/users/steamuser/AppData/Local/Warframe/EE.log"),
        prefix.join("drive_c/users/steamuser/Local Settings/Application Data/Warframe/EE.log"),
    ]
    .into_iter()
    .find(|path| path.is_file())
}

fn monitor_game(shared: SharedRuntime) {
    let procfs = LinuxProc::new();
    let mut observed_process = None;
    let mut log = None::<(PathBuf, u64)>;
    loop {
        if let Ok(Some(process)) = procfs.discover() {
            if observed_process != Some(process.pid()) {
                observed_process = Some(process.pid());
                log = inventory_log_path(process.pid()).map(|path| {
                    let offset = fs::metadata(&path)
                        .map(|metadata| metadata.len())
                        .unwrap_or(0);
                    (path, offset)
                });
                let _ = refresh_blocking(&shared);
            } else if let Some((path, offset)) = &mut log {
                if let Ok(metadata) = fs::metadata(&*path) {
                    if metadata.len() < *offset {
                        *offset = 0;
                    }
                    if metadata.len() > *offset {
                        let available = metadata.len() - *offset;
                        let bounded = available.min(1024 * 1024);
                        if let Ok(mut file) = fs::File::open(&*path) {
                            let start = metadata.len() - bounded;
                            if file.seek(SeekFrom::Start(start)).is_ok() {
                                let mut bytes = Vec::with_capacity(bounded as usize);
                                if file.take(bounded).read_to_end(&mut bytes).is_ok()
                                    && contains_inventory_sync_trigger(&bytes)
                                {
                                    let _ = refresh_blocking(&shared);
                                }
                            }
                        }
                        *offset = metadata.len();
                    }
                }
            }
        } else {
            observed_process = None;
            log = None;
        }
        std::thread::sleep(Duration::from_secs(5));
    }
}

fn start_monitor(shared: SharedRuntime) {
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
            monitor_game(shared);
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
                start_monitor(Arc::clone(app.state::<SharedRuntime>().inner()));
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
