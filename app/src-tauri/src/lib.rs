#![forbid(unsafe_code)]

use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use app_core::{AcquisitionPort, AppCore, AppView, InventoryRefreshOutcome, PricingProgress};
use local_store::{SnapshotInstant, SnapshotMeta, StoreError};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, State};
use warframe_acquisition::{
    CatalogCache, CollectionPriceCache, InventoryAcquirer, InventoryHttpTransport,
    MarketPriceCache, ProcessDiscovery, RelicsRunHttp, WarmOutcome, WfcdCatalogHttp,
    dump_is_current, latest_dump,
};

/// The platform's process-memory backend. Both sides implement `MemoryReader` and
/// `ProcessDiscovery`, which is the seam everything below the app already works through -- naming
/// the concrete type once here is what keeps `cfg` out of the call sites.
#[cfg(unix)]
use warframe_acquisition::LinuxProc as GameMemory;
#[cfg(windows)]
use warframe_acquisition::WindowsProc as GameMemory;

mod kiosk_geometry;
mod kiosk_log;
mod kiosk_ocr;
mod kiosk_scroll;
mod kiosk_view;
#[cfg(target_os = "linux")]
pub mod linux_renderer;
pub mod market_account;
pub mod monitor;
mod overlay_window;
pub mod report;
pub mod reward_capture;
mod reward_log;
mod reward_observer;
mod reward_ocr;
mod reward_source;
pub use kiosk_log::KioskLogEvent;
pub use kiosk_ocr::{BasketRow, GridCell};
pub use kiosk_view::{KioskState, KioskView};
pub use overlay_window::{
    OverlayGeometry, WindowRect, kiosk_overlay_geometry, placement_notice, reward_overlay_geometry,
};
pub use reward_capture::x11::{largest_warframe_window, warframe_window_from_xwininfo_tree};
pub use reward_log::{RewardLogEvent, RewardLogMachine};
pub use reward_observer::{
    RewardObservation, RewardObserverState, match_reward_text, normalize_ocr,
};
pub use reward_ocr::{
    MAX_CARDS, ScreenRewardSource, TESSERACT_EXECUTABLE, best_match, card_block_left,
    card_block_width, luma, normalize_contrast, ocr_crop, prepare_crop, read_cards, read_cards_in,
    tesseract_program, threshold_inverted,
};
pub use reward_source::{
    BoundMemoryRewardSource, LiveMemoryRewardState, MemoryRewardSource, RewardChoiceSet,
    RewardChoiceSource, RewardSourceCoordinator, RewardSourceDiagnostic, RewardSourceResult,
    VisualRewardSource,
};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct SetupStatus {
    pub risk_accepted: bool,
    pub desktop_capture_action_available: bool,
}

#[derive(Deserialize, Serialize)]
struct PersistedSetupStatus {
    risk_accepted: bool,
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
        Ok(bytes) => Ok(serde_json::from_slice::<PersistedSetupStatus>(&bytes)
            .map(|stored| SetupStatus {
                risk_accepted: stored.risk_accepted,
                ..SetupStatus::default()
            })
            .unwrap_or_default()),
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
        ..SetupStatus::default()
    };
    let temporary = path.with_extension("tmp");
    fs::write(
        &temporary,
        serde_json::to_vec(&PersistedSetupStatus {
            risk_accepted: status.risk_accepted,
        })
        .map_err(|_| "setup status could not be saved")?,
    )
    .map_err(|_| "setup status could not be saved")?;
    fs::rename(temporary, path).map_err(|_| "setup status could not be saved")?;
    Ok(status)
}

#[derive(Clone, Copy, Debug)]
struct CaptureSetupInput {
    game_running: bool,
    x11_game_window: bool,
    wlroots_available: bool,
    kwin_available: bool,
    portal_session_live: bool,
}

fn desktop_capture_action_available(input: CaptureSetupInput) -> bool {
    let capture_ready = input.x11_game_window
        || input.wlroots_available
        || input.kwin_available
        || input.portal_session_live;
    input.game_running && !capture_ready
}

fn current_setup_status(stored: SetupStatus, game_running: bool) -> SetupStatus {
    let mut input = CaptureSetupInput {
        game_running,
        x11_game_window: false,
        wlroots_available: false,
        kwin_available: false,
        portal_session_live: false,
    };
    if game_running {
        input.x11_game_window = reward_capture::x11_game_window_available();
        #[cfg(target_os = "linux")]
        if !input.x11_game_window {
            input.wlroots_available = reward_capture::direct::available();
            input.kwin_available =
                !input.wlroots_available && reward_capture::kwin::available_cached();
            input.portal_session_live = !input.wlroots_available
                && !input.kwin_available
                && reward_capture::portal::PortalCapture::has_live_session();
        }
    }
    SetupStatus {
        risk_accepted: stored.risk_accepted,
        desktop_capture_action_available: desktop_capture_action_available(input),
    }
}

pub fn contains_inventory_sync_trigger(bytes: &[u8]) -> bool {
    bytes.split(|byte| *byte == b'\n').any(|line| {
        line.windows(b"Inventory sync done".len())
            .any(|window| window == b"Inventory sync done")
    })
}

/// The presence socket and the two answers that only mean anything beside it: whether presence is
/// following the game reader, and what the socket was last asked to hold.
///
/// Kept together because no caller has ever wanted one without the others -- a status change is a
/// socket write and a new `wanted` in the same breath, and going offline is all three at once.
#[derive(Default)]
struct PresenceHold {
    /// Open only while a status is being held. Dropping it is how this application goes offline:
    /// warframe.market has no settable `offline`, and a client that stays connected claiming
    /// `invisible` is still a client the server counts as connected.
    link: Option<warframe_status::StatusLink>,
    /// Whether presence follows the game reader rather than a choice the player made.
    automatic: bool,
    /// What the socket was last asked to hold. Kept beside the link rather than read back off it:
    /// the link reports only what the server has confirmed, and that is `None` for the first
    /// moment of every connection.
    wanted: Option<warframe_status::Presence>,
}

impl PresenceHold {
    /// Hold `wanted`, or go offline when it is `None`.
    ///
    /// `token` is called only when a socket has to be opened, so switching status on a live
    /// connection never reaches for the credential.
    fn request(
        &mut self,
        wanted: Option<warframe_status::Presence>,
        automatic: bool,
        token: impl FnOnce() -> Result<String, String>,
    ) -> Result<(), String> {
        self.automatic = automatic;
        match wanted {
            None => self.release(),
            Some(status) => {
                match &self.link {
                    Some(link) => link.set(status),
                    None => {
                        self.link = Some(warframe_status::StatusLink::connect(token()?, status));
                    }
                }
                self.wanted = Some(status);
            }
        }
        Ok(())
    }

    /// Whether presence tracks the game reader, and so has to be re-derived rather than only set
    /// when the player presses something.
    fn is_automatic(&self) -> bool {
        self.automatic
    }

    /// Move an automatic hold onto `derived`, which the caller read off the game reader. A no-op
    /// when the socket is already holding it, so a poll that changes nothing writes nothing.
    fn follow_reader(&mut self, derived: warframe_status::Presence) {
        if !self.automatic || self.wanted == Some(derived) {
            return;
        }
        self.wanted = Some(derived);
        if let Some(link) = &self.link {
            link.set(derived);
        }
    }

    /// Close the socket and stop following anything: the credential it authenticated with is gone,
    /// and holding it open would keep announcing an account the player has unlinked.
    fn sign_out(&mut self) {
        self.release();
        self.automatic = false;
    }

    /// What other players see, which is the server's answer rather than the request that was made.
    fn view(&self) -> app_core::PresenceView {
        app_core::PresenceView {
            status: self.link.as_ref().and_then(|link| link.committed()),
            wanted: self.wanted,
            auto: self.automatic,
        }
    }

