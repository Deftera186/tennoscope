#![forbid(unsafe_code)]

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

use app_core::{AcquisitionPort, AppCore, AppView, InventoryRefreshOutcome, PricingProgress};
use local_store::SnapshotMeta;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager, State};
use warframe_acquisition::{
    CatalogCache, CatalogIndex, CollectionPriceCache, GameProcess, InventoryAcquirer,
    InventoryHttpTransport, MarketPriceCache, MemoryReader, ProcessDiscovery, RelicCatalogCache,
    RelicRewardIndex, RelicsRunHttp, RewardCatalogEntry, RewardMemoryScanner, WarmOutcome,
    WfcdCatalogHttp, WfcdRelicCatalogHttp, dump_is_current, latest_dump,
};
use warframe_domain::RewardCandidate;

/// The platform's process-memory backend. Both sides implement `MemoryReader` and
/// `ProcessDiscovery`, which is the seam everything below the app already works through -- naming
/// the concrete type once here is what keeps `cfg` out of the call sites.
#[cfg(unix)]
use warframe_acquisition::LinuxProc as GameMemory;
#[cfg(windows)]
use warframe_acquisition::WindowsProc as GameMemory;

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

pub mod market_account;
mod monitor;
mod overlay_window;
pub mod report;
mod reward_log;
mod reward_observer;
mod reward_ocr;
mod reward_source;
pub use monitor::{
    LogMonitorDiagnostic, LogObservation, MonitorInput, MonitorMachine, MonitorResult,
};
pub use overlay_window::{OverlayGeometry, WindowRect, borderless_notice, reward_overlay_geometry};
pub use reward_log::{RewardLogEvent, RewardLogMachine};
pub use reward_observer::{
    RewardObservation, RewardObserverState, match_reward_text, normalize_ocr,
};
pub use reward_ocr::{
    MAX_CARDS, ScreenRewardSource, TESSERACT_EXECUTABLE, best_match, card_block_left,
    card_block_width, largest_warframe_window, luma, normalize_contrast, ocr_crop, prepare_crop,
    read_cards, read_cards_in, tesseract_program, threshold_inverted,
    warframe_window_from_xwininfo_tree,
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
    overlay_preview_until: Option<Instant>,
    monitor_started: bool,
    /// Last-known EE.log path, cached so reports can include it even after the game exits.
    last_ee_log_path: Option<PathBuf>,
    // Survives across missions on purpose: the same relic pools recur all evening, so a price
    // fetched two runs ago is one this run does not have to make. Shared with the collection, so
    // a pool warmed mid-mission also prices those items in the browser.
    live_prices: MarketPriceCache,
    market: market_account::MarketSession,
    // Bumped on every write to the linked account (forget, or a successful order write) so an
    // in-flight `publish_account` fetch that started before the change can recognize its result is
    // stale and drop it instead of republishing over a newer state.
    market_generation: u64,
    /// The presence socket, open only while a status is being held. Dropping it is how this
    /// application goes offline: warframe.market has no settable `offline`, and a client that
    /// stays connected claiming `invisible` is still a client the server counts as connected.
    presence: Option<warframe_status::StatusLink>,
    /// Whether presence follows the game reader rather than a choice the player made.
    presence_auto: bool,
    /// What the socket was last asked to hold. Kept beside the link rather than read back off it:
    /// the link reports only what the server has confirmed, and that is `None` for the first
    /// moment of every connection.
    presence_wanted: Option<warframe_status::Presence>,
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
            .and_then(|process| inventory_log_path(process.pid()))
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
        runtime.presence_auto = auto;
        let wanted = if auto {
            Some(auto_presence(&runtime))
        } else {
            status
        };
        match wanted {
            None => runtime.presence = None,
            Some(status) => match &runtime.presence {
                Some(link) => link.set(status),
                None => {
                    let token = runtime
                        .market
                        .token()
                        .map_err(|error| market_account::failure_message(error).to_owned())?
                        .ok_or_else(|| "No warframe.market account is linked".to_owned())?;
                    runtime.presence = Some(warframe_status::StatusLink::connect(
                        token.expose().to_owned(),
                        status,
                    ));
                }
            },
        }
        runtime.presence_wanted = wanted;
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
    if runtime.presence_auto {
        let wanted = auto_presence(runtime);
        if runtime.presence_wanted != Some(wanted) {
            runtime.presence_wanted = Some(wanted);
            if let Some(link) = &runtime.presence {
                link.set(wanted);
            }
        }
    }
    let presence = app_core::PresenceView {
        status: runtime.presence.as_ref().and_then(|link| link.committed()),
        wanted: runtime.presence_wanted,
        auto: runtime.presence_auto,
    };
    runtime
        .core
        .set_presence(presence)
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
/// If a write or a sign-out happened while this fetch was unlocked, `generation` is now stale:
/// whatever the fetch found is older than what is already published, and must be dropped rather
/// than clobbering the newer state -- for a sign-out in particular, rather than resurrecting
/// `items` that `forget` just cleared. Returns the current view in that case, unchanged.
fn discard_if_stale(runtime: &mut Runtime, generation: u64) -> Option<Result<AppView, String>> {
    if runtime.market_generation == generation {
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
            runtime.market_generation,
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
            runtime.market_generation = runtime.market_generation.wrapping_add(1);
            // The socket authenticated with the credential just discarded. Holding it open would
            // keep announcing an account the player has unlinked.
            runtime.presence = None;
            runtime.presence_auto = false;
            runtime.presence_wanted = None;
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
        // is about to be superseded, so bump the generation before it can re-lock and publish.
        runtime.market_generation = runtime.market_generation.wrapping_add(1);
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
        overlay_preview_until: None,
        monitor_started: false,
        last_ee_log_path: None,
        live_prices,
        market: market_account::MarketSession::new(warframe_market::open_credential_store(
            paths.database.clone(),
        )),
        market_generation: 0,
        presence: None,
        presence_auto: false,
        presence_wanted: None,
    })))
}