    fn release(&mut self) {
        self.link = None;
        self.wanted = None;
    }
}

/// Whether an inventory refresh may start, which is one question with two reasons to answer no:
/// another refresh is running, or one finished recently enough that repeating it would only cost
/// the player a process-memory read for the same answer.
///
/// Kept together because neither field means anything alone -- `started` without `running` is a
/// debounce, and `running` without `started` is a refresh nothing will ever release.
#[derive(Default)]
struct RefreshWindow {
    /// When the last refresh began, held after it ends so the debounce outlives it.
    started: Option<Instant>,
    running: bool,
}

impl RefreshWindow {
    /// How long after a refresh starts another is refused. A process-memory read costs the player
    /// a stutter, and the inventory it reads does not change faster than this.
    const DEBOUNCE: Duration = Duration::from_secs(15);

    /// Claim the window for a refresh starting now, or answer `false` when one may not start.
    /// A `true` answer means the caller owns the window until it calls [`Self::finish`].
    fn begin(&mut self) -> bool {
        if self.running
            || self
                .started
                .is_some_and(|started| started.elapsed() < Self::DEBOUNCE)
        {
            return false;
        }
        self.running = true;
        self.started = Some(Instant::now());
        true
    }

    /// Release the window. The debounce keeps running from when the refresh began.
    fn finish(&mut self) {
        self.running = false;
    }
}

struct Runtime {
    core: AppCore,
    app_data: PathBuf,
    setup_path: PathBuf,
    setup: SetupStatus,
    refresh: RefreshWindow,
    monitor_started: bool,
    /// Updated by the existing process watcher; setup status never starts a second watcher.
    game_running: bool,
    /// Last-known EE.log path, cached so reports can include it even after the game exits.
    last_ee_log_path: Option<PathBuf>,
    // Survives across missions on purpose: the same relic pools recur all evening, so a price
    // fetched two runs ago is one this run does not have to make. Shared with the collection, so
    // a pool warmed mid-mission also prices those items in the browser.
    live_prices: MarketPriceCache,
    market: market_account::MarketSession,
    presence: PresenceHold,
}
type SharedRuntime = Arc<Mutex<Runtime>>;

#[tauri::command]
async fn get_view(state: State<'_, SharedRuntime>) -> Result<AppView, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let mut runtime = shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?;
        // The socket commits a status some milliseconds after it is asked to, on its own thread.
        // Reading it here means the switch settles on the next poll the frontend already makes,
        // rather than needing an event channel of its own for one field.
        publish_presence(&mut runtime)
    })
    .await
    .map_err(|_| "application view task failed".to_owned())?
}

/// Assemble the GitHub-safe report text only (used by "Copy diagnostics").
#[tauri::command]
async fn collect_report_text(
    state: State<'_, SharedRuntime>,
    app: AppHandle,
) -> Result<String, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let runtime = shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?;
        let view = runtime
            .core
            .current_view()
            .map_err(|_| "application view is unavailable".to_owned())?;
        let health_json = serde_json::to_string_pretty(&view.health())
            .map_err(|_| "health could not be serialized".to_owned())?;
        let request = build_report_request(&app, &runtime, &health_json, false);
        report::assemble_report_text(
            &request.meta,
            &request.health_json,
            report::EeLogState::NotRequested,
        )
    })
    .await
    .map_err(|_| "report task failed".to_owned())?
}

/// Write the report folder and return the text plus the folder path.
#[tauri::command]
async fn collect_report(
    state: State<'_, SharedRuntime>,
    app: AppHandle,
) -> Result<report::CollectedReport, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let runtime = shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?;
        let view = runtime
            .core
            .current_view()
            .map_err(|_| "application view is unavailable".to_owned())?;
        let health_json = serde_json::to_string_pretty(&view.health())
            .map_err(|_| "health could not be serialized".to_owned())?;
        let request = build_report_request(&app, &runtime, &health_json, true);
        // The copy below can be hundreds of MB of EE.log. Holding the lock across it freezes the
        // UI, the monitor tick and the reward poller for its duration.
        drop(runtime);
        report::collect_report(&request)
    })
    .await
    .map_err(|_| "report task failed".to_owned())?
}

fn build_report_request(
    app: &AppHandle,
    runtime: &Runtime,
    health_json: &str,
    want_ee_log: bool,
) -> report::ReportRequest {
    let os_arch = format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH);
    let profile = if cfg!(debug_assertions) {
        "pre-release".to_owned()
    } else {
        "stable".to_owned()
    };
    let version = app.package_info().version.to_string();
    let log_dir = app
        .path()
        .app_log_dir()
        .unwrap_or_else(|_| runtime.app_data.clone());
    let ee_log_wanted = want_ee_log;
    let ee_log_path = if ee_log_wanted {
        GameMemory::new()
            .discover()
            .ok()
            .flatten()
            .and_then(|process| monitor::inventory_log_path(process.pid()))
            .or_else(|| runtime.last_ee_log_path.clone())
    } else {
        None
    };
    report::ReportRequest {
        meta: report::ReportMeta {
            version,
            profile,
            os_arch,
            timestamp: report::utc_civil(),
            log_dir,
            app_data: runtime.app_data.clone(),
        },
        health_json: health_json.to_owned(),
        ee_log_wanted,
        ee_log_path,
    }
}

/// What the presence switch is asked for, including going offline.
///
/// `None` is offline, and offline is the socket closing rather than a value sent over it: the
/// server has no settable `offline`, and a connection held open claiming otherwise is still a
/// connection.
#[tauri::command]
async fn set_market_presence(
    state: State<'_, SharedRuntime>,
    status: Option<warframe_status::Presence>,
    auto: bool,
) -> Result<AppView, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let mut runtime = shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?;
        let wanted = if auto {
            Some(auto_presence(&runtime))
        } else {
            status
        };
        // Split the borrow: opening a socket needs the credential, which lives in a sibling field.
        let Runtime {
            presence, market, ..
        } = &mut *runtime;
        presence.request(wanted, auto, || {
            let token = market
                .token()
                .map_err(|error| market_account::failure_message(error).to_owned())?
                .ok_or_else(|| "No warframe.market account is linked".to_owned())?;
            Ok(token.expose().to_owned())
        })?;
        let outcome = publish_presence(&mut runtime);
        match &outcome {
            Ok(_) => log::info!("market: presence ok"),
            Err(error) => log::warn!("market: presence failed: {error}"),
        }
        outcome
    })
    .await
    .map_err(|_| "presence task failed".to_owned())?
}

/// ponytail: the game reader's own health, mapped straight across -- `ready` means the process is
/// open, which is as much as this application currently knows. The upgrade is EE.log activity,
/// which is also what would let the `activity` object the API accepts be filled in.
fn auto_presence(runtime: &Runtime) -> warframe_status::Presence {
    let ready = runtime
        .core
        .current_view()
        .is_ok_and(|view| view.health().game_reader().state() == app_core::HealthState::Ready);
    if ready {
        warframe_status::Presence::Ingame
    } else {
        warframe_status::Presence::Online
    }
}

/// Copy what the socket says onto the view. Read rather than assumed: the switch shows what other
/// players see, which is the server's answer and not the request that was made.
///
/// Automatic mode is re-derived here rather than only when it is switched on. It maps the game
/// reader's state, and that state changes on its own -- computing it once at the press would mean
/// "follow the game" stopped following the moment Warframe was launched.
fn publish_presence(runtime: &mut Runtime) -> Result<AppView, String> {
    if runtime.presence.is_automatic() {
        let derived = auto_presence(runtime);
        runtime.presence.follow_reader(derived);
    }
    let presence = runtime.presence.view();
    runtime
        .core
        .set_presence(presence)
        .map_err(|_| "application view is unavailable".to_owned())
}