#[cfg(unix)]
fn inventory_log_path(pid: u32) -> Option<PathBuf> {
    inventory_log_path_at(Path::new("/proc"), pid)
}

/// On Windows the game writes to its own `%LOCALAPPDATA%`, so there is no prefix to discover and
/// the PID is not needed -- but the signature is shared with the Wine path, which does need it.
#[cfg(windows)]
fn inventory_log_path(_pid: u32) -> Option<PathBuf> {
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

fn monitor_game(shared: SharedRuntime, app: AppHandle) {
    let procfs = GameMemory::new();
    let mut machine = MonitorMachine::new(15);
    let mut reward_state = RewardObserverState::new(1, 1);
    let mut reward_log = RewardLogMachine::default();
    let mut announced_process = None;
    let mut tracked_resolution: Option<(u32, Option<PathBuf>)> = None;
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
    let visual_pool: SharedRelicPool = Arc::new(Mutex::new(RelicPool::default()));
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
                    }
                }
                build_monitor_input(&machine, now, process.pid(), path)
            }
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
            // The poller's read is the one with nothing checking it. The log-driven path verifies
            // its cards against the reward EE.log states outright; this one publishes on the
            // closed-set match alone, so the set it matched against is the only evidence there is.
            if let Ok(pool) = visual_pool.lock() {
                pool.trace_published(&names);
            }
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
    procfs: &GameMemory,
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
            if let Ok(mut pool) = visual_pool.lock() {
                pool.adopt(&relic_paths, entries.clone());
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
        log::debug!("[DEBUG-poller] arm declined: empty pool");
        return None;
    }
    let already_running = visual_polling.swap(true, Ordering::AcqRel);
    log::debug!("[DEBUG-poller] arm pool={pool_size} already_running={already_running}");
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
                log::warn!("[DEBUG-poller] poll failed: {reason}");
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
        let procfs = GameMemory::new();
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
        // Read the cards but could not find the window to draw over: on Windows that is exclusive
        // fullscreen, and the player is the only one who can fix it. Said here rather than in the
        // README because a strip that silently fails to appear reads as a broken app.
        if let Some(notice) = overlay_window::overlay_placement_notice() {
            let _ = runtime.core.record_capture_degraded(notice);
        }
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

fn build_monitor_input(
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
            hide_reward_overlay,
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
mod tests {
    use super::*;
    use std::sync::Mutex as StdMutex;
    use warframe_market::{CredentialBacking, CredentialStore, MarketError, MarketToken};

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
            last_refresh_started: None,
            refresh_in_flight: false,
            overlay_preview_until: None,
            monitor_started: false,
            last_ee_log_path: None,
            live_prices: MarketPriceCache::new(),
            market: market_account::MarketSession::new(Box::new(MemoryStore::default())),
            market_generation: 0,
            presence: None,
            presence_auto: false,
            presence_wanted: None,
        }))
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
        let generation = shared.lock().expect("lock").market_generation;

        // The sign-out that would race a slow fetch in production: forget the credential and bump
        // the generation, exactly as `market_sign_out` does.
        {
            let mut runtime = shared.lock().expect("lock");
            runtime.market.forget().expect("forget clears");
            runtime.market_generation = runtime.market_generation.wrapping_add(1);
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