#[tauri::command]
async fn get_setup_status(state: State<'_, SharedRuntime>) -> Result<SetupStatus, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let (stored, game_running) = {
            let runtime = shared
                .lock()
                .map_err(|_| "application state is unavailable".to_owned())?;
            (runtime.setup.clone(), runtime.game_running)
        };
        Ok(current_setup_status(stored, game_running))
    })
    .await
    .map_err(|_| "setup task failed".to_owned())?
}

#[tauri::command]
async fn accept_risk_disclosure(
    app: AppHandle,
    state: State<'_, SharedRuntime>,
) -> Result<SetupStatus, String> {
    let shared = Arc::clone(state.inner());
    let result = tauri::async_runtime::spawn_blocking(move || {
        // The lock is dropped before `accept_setup_risk` writes and before `current_setup_status`
        // probes the display server: both can block for seconds, and holding the central runtime
        // mutex across them stalls every other command and monitor update.
        let setup_path = {
            let runtime = shared
                .lock()
                .map_err(|_| "application state is unavailable".to_owned())?;
            runtime.setup_path.clone()
        };
        let stored = accept_setup_risk(&setup_path)?;
        let game_running = {
            let mut runtime = shared
                .lock()
                .map_err(|_| "application state is unavailable".to_owned())?;
            runtime.setup = stored.clone();
            runtime.game_running
        };
        Ok(current_setup_status(stored, game_running))
    })
    .await
    .map_err(|_| "setup task failed".to_owned())?;
    if result.is_ok() {
        start_collection_prices(Arc::clone(state.inner()));
        monitor::start(Arc::clone(state.inner()), app);
    }
    result
}

#[tauri::command]
async fn authorize_screen_capture(
    app: AppHandle,
    state: State<'_, SharedRuntime>,
) -> Result<SetupStatus, String> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (app, state);
        Err("desktop capture is unavailable".to_owned())
    }

    #[cfg(target_os = "linux")]
    {
        tauri::async_runtime::spawn_blocking(reward_capture::portal::PortalCapture::authorize)
            .await
            .map_err(|_| "desktop capture task failed".to_owned())?
            .map_err(str::to_owned)?;

        let status = get_setup_status(state.clone()).await?;
        if status.risk_accepted {
            monitor::start(Arc::clone(state.inner()), app);
        }
        Ok(status)
    }
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
///
/// What comes back is written into the persisted price table, not left in the 15-minute live
/// cache. A price the player deliberately asked for is the best number the app has for that item,
/// and letting it expire back to a day-old figure would discard a request they spent.
#[tauri::command]
async fn refresh_prices(
    item_ids: Vec<String>,
    state: State<'_, SharedRuntime>,
) -> Result<AppView, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let (names, cache, app_data) = {
            let runtime = shared
                .lock()
                .map_err(|_| "application state is unavailable".to_owned())?;
            (
                runtime
                    .core
                    .market_names_for(&item_ids)
                    .map_err(|_| "collection items could not be resolved".to_owned())?,
                runtime.live_prices.clone(),
                runtime.app_data.clone(),
            )
        };
        if let Some(market) = warframe_acquisition::WarframeMarketHttp::new() {
            let (outcome, unpriced) = warm_with_progress(&shared, &market, &names, &cache);
            let persisted = store_checked_prices(
                &shared,
                &CollectionPriceCache::new(&app_data),
                &names,
                &cache,
                &unpriced,
            );
            if let Ok(mut runtime) = shared.lock() {
                // The live path shares the overlay's row, since both answer "could we reach
                // warframe.market just now". The dump's date lives in its own row and is not
                // disturbed by this.
                if let Some(failure) = outcome.failure() {
                    let _ = runtime.core.record_market_degraded(failure);
                }
                // The collection price row is the only one that can report a price which reached
                // memory but not disk, where it would not survive the next start.
                match persisted {
                    // Only ever refreshes a row that already reports health. A page refresh knows
                    // nothing about the dump download, so writing Ready here would clear a startup
                    // failure -- "No warframe.market price dump could be read" -- and leave the row
                    // reading healthy over whatever stale table that failure left behind. But if the
                    // row is Degraded from a transient failure (a market blip or failed disk write),
                    // we need to clear it with a successful refresh. The discriminator is last_success:
                    // None means "no successful startup price load ever happened", sticky across
                    // refreshes; Some(_) means "there was once a working price table", clearable on
                    // transient failures. Only clear Degraded if there was prior success.
                    Some((priced, date, true))
                        if runtime
                            .core
                            .health()
                            .collection_prices()
                            .last_success()
                            .is_some() =>
                    {
                        let _ = runtime.core.record_collection_prices_ready(priced, date);
                    }
                    Some((priced, date, true)) => {
                        // Ready with no prior success: keep it as is (likely Just cached from startup)
                        let _ = runtime.core.record_collection_prices_ready(priced, date);
                    }
                    Some((_, _, false)) => {
                        let _ = runtime
                            .core
                            .record_collection_prices_degraded(CHECKED_PRICES_UNSAVED);
                    }
                    _ => {}
                }
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

/// Now, as Unix seconds in a string.
///
/// The same form `refresh_blocking` already stamps snapshot metadata with, rather than a second
/// vocabulary for the same idea. Only two things read it: the reconciliation, which normalises
/// both forms to an instant anyway, and the interface, whose `snapshotFreshness` already parses
/// epoch seconds -- so formatting a calendar date here would add a date algorithm to produce a
/// string nothing needs in that shape.
fn now_unix_seconds() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
        .to_string()
}

/// Fetch the account and publish it, turning any failure into a health message rather than an
/// error the interface has to interpret.
///
/// The transport is built per call rather than held: it is cheap, and a client built at startup on
/// a machine that was offline then would be a client that never works.
///
/// The runtime mutex is taken three times rather than once and held throughout, because `get_view`
/// polls that same mutex every 2.5 seconds from the frontend. Holding it across the item fetch and
/// `list_mine` -- each a real HTTP call with its own timeout, plus whatever the pacer makes them
/// wait -- would freeze that poll, and with it the whole interface, for as long as warframe.market
/// takes to answer. Cheap state is read under the first lock and carried out by value; the network
/// happens with no lock held; the result is published under a final lock taken only to write it.
/// If a sign-out happened while this fetch was unlocked, `generation` is now stale: whatever the
/// fetch found describes a credential the session has since discarded, and must be dropped rather
/// than resurrecting `items` that `forget` just cleared. Returns the current view in that case,
/// unchanged.
fn discard_if_stale(
    runtime: &mut Runtime,
    generation: market_account::Generation,
) -> Option<Result<AppView, String>> {
    if !runtime.market.is_stale(generation) {
        return None;
    }
    Some(
        runtime
            .core
            .current_view()
            .map_err(|_| "application view is unavailable".to_owned()),
    )
}

fn publish_account(shared: &SharedRuntime) -> Result<AppView, String> {
    let (pacer, token, backing, cached_items, collection, snapshot, generation) = {
        let runtime = shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?;
        let token = runtime
            .market
            .token()
            .map_err(|error| market_account::failure_message(error).to_owned())?;
        let collection = runtime
            .core
            .collection_for_reconciliation()
            .map_err(|_| "the collection could not be read".to_owned())?;
        let snapshot = runtime
            .core
            .latest_snapshot_meta()
            .map_err(|_| "the snapshot could not be read".to_owned())?;
        (
            runtime.live_prices.pacer(),
            token,
            runtime.market.backing(),
            runtime.market.cached_items(),
            collection,
            snapshot,
            runtime.market.generation(),
        )
    };

    let Some(token) = token else {
        let mut runtime = shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?;
        if let Some(result) = discard_if_stale(&mut runtime, generation) {
            return result;
        }
        return runtime
            .core
            .set_market_account(app_core::MarketAccountView::unlinked())
            .map_err(|_| "application view is unavailable".to_owned());
    };
    let Ok(transport) = warframe_market::MarketHttp::new(pacer) else {
        let mut runtime = shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?;
        if let Some(result) = discard_if_stale(&mut runtime, generation) {
            return result;
        }
        return runtime
            .core
            .record_market_account_failure(market_account::failure_message(
                warframe_market::MarketError::Unreachable,
            ))
            .map_err(|_| "application view is unavailable".to_owned());
    };
    let now = now_unix_seconds();

    // Unlocked from here: an item fetch (only on the first call after launch) and `list_mine` are
    // both real network round trips.
    let outcome = fetch_account(
        &transport,
        &token,
        backing,
        cached_items,
        &collection,
        snapshot.as_ref(),
        &now,
    );

    let mut runtime = shared
        .lock()
        .map_err(|_| "application state is unavailable".to_owned())?;
    if let Some(result) = discard_if_stale(&mut runtime, generation) {
        return result;
    }
    match outcome {
        Ok(FetchedAccount {
            items,
            renewed,
            view,
        }) => {
            runtime.market.set_items(items);
            match runtime.market.adopt(renewed) {
                Ok(()) => runtime
                    .core
                    .set_market_account(view)
                    .map_err(|_| "application view is unavailable".to_owned()),
                // The account was read successfully but the renewed credential could not be kept.
                // The fetch is not wasted for that: it is reported as a health problem rather than
                // silently discarded, and the next refresh will simply ask again.
                Err(error) => runtime
                    .core
                    .record_market_account_failure(market_account::failure_message(error))
                    .map_err(|_| "application view is unavailable".to_owned()),
            }
        }
        Err(error) => runtime
            .core
            .record_market_account_failure(market_account::failure_message(error))
            .map_err(|_| "application view is unavailable".to_owned()),
    }
}

/// What a successful, unlocked account fetch produced: the item table to keep for next time (newly
/// fetched, or simply the one that was already cached), the token to store (renewed on every use),
/// and the view to publish.
struct FetchedAccount {
    items: std::sync::Arc<warframe_market::MarketItems>,
    renewed: warframe_market::MarketToken,
    view: app_core::MarketAccountView,
}

/// The network part of `publish_account`, done with no runtime lock held.
///
/// A refused credential is not an error here: it is the account's own state, and the interface has
/// a repair for it. Its own token is not carried anywhere since nothing renews on a 401.
fn fetch_account(
    transport: &dyn warframe_market::MarketTransport,
    token: &warframe_market::MarketToken,
    backing: warframe_market::CredentialBacking,
    cached_items: Option<std::sync::Arc<warframe_market::MarketItems>>,
    collection: &warframe_domain::Collection,
    snapshot: Option<&SnapshotMeta>,
    now: &str,
) -> Result<FetchedAccount, warframe_market::MarketError> {
    let items = match cached_items {
        Some(items) => items,
        None => std::sync::Arc::new(warframe_market::MarketItems::fetch(transport)?),
    };
    match warframe_market::list_mine(transport, token) {
        Ok((orders, renewed)) => {
            let reconciled = app_core::reconcile_orders(&orders, &items, collection, snapshot);
            let view = app_core::MarketAccountView::linked(backing, reconciled, now.to_owned())
                .with_listable(&items, collection);
            Ok(FetchedAccount {
                items,
                renewed,
                view,
            })
        }
        Err(warframe_market::MarketError::Unauthorized) => Ok(FetchedAccount {
            items,
            renewed: token.clone(),
            view: app_core::MarketAccountView::needs_relink(),
        }),
        Err(error) => Err(error),
    }
}

#[tauri::command]
async fn market_status(state: State<'_, SharedRuntime>) -> Result<AppView, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || publish_account(&shared))
        .await
        .map_err(|_| "market status task failed".to_owned())?
}

/// Exchange an email and password for a token, then publish the account.
///
/// The password reaches this function, is passed once to the signin call, and is dropped. It is
/// not stored, not echoed back, and not part of any value this command returns.
#[tauri::command]
async fn market_sign_in(
    email: String,
    password: String,
    state: State<'_, SharedRuntime>,
) -> Result<AppView, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let pacer = {
            let runtime = shared
                .lock()
                .map_err(|_| "application state is unavailable".to_owned())?;
            runtime.live_prices.pacer()
        };
        let transport = warframe_market::MarketHttp::new(pacer).map_err(|_| {
            market_account::failure_message(warframe_market::MarketError::Unreachable).to_owned()
        })?;
        let token = warframe_market::sign_in(&transport, &email, &password)
            .map_err(|error| market_account::failure_message(error).to_owned())?;
        shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?
            .market
            .adopt(token)
            .map_err(|error| market_account::failure_message(error).to_owned())?;
        publish_account(&shared)
    })
    .await
    .map_err(|_| "sign-in task failed".to_owned())?
}

/// Link with a token pasted from a signed-in browser session.
///
/// Verified before it is stored, so a bad paste fails at the paste box rather than at the next
/// action.
#[tauri::command]
async fn market_link_token(
    token: String,
    state: State<'_, SharedRuntime>,
) -> Result<AppView, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let pacer = {
            let runtime = shared
                .lock()
                .map_err(|_| "application state is unavailable".to_owned())?;
            runtime.live_prices.pacer()
        };
        let transport = warframe_market::MarketHttp::new(pacer).map_err(|_| {
            market_account::failure_message(warframe_market::MarketError::Unreachable).to_owned()
        })?;
        let verified =
            warframe_market::verify_token(&transport, &warframe_market::MarketToken::new(token))
                .map_err(|error| market_account::failure_message(error).to_owned())?;
        shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?
            .market
            .adopt(verified)
            .map_err(|error| market_account::failure_message(error).to_owned())?;
        publish_account(&shared)
    })
    .await
    .map_err(|_| "link task failed".to_owned())?
}

#[tauri::command]
async fn market_sign_out(state: State<'_, SharedRuntime>) -> Result<AppView, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        {
            let mut runtime = shared
                .lock()
                .map_err(|_| "application state is unavailable".to_owned())?;
            runtime
                .market
                .forget()
                .map_err(|error| market_account::failure_message(error).to_owned())?;
            runtime.presence.sign_out();
        }
        shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?
            .core
            .set_market_account(app_core::MarketAccountView::unlinked())
            .map_err(|_| "application view is unavailable".to_owned())
    })
    .await
    .map_err(|_| "sign-out task failed".to_owned())?
}

#[tauri::command]
async fn refresh_orders(state: State<'_, SharedRuntime>) -> Result<AppView, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = publish_account(&shared);
        match &outcome {
            Ok(_) => log::info!("market: order refresh ok"),
            Err(error) => log::warn!("market: order refresh failed: {error}"),
        }
        outcome
    })
    .await
    .map_err(|_| "order refresh task failed".to_owned())?
}

/// Take one order down, then refresh so the list reflects what the account now holds.
///
/// Refuses an id that is not on the account view currently held, rather than asking
/// warframe.market about it: a stale or fabricated id must not reach a delete, since it acts
/// irreversibly on a real account.
#[tauri::command]
async fn remove_order(
    order_id: String,
    state: State<'_, SharedRuntime>,
) -> Result<AppView, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        {
            let runtime = shared
                .lock()
                .map_err(|_| "application state is unavailable".to_owned())?;
            if let Err(message) =
                market_account::authorize_removal(runtime.core.market_account(), &order_id)
            {
                return Err(message.to_owned());
            }
        }
        write_then_refresh(&shared, "remove", |transport, token| {
            warframe_market::delete_order(transport, token, &order_id)
        })
    })
    .await
    .map_err(|_| "order removal task failed".to_owned())?
}

/// Publish a sell listing for one row of the collection.
///
/// The item is named by its collection row id -- the whole key, rank suffix or relic tier
/// included, never a market id: a market id from the frontend is a value nothing checked, and it
/// decides which item a real listing is published against. `authorize_sell` resolves it here,
/// refusing rows this device does not hold and rows whose listing would need details no row
/// knows, and returning the rank, subtype and per-trade size the row's own identity implies.
///
/// Price and quantity do come from the caller, because they are the two things the player is
/// choosing. `create_order` bounds both against what the API accepts before spending a request.
#[tauri::command]
async fn create_order(
    collection_id: String,
    platinum: u32,
    quantity: u32,
    visible: bool,
    state: State<'_, SharedRuntime>,
) -> Result<AppView, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        // The table comes out as its own handle and the listing borrows it, so the runtime lock is
        // gone before the slow part: the write below is a network round trip, and holding the lock
        // through it would stall every poll. The collection is read under a second, equally short
        // lock rather than carried out, because it is a copy the core may be replacing in between.
        let items = {
            let runtime = shared
                .lock()
                .map_err(|_| "application state is unavailable".to_owned())?;
            runtime.market.cached_items().ok_or_else(|| {
                market_account::failure_message(warframe_market::MarketError::Unreachable)
                    .to_owned()
            })?
        };
        let listing = {
            let runtime = shared
                .lock()
                .map_err(|_| "application state is unavailable".to_owned())?;
            let collection = runtime
                .core
                .collection_for_reconciliation()
                .map_err(|error| error.to_string())?;
            market_account::authorize_sell(&items, &collection, &collection_id)
                .map_err(str::to_owned)?
        };
        write_then_refresh(&shared, "create", |transport, token| {
            warframe_market::create_order(
                transport,
                token,
                warframe_market::NewSellOrder::from_listing(listing, platinum, quantity, visible),
            )
        })
    })
    .await
    .map_err(|_| "listing task failed".to_owned())?
}

/// Lower one order to the quantity the collection says is held.
///
/// The quantity is never taken from the caller: it is derived here from the reconciliation's own
/// `OrderStatus::Overshoot { owned }` on the order named, which is the only quantity this command
/// will ever send. An order that is not currently flagged as an overshoot -- including one whose
/// id is not on the held list at all -- is refused before anything is sent, because a value the
/// frontend supplied unchecked would be a write of anything it liked to a real account.
#[tauri::command]
async fn set_order_quantity(
    order_id: String,
    state: State<'_, SharedRuntime>,
) -> Result<AppView, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let quantity = {
            let runtime = shared
                .lock()
                .map_err(|_| "application state is unavailable".to_owned())?;
            market_account::authorize_quantity_write(runtime.core.market_account(), &order_id)
                .map_err(|message| message.to_owned())?
        };
        write_then_refresh(&shared, "quantity", |transport, token| {
            warframe_market::set_order_quantity(transport, token, &order_id, quantity)
        })
    })
    .await
    .map_err(|_| "order update task failed".to_owned())?
}

/// Edit the price and the count of a listing the player is looking at.
///
/// Unlike the derived quantity repair beside it, both numbers are the player's own choice -- and
/// everything this device can check about them is checked here: `authorize_update` bounds the
/// count against the holding of the row the order names, and the market crate bounds the price and
/// the count against what the API accepts. Neither bound is the frontend's to enforce alone,
/// because a write past either acts on a real account.
#[tauri::command]
async fn update_order(
    order_id: String,
    platinum: u32,
    quantity: u32,
    state: State<'_, SharedRuntime>,
) -> Result<AppView, String> {
    let shared = Arc::clone(state.inner());
    tauri::async_runtime::spawn_blocking(move || {
        {
            let runtime = shared
                .lock()
                .map_err(|_| "application state is unavailable".to_owned())?;
            let collection = runtime
                .core
                .collection_for_reconciliation()
                .map_err(|error| error.to_string())?;
            market_account::authorize_update(
                runtime.core.market_account(),
                &collection,
                &order_id,
                quantity,
            )
            .map_err(str::to_owned)?;
        }
        write_then_refresh(&shared, "update", |transport, token| {
            warframe_market::update_order(transport, token, &order_id, platinum, quantity)
        })
    })
    .await
    .map_err(|_| "order edit task failed".to_owned())?
}

/// Both writes share this: perform it, keep whatever token came back, then refresh.
///
/// The refresh happens whether or not the renewed token could be stored. A write that changed the
/// account and left the list showing the old state would invite the player to press the same
/// button again; a credential that failed to store is instead surfaced through the health row on
/// the next fetch, when `token()` finds nothing and the account reads as unlinked or refused.
fn write_then_refresh<F>(
    shared: &SharedRuntime,
    kind: &'static str,
    write: F,
) -> Result<AppView, String>
where
    F: FnOnce(
        &dyn warframe_market::MarketTransport,
        &warframe_market::MarketToken,
    ) -> Result<warframe_market::MarketToken, warframe_market::MarketError>,
{
    let (pacer, token) = {
        let runtime = shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?;
        (
            runtime.live_prices.pacer(),
            runtime
                .market
                .token()
                .map_err(|error| market_account::failure_message(error).to_owned())?,
        )
    };
    let Some(token) = token else {
        return Err(
            market_account::failure_message(warframe_market::MarketError::Unauthorized).to_owned(),
        );
    };
    let transport = warframe_market::MarketHttp::new(pacer).map_err(|_| {
        market_account::failure_message(warframe_market::MarketError::Unreachable).to_owned()
    })?;
    let renewed = match write(&transport, &token) {
        Ok(renewed) => {
            log::info!("market: order {kind} ok");
            renewed
        }
        Err(error) => {
            log::warn!("market: order {kind} failed: {error}");
            return Err(market_account::failure_message(error).to_owned());
        }
    };
    // Not propagated on failure: the write already happened on the account, and returning early
    // here would leave the list on screen out of date with no way back except pressing the same
    // button again. `publish_account` re-reads the token itself and reports whatever it finds.
    let _ = {
        let mut runtime = shared
            .lock()
            .map_err(|_| "application state is unavailable".to_owned())?;
        // The write changed the account: any fetch already in flight is now reading a state that
        // is about to be superseded, so supersede it before it can re-lock and publish.
        runtime.market.supersede_reads();
        runtime.market.adopt(renewed)
    };
    publish_account(shared)
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
        if !runtime.refresh.begin() {
            return runtime
                .core
                .current_view()
                .map_err(|_| "application view is unavailable".to_owned());
        }
        runtime.app_data.clone()
    };
    let port = ProductionAcquisition { app_data };
    let outcome = port.refresh();
    let result = apply_outcome(shared, outcome);
    if let Ok(mut runtime) = shared.lock() {
        runtime.refresh.finish();
    }
    result
}

struct ProductionAcquisition {
    app_data: PathBuf,
}

fn snapshot_clock(time: SystemTime) -> Result<(u64, SnapshotInstant), StoreError> {
    let observed_at = SnapshotInstant::from_system_time(time)?;
    Ok((observed_at.unix_seconds() as u64, observed_at))
}
impl AcquisitionPort for ProductionAcquisition {
    fn refresh(&self) -> InventoryRefreshOutcome {
        let catalog_http = match WfcdCatalogHttp::new() {
            Ok(client) => client,
            Err(_) => return InventoryRefreshOutcome::catalog_failed(),
        };
        let (now, observed_at) = match snapshot_clock(SystemTime::now()) {
            Ok(clock) => clock,
            Err(_) => return InventoryRefreshOutcome::catalog_failed(),
        };
        let catalog =
            match CatalogCache::new(self.app_data.join("catalog")).load(&catalog_http, now) {
                Ok(catalog) => catalog,
                Err(_) => return InventoryRefreshOutcome::catalog_failed(),
            };
        let procfs = GameMemory::new();
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
                    observed_at,
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
        refresh: RefreshWindow::default(),
        monitor_started: false,
        game_running: false,
        last_ee_log_path: None,
        live_prices,
        market: market_account::MarketSession::new(warframe_market::open_credential_store(
            paths.database.clone(),
        )),
        presence: PresenceHold::default(),
    })))
}

/// Price the collection: cached table first so items are priced before any request is made, then
/// at most one download for the day's dump. Nothing else. No request is made per item, ever,
/// unless the player asks for one.
///
/// There is nothing to schedule. The whole collection is priced by a single file, so this runs
/// once at start and is done -- no queue, no worker, no rate limiting, because there are no
/// per-item requests to pace. A cached table that is already as new as anything published skips
/// the download entirely; the file is 3.9 MB and it changes once a day.
///
/// Relics used to be the exception, swept live at 3 requests a second for about 22 seconds of
/// every launch, because the dump's relic prices read up to 6x high. They are not an exception any
/// more: the fault was the *ask*, and the same file carries completed trades, which are per unit.
/// One file prices only the relics that traded that day, so `PriceTable::adopt` unions it with the
/// files before it and coverage goes from 45% of a real collection's relics to 96%. The sweep's
/// remaining job -- the last few percent -- is not worth 70 requests a launch against a holding
/// that came to 391p.
///
/// The dumps lag, so the usual launch re-downloads the same file it already had. The refreshed
/// table adopts from the table the runtime is *currently* serving, or the download would throw
/// away both the carried relic prices and every price the player spent a request on.
///
/// The download and the fold are deliberately separate steps. `latest_dump` spends seconds on the
/// network, so it runs outside the lock; the fold, the write to disk and the publish then happen
/// under one hold of it. Folding in a table read *before* the download would silently erase any
/// price a page refresh landed while it was in flight -- prices the player has already paid
/// requests for -- and erase them from disk as well as memory. Do not reorder these.
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
        if let Some(table) = cached.as_ref() {
            if let Ok(mut runtime) = shared.lock() {
                let priced = table.len();
                let date = table.dump_date().to_owned();
                runtime.core.set_collection_prices(Arc::new(table.clone()));
                let _ = runtime.core.record_collection_prices_ready(priced, date);
            }
        }
        if !cached
            .as_ref()
            .is_some_and(|table| dump_is_current(table.dump_date(), now))
        {
            let Some(source) = RelicsRunHttp::new() else {
                return;
            };
            // Seconds of network, outside the lock.
            let Ok(mut table) = latest_dump(&source, now) else {
                // A dump that could not be read and a disk that could not be written are different
                // problems with different fixes, and only one of them is warframe.market's.
                if let Ok(mut runtime) = shared.lock() {
                    let _ = runtime.core.record_collection_prices_degraded(
                        "No warframe.market price dump could be read",
                    );
                }
                return;
            };
            let Ok(mut runtime) = shared.lock() else {
                return;
            };
            // The current table, not `cached`: anything checked during the download is in here.
            if let Some(current) = runtime.core.collection_prices() {
                table.adopt(&current);
            }
            let stored = cache.store_table(&table);
            let priced = table.len();
            let date = table.dump_date().to_owned();
            runtime.core.set_collection_prices(Arc::new(table));
            let _ = match stored {
                Ok(()) => runtime.core.record_collection_prices_ready(priced, date),
                Err(_) => runtime.core.record_collection_prices_degraded(
                    "Prices loaded but could not be saved for the next start",
                ),
            };
        }
    });
}

/// What the collection price row says when a checked price reached memory but not disk.
const CHECKED_PRICES_UNSAVED: &str = "Checked prices could not be saved for the next start";

/// Price each name in turn, publishing how far along the pass is and collecting the names nobody
/// is selling.
///
/// Both things this does beyond `MarketPriceCache::warm` need the loop opened up. Progress has to
/// be published *during* the pass -- twenty-two seconds of silence on a figure that moves the whole
/// time is the complaint this answers -- and a `NoSellers` verdict has to be attributed to the name
/// that produced it, which a summed `WarmOutcome` cannot do.
///
/// Each name still goes through `warm`, so every request claims a slot from the same shared clock
/// the reward fill and the pool warm claim from; a one-element slice is paced exactly as a
/// forty-eight-element one. That is why this is a loop around the existing call rather than a second
/// implementation of it beside the rate limiter.
///
/// `Unavailable` is deliberately not collected. An unreachable endpoint is a reason to try again,
/// and recording it as an answer would blacklist a relic until tomorrow's dump over a router that
/// rebooted mid-pass.
fn warm_with_progress(
    shared: &SharedRuntime,
    market: &dyn warframe_acquisition::MarketPriceSource,
    names: &[String],
    live_prices: &MarketPriceCache,
) -> (WarmOutcome, Vec<String>) {
    let mut total = WarmOutcome::default();
    let mut unpriced = Vec::new();
    for (done, name) in names.iter().enumerate() {
        publish_pricing_progress(
            shared,
            Some(PricingProgress {
                done,
                total: names.len(),
            }),
        );
        let one = live_prices.warm(
            market,
            std::slice::from_ref(name),
            warframe_acquisition::MARKET_MIN_GAP,
        );
        if one.no_sellers > 0 {
            unpriced.push(name.clone());
        }
        total.stored += one.stored;
        total.no_sellers += one.no_sellers;
        total.unavailable += one.unavailable;
        total.oversize += one.oversize;
    }
    publish_pricing_progress(shared, None);
    (total, unpriced)
}

fn publish_pricing_progress(shared: &SharedRuntime, pricing: Option<PricingProgress>) {
    if let Ok(mut runtime) = shared.lock() {
        runtime.core.set_pricing_progress(pricing);
    }
}

/// Folds prices just checked against warframe.market into the persisted price table, so they
/// outlive the 15-minute live cache and survive a restart.
///
/// The whole read-modify-write-persist runs under one hold of the runtime lock. The page refresh
/// is the only writer, but two of them overlap readily -- the player clicks, changes page, clicks
/// again -- and either could otherwise clone the table, be overtaken, and then write its stale
/// copy over the other's prices on disk.
/// The network work is deliberately *not* in here: callers pace their own requests first and call
/// this with the answers, so the lock the 2.5-second view poll needs is held for a clone and a
/// file write rather than for twenty seconds of HTTP.
///
/// `unpriced` are the names the market answered about with nothing for sale. They are folded in
/// the same hold and persisted by the same write, because a no-seller answer that only reached
/// memory would make the next refresh re-ask about them after a restart.
///
/// Returns how many items the table can now price, the dump date it belongs to, and whether the
/// write to disk succeeded.
fn store_checked_prices(
    shared: &SharedRuntime,
    cache: &CollectionPriceCache,
    names: &[String],
    live_prices: &MarketPriceCache,
    unpriced: &[String],
) -> Option<(usize, String, bool)> {
    let mut runtime = shared.lock().ok()?;
    let table = runtime.core.collection_prices()?;
    let mut updated = (*table).clone();
    for name in names {
        if let Some(price) = live_prices.get(name) {
            updated.insert_checked(name, price);
        }
    }
    for name in unpriced {
        updated.mark_checked_unpriced(name);
    }
    let stored = cache.store_table(&updated).is_ok();
    let priced = updated.len();
    let date = updated.dump_date().to_owned();
    runtime.core.set_collection_prices(Arc::new(updated));
    Some((priced, date, stored))
}

#[tauri::command]
fn show_reward_overlay(app: AppHandle) {
    // The preview has no screen to measure, so it shows the full-squad strip.
    overlay_window::show_reward_overlay(&app, reward_ocr::MAX_CARDS);
}

#[tauri::command]
fn hide_reward_overlay(app: AppHandle) {
    overlay_window::hide_reward_overlay(&app);
}

/// The kiosk window pulls the latest epoch through this rather than receiving the payload in the
/// event, so a missed `kiosk-updated` (window still loading, webview busy) costs one fetch
/// instead of a stale overlay until the next poll.
#[tauri::command]
fn get_kiosk_view(kiosk: State<'_, KioskState>) -> Option<KioskView> {
    kiosk.get()
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Thread panics otherwise die in whatever terminal spawned the dev server; the log is
    // where a silent overlay gets diagnosed, so a panic anywhere is an error there too.
    {
        let default_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let thread = std::thread::current();
            log::error!(
                "[DEBUG-kiosk] panic in thread {}: {info}",
                thread.name().unwrap_or("<unnamed>")
            );
            default_hook(info);
        }));
    }
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
    // This puts the *main* window on XWayland too, which a compositor doing fractional
    // scaling will render blurry. Split the overlay into its own X11 process if that ever matters
    // more than having one.
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_some() {
        gtk::gdk::set_allowed_backends("x11");
    }
    tauri::Builder::default()
        // Raise the window that is already open rather than starting a rival process. Two instances
        // tail the same EE.log, write the same database and draw two override-redirect overlays at
        // the same coordinates over the game, where whichever raised last wins -- so the strip on
        // screen need not be the build you just started.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        // `reward-overlay` is only ever hidden, never destroyed, so Tauri still holds a live window
        // once the main window closes and the app stays up: tailing the log and drawing the overlay
        // over the game with no UI left to close it by.
        .on_window_event(|window, event| {
            if window.label() == "main"
                && matches!(event, tauri::WindowEvent::CloseRequested { .. })
            {
                window.app_handle().exit(0);
            }
        })
        .setup(|app| {
            // The file target keeps debug traces in dev builds and trims to Info in stable
            // releases: per-OCR-attempt debug lines land every 200 ms, and with only 5 MiB
            // per rotated file a stable session of hours would otherwise keep just the last
            // minutes of history — the very window the report block exists to serve.
            let file = tauri_plugin_log::Target::new(tauri_plugin_log::TargetKind::LogDir {
                file_name: Some("tennoscope.log".to_owned()),
            });
            let mut targets = vec![if cfg!(debug_assertions) {
                file
            } else {
                file.filter(|metadata| metadata.level() <= log::Level::Info)
            }];
            if cfg!(debug_assertions) {
                targets.push(tauri_plugin_log::Target::new(
                    tauri_plugin_log::TargetKind::Stdout,
                ));
            }
            app.handle().plugin(
                tauri_plugin_log::Builder::default()
                    // Debug for our own crates only: `wry`, `zbus` and `rustls` at Debug would
                    // evict the reward diagnostics from the rotation window this exists to hold.
                    //
                    // `app_lib` is the one that matters and the one that was missing: the log
                    // target is the *library* name from `[lib]`, not the package name, so naming
                    // only `tennoscope` filtered out every reward diagnostic there is --
                    // `[DEBUG-capture]`, `[DEBUG-card]` and `[DEBUG-poller]` all log from this
                    // crate's lib. The 2026-08-20 report is what that costs: a wall of identical
                    // `poll failed` warnings and no way to see which monitor was captured or what
                    // the cards actually read. `tennoscope` stays because the binary logs under it.
                    .level(log::LevelFilter::Info)
                    .level_for("app_lib", log::LevelFilter::Debug)
                    .level_for("tennoscope", log::LevelFilter::Debug)
                    .level_for("app_core", log::LevelFilter::Debug)
                    .level_for("warframe_acquisition", log::LevelFilter::Debug)
                    .level_for("warframe_market", log::LevelFilter::Debug)
                    .max_file_size(5 * 1024 * 1024)
                    .rotation_strategy(tauri_plugin_log::RotationStrategy::KeepSome(3))
                    .targets(targets)
                    .build(),
            )?;
            // Before anything can read a reward screen: the NSIS bundle ships Tesseract under the
            // resource directory so a Windows player installs one thing, not two.
            if let Ok(resources) = app.path().resource_dir() {
                reward_ocr::use_bundled_tesseract(&resources);
            }
            let runtime = initialize_runtime(app.handle())?;
            let startup = runtime
                .lock()
                .map(|state| current_setup_status(state.setup.clone(), state.game_running))
                .unwrap_or_default();
            app.manage(runtime);
            // The kiosk window's whole IPC surface: `get_kiosk_view` pulls whatever the poller
            // last published, so the cell exists from the start, empty until a kiosk opens.
            app.manage(kiosk_view::KioskState::default());
            if startup.risk_accepted {
                start_collection_prices(Arc::clone(app.state::<SharedRuntime>().inner()));
                monitor::start(
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
            authorize_screen_capture,
            refresh_inventory,
            refresh_prices,
            load_fake_session,
            show_reward_overlay,
            hide_reward_overlay,
            get_kiosk_view,
            market_status,
            market_sign_in,
            market_link_token,
            market_sign_out,
            refresh_orders,
            set_market_presence,
            collect_report,
            collect_report_text,
            remove_order,
            create_order,
            set_order_quantity,
            update_order
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod live_bench {
    use crate::{kiosk_ocr, reward_capture::GameCapture};
    use image::GenericImageView;
    use std::time::Instant;

    /// Live-machine cost of one whole kiosk pass. Ignored by default: needs the game running.
    #[test]
    #[ignore]
    fn bench_live_kiosk_pass() {
        let candidates: Vec<warframe_acquisition::RewardCatalogEntry> = std::iter::repeat_n(
            warframe_acquisition::RewardCatalogEntry {
                name: "Fulmin Prime Receiver".into(),
                ducats: 100,
            },
            12,
        )
        .collect();

        let t0 = Instant::now();
        let frame = GameCapture::new()
            .capture_candidates()
            .expect("capture")
            .into_iter()
            .next()
            .expect("game capture candidate")
            .image;
        println!("capture: {:?}", t0.elapsed());

        let (w, h) = frame.dimensions();
        println!("frame: {w}x{h}");

        let t1 = Instant::now();
        let dy: i32 = std::env::var("KIOSK_BENCH_DY")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        println!("bench dy: {dy}");
        let cells = kiosk_ocr::read_grid(&frame, &candidates, dy);
        println!(
            "read_grid (24 crops): {:?} -> {} cells",
            t1.elapsed(),
            cells.len()
        );

        let t2 = Instant::now();
        let rows = kiosk_ocr::read_basket(&frame, &candidates);
        for row in &rows {
            println!(
                "  basket row {} = {:?} score {:.2}",
                row.index, row.name, row.score
            );
        }
        println!(
            "read_basket (8 crops): {:?} -> {} rows",
            t2.elapsed(),
            rows.len()
        );
        println!("TOTAL pass: {:?}", t0.elapsed());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use warframe_market::{CredentialBacking, CredentialStore, MarketError, MarketToken};

    #[test]
    fn snapshot_clock_supplies_one_second_to_catalog_and_snapshot_metadata() {
        let time = UNIX_EPOCH + Duration::from_secs(1_785_492_000);

        let (catalog_second, observed_at) = snapshot_clock(time).expect("valid clock");

        assert_eq!(catalog_second, 1_785_492_000);
        assert_eq!(observed_at.unix_seconds(), 1_785_492_000);
    }

    #[test]
    fn snapshot_clock_rejects_a_pre_epoch_host_clock() {
        let time = UNIX_EPOCH.checked_sub(Duration::from_secs(1)).unwrap();

        assert!(snapshot_clock(time).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn main_window_does_not_advertise_a_minimum_height_to_x11() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("valid Tauri config");
        let main = config["app"]["windows"]
            .as_array()
            .expect("window list")
            .iter()
            .find(|window| window["label"] == "main")
            .expect("main window");

        assert!(
            main.get("minHeight").is_none(),
            "an X11 compositor may tile below the hint; WebKitGTK can stay blank until another resize"
        );
    }

    fn setup_input() -> CaptureSetupInput {
        CaptureSetupInput {
            game_running: false,
            x11_game_window: false,
            wlroots_available: false,
            kwin_available: false,
            portal_session_live: false,
        }
    }

    #[test]
    fn direct_capture_paths_do_not_expose_the_portal_action() {
        for input in [
            CaptureSetupInput {
                game_running: true,
                x11_game_window: true,
                ..setup_input()
            },
            CaptureSetupInput {
                game_running: true,
                wlroots_available: true,
                ..setup_input()
            },
            CaptureSetupInput {
                game_running: true,
                kwin_available: true,
                ..setup_input()
            },
        ] {
            assert!(!desktop_capture_action_available(input));
        }
    }

    #[test]
    fn native_wayland_without_a_direct_backend_exposes_the_portal_action() {
        assert!(desktop_capture_action_available(CaptureSetupInput {
            game_running: true,
            ..setup_input()
        }));
    }

    #[test]
    fn no_running_game_does_not_claim_desktop_capture_is_required() {
        assert!(!desktop_capture_action_available(setup_input()));
    }

    #[test]
    fn a_live_portal_session_hides_the_action() {
        let live = CaptureSetupInput {
            game_running: true,
            portal_session_live: true,
            ..setup_input()
        };
        assert!(!desktop_capture_action_available(live));
    }

    /// A credential store that holds one token in memory, so `publish_account` can be exercised
    /// with no keyring and no network.
    #[derive(Default)]
    struct MemoryStore {
        held: StdMutex<Option<String>>,
    }

    impl CredentialStore for MemoryStore {
        fn load(&self) -> Result<Option<MarketToken>, MarketError> {
            Ok(self
                .held
                .lock()
                .expect("lock")
                .clone()
                .map(MarketToken::new))
        }
        fn store(&self, token: &MarketToken) -> Result<(), MarketError> {
            *self.held.lock().expect("lock") = Some(token.expose().to_owned());
            Ok(())
        }
        fn clear(&self) -> Result<(), MarketError> {
            *self.held.lock().expect("lock") = None;
            Ok(())
        }
        fn backing(&self) -> CredentialBacking {
            CredentialBacking::Database
        }
    }

    fn test_runtime(directory: &Path) -> SharedRuntime {
        let core = AppCore::open(&directory.join("test.sqlite3")).expect("core opens");
        Arc::new(Mutex::new(Runtime {
            core,
            app_data: directory.to_path_buf(),
            setup_path: directory.join("setup.json"),
            setup: SetupStatus::default(),
            refresh: RefreshWindow::default(),
            monitor_started: false,
            game_running: false,
            last_ee_log_path: None,
            live_prices: MarketPriceCache::new(),
            market: market_account::MarketSession::new(Box::new(MemoryStore::default())),
            presence: PresenceHold::default(),
        }))
    }

    /// A refresh that is already running, and one that just finished, are both reasons not to
    /// start another -- and the caller must not have to remember that they are two questions.
    #[test]
    fn refresh_window_refuses_while_running_and_during_the_debounce() {
        let mut window = RefreshWindow::default();

        assert!(window.begin(), "the first refresh has nothing to wait for");
        assert!(
            !window.begin(),
            "a second refresh must not overlap the first"
        );

        window.finish();
        assert!(
            !window.begin(),
            "a refresh that just finished still holds the debounce"
        );

        window.started = Some(Instant::now() - Duration::from_secs(16));
        assert!(window.begin(), "past the debounce, refreshing is allowed");
    }

    /// A visual capture failure must not make healthy inventory acquisition or EE.log monitoring
    /// look broken; each health row describes a distinct subsystem.
    #[test]
    fn capture_failure_degrades_only_capture_health() {
        let directory = tempfile::tempdir().expect("temp dir");
        let shared = test_runtime(directory.path());
        let mut runtime = shared.lock().expect("lock");

        runtime
            .core
            .record_game_process_ready()
            .expect("game reader becomes ready");
        runtime
            .core
            .record_log_monitor_ready()
            .expect("EE.log becomes ready");
        let before = runtime.core.current_view().expect("view reads");
        let before_collection_ids = before
            .collection()
            .items()
            .iter()
            .map(|item| item.id().to_owned())
            .collect::<Vec<_>>();
        let before_game_reader = (
            before.health().game_reader().state(),
            before.health().game_reader().message().to_owned(),
            before
                .health()
                .game_reader()
                .last_success()
                .map(str::to_owned),
        );
        let before_log_monitor = (
            before.health().log_monitor().state(),
            before.health().log_monitor().message().to_owned(),
            before
                .health()
                .log_monitor()
                .last_success()
                .map(str::to_owned),
        );
        let before_stage_states = before
            .health()
            .acquisition_stages()
            .iter()
            .map(|stage| stage.state())
            .collect::<Vec<_>>();

        let after = runtime
            .core
            .record_capture_degraded("Screen capture failed: compositor refused the frame")
            .expect("capture failure publishes");

        assert_eq!(
            after.health().capture().state(),
            app_core::HealthState::Degraded
        );
        assert_eq!(
            after
                .collection()
                .items()
                .iter()
                .map(|item| item.id())
                .collect::<Vec<_>>(),
            before_collection_ids
        );
        assert_eq!(
            (
                after.health().game_reader().state(),
                after.health().game_reader().message().to_owned(),
                after
                    .health()
                    .game_reader()
                    .last_success()
                    .map(str::to_owned),
            ),
            before_game_reader
        );
        assert_eq!(
            after
                .health()
                .acquisition_stages()
                .iter()
                .map(|stage| stage.state())
                .collect::<Vec<_>>(),
            before_stage_states
        );
        assert_eq!(
            (
                after.health().log_monitor().state(),
                after.health().log_monitor().message().to_owned(),
                after
                    .health()
                    .log_monitor()
                    .last_success()
                    .map(str::to_owned),
            ),
            before_log_monitor
        );
    }

    /// A fetch that is still in flight when a sign-out happens must not publish over it: a stale
    /// generation is discarded rather than resurrecting a linked view the sign-out just cleared.
    #[test]
    fn a_stale_fetch_does_not_overwrite_a_sign_out() {
        let directory = tempfile::tempdir().expect("temp dir");
        let shared = test_runtime(directory.path());

        // A token is present when the fetch reads its generation, standing in for the moment
        // `publish_account` has already released its first lock and is about to go unlocked for
        // the network.
        shared
            .lock()
            .expect("lock")
            .market
            .adopt(MarketToken::new("fake-token".to_owned()))
            .expect("token stores");
        let generation = shared.lock().expect("lock").market.generation();

        // The sign-out that would race a slow fetch in production: forget the credential and bump
        // the generation, exactly as `market_sign_out` does.
        {
            let mut runtime = shared.lock().expect("lock");
            runtime.market.forget().expect("forget clears");
            runtime.market.supersede_reads();
            runtime
                .core
                .set_market_account(app_core::MarketAccountView::unlinked())
                .expect("unlinked view publishes");
        }

        // The delayed fetch now re-locks with the generation it captured before the sign-out, and
        // must discard rather than publish its (now stale) result.
        let mut runtime = shared.lock().expect("lock");
        let outcome = discard_if_stale(&mut runtime, generation);
        let view = outcome
            .expect("a stale generation is caught")
            .expect("view reads");

        assert_eq!(
            view.market_account().link,
            app_core::LinkState::Unlinked,
            "the sign-out's view must survive a late fetch from before it"
        );
    }
}
