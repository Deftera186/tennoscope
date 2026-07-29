#![forbid(unsafe_code)]

mod fake_session;

use std::path::Path;
use std::sync::Arc;

use local_store::{SnapshotMeta, SqliteStore, StoreError};
use serde::Serialize;
use thiserror::Error;
use warframe_acquisition::{
    AcquisitionFailure, AcquisitionResult, AcquisitionStage, CatalogIndex, CatalogLoadSource,
    MarketPriceCache, PriceTable, StageState,
};
use warframe_domain::{
    CatalogItem, Category, DomainError, InventoryEntry, InventorySnapshot, RewardAdvisor,
    RewardCandidate, RewardView,
};

#[derive(Debug, Error)]
pub enum AppError {
    #[error("local store operation failed: {0}")]
    Store(#[from] StoreError),
    #[error("invalid application fixture data: {0}")]
    Domain(#[from] DomainError),
    #[error("backend health message must not be blank")]
    BlankHealthMessage,
}

pub struct AppCore {
    store: SqliteStore,
    reward: RewardView,
    health: HealthView,
    prices: Option<Arc<PriceTable>>,
    live: Option<MarketPriceCache>,
}

pub trait AcquisitionPort {
    fn refresh(&self) -> InventoryRefreshOutcome;
}

#[derive(Clone)]
pub enum InventoryRefreshOutcome {
    Success {
        result: AcquisitionResult,
        meta: SnapshotMeta,
        catalog_source: CatalogLoadSource,
        catalog_fetched_unix: u64,
    },
    AcquisitionFailed(AcquisitionFailure),
    CatalogFailed,
}

impl InventoryRefreshOutcome {
    pub fn success(
        result: AcquisitionResult,
        meta: SnapshotMeta,
        catalog_source: CatalogLoadSource,
        catalog_fetched_unix: u64,
    ) -> Self {
        Self::Success {
            result,
            meta,
            catalog_source,
            catalog_fetched_unix,
        }
    }
    pub fn acquisition_failed(failure: AcquisitionFailure) -> Self {
        Self::AcquisitionFailed(failure)
    }
    pub const fn catalog_failed() -> Self {
        Self::CatalogFailed
    }
}

impl AppCore {
    pub fn in_memory() -> Result<Self, AppError> {
        Self::from_store(SqliteStore::in_memory()?)
    }

    pub fn open(path: &Path) -> Result<Self, AppError> {
        Self::from_store(SqliteStore::open(path)?)
    }

    fn from_store(store: SqliteStore) -> Result<Self, AppError> {
        Ok(Self {
            store,
            reward: RewardAdvisor::advise(Vec::new()),
            health: HealthView::phase_one()?,
            prices: None,
            live: None,
        })
    }

    /// The daily price table, once it has loaded. Held rather than passed in on every call
    /// because the view is rebuilt every 2.5 seconds and the table changes once a day.
    pub fn set_collection_prices(&mut self, prices: Arc<PriceTable>) {
        self.prices = Some(prices);
    }

    /// The live price cache, shared with the reward overlay. Cheap to clone and entries expire on
    /// their own, so the collection reads whatever the player last asked warframe.market about --
    /// including anything a relic pool warmed during a mission.
    pub fn set_live_prices(&mut self, live: MarketPriceCache) {
        self.live = Some(live);
    }

    /// warframe.market's names for the given collection items, deduplicated.
    ///
    /// Deduplication is not a micro-optimization: a page can hold all four refinements of one
    /// relic, which are one item on warframe.market, and asking four times would spend four
    /// requests to learn the same number.
    pub fn market_names_for(&self, item_ids: &[String]) -> Result<Vec<String>, AppError> {
        let Some(prices) = self.prices.as_ref() else {
            return Ok(Vec::new());
        };
        let collection = self.store.load_collection()?;
        let mut names = collection
            .entries()
            .filter(|entry| item_ids.iter().any(|id| id == entry.item.id.as_str()))
            .filter_map(|entry| prices.market_name(&entry.item.name).map(str::to_owned))
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        Ok(names)
    }

    /// warframe.market's names for the relics the player owns, deduplicated across refinement
    /// tiers.
    ///
    /// Bounded by ownership rather than by the dump: the dump lists every relic the game has (772
    /// measured), while the sweep this feeds spends one request per name at 3/second, and a real
    /// collection owns a few dozen of them -- 65 versus 772 is the difference between 22 seconds
    /// and four minutes.
    pub fn owned_relic_market_names(&self) -> Result<Vec<String>, AppError> {
        let Some(prices) = self.prices.as_ref() else {
            return Ok(Vec::new());
        };
        let relic_names: std::collections::HashSet<String> =
            prices.relic_market_names().into_iter().collect();
        let collection = self.store.load_collection()?;
        let mut names = collection
            .entries()
            .filter(|entry| entry.quantity >= 1)
            .filter_map(|entry| prices.market_name(&entry.item.name).map(str::to_owned))
            .filter(|name| relic_names.contains(name))
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        Ok(names)
    }

    /// The daily price table currently backing the collection view, if it has loaded.
    ///
    /// Returned by value (an `Arc` clone) rather than borrowed, so a caller can read it, drop the
    /// runtime lock, and spend a long time (the relic sweep, ~22 seconds) working from the copy
    /// without holding the lock the 2.5-second view poll also needs.
    pub fn collection_prices(&self) -> Option<Arc<PriceTable>> {
        self.prices.clone()
    }

    /// The health rows as they stand, so a caller can read what a row already says before
    /// overwriting it with something less true.
    pub fn health(&self) -> &HealthView {
        &self.health
    }

    pub fn current_view(&self) -> Result<AppView, AppError> {
        let collection = self.store.load_collection()?;
        let mut items = collection
            .entries()
            .map(|entry| {
                // Mastery is not ownership. An item at quantity 0 is not in the inventory
                // and must not carry a price or contribute to the collection's worth.
                if entry.quantity == 0 {
                    return CollectionItemView::from(entry);
                }
                // Both lookups go through the market's own name for the item: the live cache is
                // keyed by what was asked for, and that is never the catalog's name for a relic.
                let market_name = self
                    .prices
                    .as_ref()
                    .and_then(|prices| prices.market_name(&entry.item.name));
                let live = market_name
                    .zip(self.live.as_ref())
                    .and_then(|(name, cache)| cache.get(name));
                let stored = market_name
                    .zip(self.prices.as_ref())
                    .and_then(|(name, prices)| prices.price_for(name));
                // A price the player checked against warframe.market is persisted into the price
                // table so it outlives that cache's fifteen minutes: a relic has no dump price at
                // all, and for everything else the checked number is the better of the two.
                // Presenting either as a dump price once the cache has dropped it would attribute
                // it to a file it did not come from.
                let checked = market_name
                    .zip(self.prices.as_ref())
                    .is_some_and(|(name, prices)| prices.has_checked_price(name));
                CollectionItemView::priced(entry, live.or(stored), live.is_some() || checked)
            })
            .collect::<Vec<_>>();
        items.sort_by(|left, right| left.id.cmp(&right.id));
        let total_entries = items.len();
        let snapshot = self.store.latest_snapshot_meta()?;
        let mut health = self.health.clone();
        health.database = BackendHealth::ready("SQLite database available", None)?;
        Ok(AppView {
            collection: CollectionView {
                items,
                total_entries,
                snapshot,
            },
            reward: self.reward.clone(),
            health,
        })
    }

    pub fn enrich_collection_from_catalog(
        &mut self,
        catalog: &CatalogIndex,
    ) -> Result<AppView, AppError> {
        let collection = self.store.load_collection()?;
        let entries = collection
            .entries()
            .map(|entry| {
                let Some(metadata) = catalog.resolve(entry.item.id.as_str()) else {
                    return Ok(entry.clone());
                };
                let mut item = CatalogItem::new(
                    entry.item.id.clone(),
                    metadata.name(),
                    metadata.category().unwrap_or(entry.item.category),
                )?;
                if let Some(image_name) = metadata.image_name() {
                    item = item.with_image_name(image_name)?;
                }
                Ok(InventoryEntry::new(item, entry.quantity).with_mastered(entry.mastered))
            })
            .collect::<Result<Vec<_>, DomainError>>()?;
        self.store
            .update_collection_metadata(&InventorySnapshot::coherent(entries)?)?;
        self.current_view()
    }

    pub fn apply_inventory_snapshot(
        &mut self,
        snapshot: InventorySnapshot,
        meta: SnapshotMeta,
    ) -> Result<AppView, AppError> {
        self.store.replace_collection(&snapshot, &meta)?;
        self.reward = RewardAdvisor::advise(Vec::new());
        self.health.game_reader = BackendHealth::inventory_sync(&meta)?;
        if !meta.is_fake() {
            self.health.reset_phase_one_integrations()?;
        }
        self.current_view()
    }

    pub fn apply_reward_candidates(
        &mut self,
        rewards: Vec<RewardCandidate>,
    ) -> Result<AppView, AppError> {
        self.reward = RewardAdvisor::advise(rewards);
        self.current_view()
    }

    pub fn load_fake_session(&mut self) -> Result<AppView, AppError> {
        let session = fake_session::build()?;
        self.apply_inventory_snapshot(session.snapshot, session.meta)?;
        self.health.capture = BackendHealth::degraded("Fake session; capture not connected")?;
        self.health.catalog = BackendHealth::degraded("Fake session; live catalog not connected")?;
        self.health.market = BackendHealth::degraded("Fake session; live market not connected")?;
        self.apply_reward_candidates(session.rewards)
    }

    pub fn finish_inventory_refresh(
        &mut self,
        attempt: Result<(AcquisitionResult, SnapshotMeta), AcquisitionFailure>,
        catalog: Option<(CatalogLoadSource, u64)>,
    ) -> Result<AppView, AppError> {
        match attempt {
            Ok((result, meta)) => {
                self.store.replace_collection(result.snapshot(), &meta)?;
                self.reward = RewardAdvisor::advise(Vec::new());
                self.health.game_reader = BackendHealth::inventory_sync(&meta)?;
                self.health.acquisition_stages = result
                    .health()
                    .stages()
                    .iter()
                    .copied()
                    .map(AcquisitionStageView::from)
                    .collect();
                self.health.catalog = match catalog {
                    Some((CatalogLoadSource::Network, fetched)) => BackendHealth::ready(
                        "Current WFCD catalog loaded",
                        Some(fetched.to_string()),
                    )?,
                    Some((CatalogLoadSource::StaleCache, fetched)) => BackendHealth::new(
                        HealthState::Degraded,
                        "Using cached WFCD catalog",
                        Some(fetched.to_string()),
                    )?,
                    None => BackendHealth::degraded("Catalog status unavailable")?,
                };
            }
            Err(failure) => {
                let last_success = self.health.game_reader.last_success.clone();
                self.health.acquisition_stages = failure
                    .health()
                    .stages()
                    .iter()
                    .copied()
                    .map(AcquisitionStageView::from)
                    .collect();
                self.health.game_reader =
                    match failure.health().stages().first().map(|stage| stage.state()) {
                        Some(StageState::Degraded) => BackendHealth::new(
                            HealthState::Degraded,
                            failure.to_string(),
                            last_success,
                        )?,
                        _ => BackendHealth::new(
                            HealthState::Failed,
                            failure.to_string(),
                            last_success,
                        )?,
                    };
            }
        }
        self.current_view()
    }

    pub fn refresh_from(&mut self, port: &dyn AcquisitionPort) -> Result<AppView, AppError> {
        match port.refresh() {
            InventoryRefreshOutcome::Success {
                result,
                meta,
                catalog_source,
                catalog_fetched_unix,
            } => self.finish_inventory_refresh(
                Ok((result, meta)),
                Some((catalog_source, catalog_fetched_unix)),
            ),
            InventoryRefreshOutcome::AcquisitionFailed(failure) => {
                self.finish_inventory_refresh(Err(failure), None)
            }
            InventoryRefreshOutcome::CatalogFailed => {
                self.record_catalog_failure("No valid WFCD catalog is available")
            }
        }
    }

    pub fn record_catalog_failure(
        &mut self,
        message: impl Into<String>,
    ) -> Result<AppView, AppError> {
        self.health.catalog = BackendHealth::failed(message)?;
        self.current_view()
    }

    pub fn record_log_monitor_failure(
        &mut self,
        message: impl Into<String>,
    ) -> Result<AppView, AppError> {
        self.health.log_monitor = BackendHealth::failed(message)?;
        self.current_view()
    }

    pub fn record_log_monitor_degraded(
        &mut self,
        message: impl Into<String>,
    ) -> Result<AppView, AppError> {
        self.health.log_monitor = BackendHealth::degraded(message)?;
        self.current_view()
    }

    pub fn record_log_monitor_ready(&mut self) -> Result<AppView, AppError> {
        self.health.log_monitor = BackendHealth::ready("EE.log monitor ready", None)?;
        self.current_view()
    }

    pub fn record_game_process_ready(&mut self) -> Result<AppView, AppError> {
        let last_success = self.health.game_reader.last_success.clone();
        self.health.game_reader = BackendHealth::new(
            HealthState::Ready,
            "Warframe process connected",
            last_success,
        )?;
        self.current_view()
    }

    pub fn record_capture_ready(
        &mut self,
        observed_at: impl Into<String>,
    ) -> Result<AppView, AppError> {
        self.health.capture =
            BackendHealth::ready("Reward screen observer ready", Some(observed_at.into()))?;
        self.current_view()
    }

    pub fn record_capture_source_ready(
        &mut self,
        source: &str,
        elapsed_ms: u128,
        observed_at: impl Into<String>,
    ) -> Result<AppView, AppError> {
        let source = if source.eq_ignore_ascii_case("memory") {
            "Memory"
        } else {
            "OCR"
        };
        self.health.capture = BackendHealth::ready(
            format!("{source} reward observer ready ({elapsed_ms} ms)"),
            Some(observed_at.into()),
        )?;
        self.current_view()
    }

    /// Live prices arrived for at least one card. Until this fires the overlay shows an em dash
    /// rather than a zero, so the panel never implies an item is worthless when it is just unpriced.
    pub fn record_market_ready(
        &mut self,
        priced: usize,
        observed_at: impl Into<String>,
    ) -> Result<AppView, AppError> {
        self.health.market = BackendHealth::ready(
            format!("warframe.market pricing ready ({priced} priced)"),
            Some(observed_at.into()),
        )?;
        self.current_view()
    }

    pub fn record_market_degraded(
        &mut self,
        message: impl Into<String>,
    ) -> Result<AppView, AppError> {
        let last_success = self.health.market.last_success.clone();
        self.health.market = BackendHealth::new(HealthState::Degraded, message, last_success)?;
        self.current_view()
    }

    /// The daily dump loaded, and the collection is priced from it.
    ///
    /// Kept apart from `record_market_ready` because the two say different things: that one
    /// reports whether the overlay reached warframe.market seconds ago, this one reports which
    /// day's dump the collection's numbers are from. Written to one row, whichever ran last
    /// erased the other, and a reader looking for the dump's date found a unix timestamp.
    pub fn record_collection_prices_ready(
        &mut self,
        priced: usize,
        dump_date: impl Into<String>,
    ) -> Result<AppView, AppError> {
        let dump_date = dump_date.into();
        self.health.collection_prices = BackendHealth::ready(
            format!("Priced from the {dump_date} price dump ({priced} items)"),
            Some(dump_date),
        )?;
        self.current_view()
    }

    pub fn record_collection_prices_degraded(
        &mut self,
        message: impl Into<String>,
    ) -> Result<AppView, AppError> {
        let last_success = self.health.collection_prices.last_success.clone();
        self.health.collection_prices =
            BackendHealth::new(HealthState::Degraded, message, last_success)?;
        self.current_view()
    }

    /// The startup relic sweep has begun, on the same row as `record_collection_prices_ready`.
    ///
    /// A ~22-second sweep with nothing written to this row until it finishes reads as work that
    /// never started, not work that is running -- Diagnostics is the only place that 22 seconds is
    /// visible at all, since nothing else in the UI waits on it. It says how many relics it set out
    /// to check and no more: the sweep skips names the live cache already holds without counting
    /// them, so anything the pass reports back is not a fraction of this number.
    pub fn record_collection_prices_sweeping(
        &mut self,
        relics: usize,
    ) -> Result<AppView, AppError> {
        let last_success = self.health.collection_prices.last_success.clone();
        self.health.collection_prices = BackendHealth::ready(
            format!("Checking live prices for {relics} owned relics"),
            last_success,
        )?;
        self.current_view()
    }

    pub fn record_capture_degraded(
        &mut self,
        message: impl Into<String>,
    ) -> Result<AppView, AppError> {
        let last_success = self.health.capture.last_success.clone();
        self.health.capture = BackendHealth::new(HealthState::Degraded, message, last_success)?;
        self.current_view()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct AppView {
    collection: CollectionView,
    reward: RewardView,
    health: HealthView,
}

impl AppView {
    pub fn collection(&self) -> &CollectionView {
        &self.collection
    }

    pub fn reward(&self) -> &RewardView {
        &self.reward
    }

    pub fn health(&self) -> &HealthView {
        &self.health
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CollectionView {
    items: Vec<CollectionItemView>,
    total_entries: usize,
    snapshot: Option<SnapshotMeta>,
}

impl CollectionView {
    pub fn items(&self) -> &[CollectionItemView] {
        &self.items
    }

    pub fn total_entries(&self) -> usize {
        self.total_entries
    }

    pub fn snapshot(&self) -> Option<&SnapshotMeta> {
        self.snapshot.as_ref()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CollectionItemView {
    id: String,
    name: String,
    category: Category,
    quantity: u32,
    mastered: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    image_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    platinum: Option<u32>,
    live: bool,
}

impl CollectionItemView {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn category(&self) -> Category {
        self.category
    }

    pub fn quantity(&self) -> u32 {
        self.quantity
    }

    pub fn mastered(&self) -> bool {
        self.mastered
    }

    pub fn image_url(&self) -> Option<&str> {
        self.image_url.as_deref()
    }

    pub fn platinum(&self) -> Option<u32> {
        self.platinum
    }

    /// Whether this price came from warframe.market just now, rather than from the daily dump.
    pub fn live(&self) -> bool {
        self.live
    }

    fn priced(entry: &warframe_domain::InventoryEntry, platinum: Option<u32>, live: bool) -> Self {
        Self {
            platinum,
            live,
            ..Self::from(entry)
        }
    }
}

impl From<&warframe_domain::InventoryEntry> for CollectionItemView {
    fn from(entry: &warframe_domain::InventoryEntry) -> Self {
        Self {
            id: entry.item.id.as_str().to_owned(),
            name: entry.item.name.clone(),
            category: entry.item.category,
            quantity: entry.quantity,
            mastered: entry.mastered,
            image_url: entry.item.image_name.as_ref().map(|name| {
                format!(
                    "https://raw.githubusercontent.com/WFCD/warframe-items/master/data/img/{name}"
                )
            }),
            platinum: None,
            live: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Ready,
    /// Enabled, unimpaired, and with nothing to do yet.
    ///
    /// Distinct from `Ready` on purpose. The reward observer shells out to `xwininfo`,
    /// `magick` and `tesseract`, and none of them are probed until a reward screen
    /// actually appears -- so before the first read there is nothing to justify a green
    /// state, and claiming one would be a guess. Distinct from `Degraded` because
    /// nothing is wrong: reporting "waiting for work" as a fault trains the reader to
    /// ignore the colour that means a real fault.
    Idle,
    Degraded,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
pub struct BackendHealth {
    state: HealthState,
    message: String,
    last_success: Option<String>,
}

impl BackendHealth {
    pub fn new(
        state: HealthState,
        message: impl Into<String>,
        last_success: Option<String>,
    ) -> Result<Self, AppError> {
        let message = message.into();
        if message.trim().is_empty() {
            return Err(AppError::BlankHealthMessage);
        }
        Ok(Self {
            state,
            message,
            last_success,
        })
    }

    pub fn ready(
        message: impl Into<String>,
        last_success: Option<String>,
    ) -> Result<Self, AppError> {
        Self::new(HealthState::Ready, message, last_success)
    }

    pub fn idle(
        message: impl Into<String>,
        last_success: Option<String>,
    ) -> Result<Self, AppError> {
        Self::new(HealthState::Idle, message, last_success)
    }

    pub fn degraded(message: impl Into<String>) -> Result<Self, AppError> {
        Self::new(HealthState::Degraded, message, None)
    }

    pub fn failed(message: impl Into<String>) -> Result<Self, AppError> {
        Self::new(HealthState::Failed, message, None)
    }

    fn inventory_sync(meta: &SnapshotMeta) -> Result<Self, AppError> {
        let message = if meta.is_fake() {
            "Deterministic fake inventory loaded".to_owned()
        } else {
            format!("Inventory synchronized from {}", meta.source())
        };
        Self::ready(message, Some(meta.observed_at().to_owned()))
    }

    pub fn state(&self) -> HealthState {
        self.state
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn last_success(&self) -> Option<&str> {
        self.last_success.as_deref()
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct HealthView {
    game_reader: BackendHealth,
    log_monitor: BackendHealth,
    capture: BackendHealth,
    catalog: BackendHealth,
    market: BackendHealth,
    /// The daily dump behind the collection's valuation. Its own row because it answers a
    /// different question from `market`: that one says whether the overlay could reach
    /// warframe.market just now, this one says how old the prices the collection is showing are.
    /// Sharing one row meant whichever wrote last erased the other's answer.
    collection_prices: BackendHealth,
    database: BackendHealth,
    acquisition_stages: Vec<AcquisitionStageView>,
}

impl HealthView {
    fn phase_one() -> Result<Self, AppError> {
        Ok(Self {
            game_reader: BackendHealth::degraded("Waiting for a logged-in Warframe process")?,
            log_monitor: BackendHealth::degraded("Waiting for Warframe EE.log")?,
            capture: BackendHealth::idle("OCR reward observer idle; no reward screen yet", None)?,
            catalog: BackendHealth::degraded("Item catalog has not loaded yet")?,
            market: BackendHealth::idle(
                "warframe.market pricing idle; nothing to price yet",
                None,
            )?,
            collection_prices: BackendHealth::idle(
                "Collection price dump has not loaded yet",
                None,
            )?,
            database: BackendHealth::ready("SQLite database available", None)?,
            acquisition_stages: Vec::new(),
        })
    }

    fn reset_phase_one_integrations(&mut self) -> Result<(), AppError> {
        self.capture = BackendHealth::idle(
            "OCR reward observer idle; no reward screen yet",
            self.capture.last_success.clone(),
        )?;
        self.catalog = BackendHealth::degraded("Item catalog has not loaded yet")?;
        self.market = BackendHealth::idle(
            "warframe.market pricing idle; nothing to price yet",
            self.market.last_success.clone(),
        )?;
        Ok(())
    }

    pub fn game_reader(&self) -> &BackendHealth {
        &self.game_reader
    }

    pub fn log_monitor(&self) -> &BackendHealth {
        &self.log_monitor
    }

    pub fn capture(&self) -> &BackendHealth {
        &self.capture
    }

    pub fn catalog(&self) -> &BackendHealth {
        &self.catalog
    }

    pub fn market(&self) -> &BackendHealth {
        &self.market
    }

    pub fn collection_prices(&self) -> &BackendHealth {
        &self.collection_prices
    }

    pub fn database(&self) -> &BackendHealth {
        &self.database
    }

    pub fn acquisition_stages(&self) -> &[AcquisitionStageView] {
        &self.acquisition_stages
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct AcquisitionStageView {
    stage: &'static str,
    state: HealthState,
    message: String,
}

impl AcquisitionStageView {
    pub fn state(&self) -> HealthState {
        self.state
    }
}

impl From<warframe_acquisition::StageHealth> for AcquisitionStageView {
    fn from(value: warframe_acquisition::StageHealth) -> Self {
        let stage = match value.stage() {
            AcquisitionStage::GameDiscovery => "game_discovery",
            AcquisitionStage::MemoryPermission => "memory_permission",
            AcquisitionStage::AuthorizationDiscovery => "authorization_discovery",
            AcquisitionStage::EndpointFetch => "endpoint_fetch",
            AcquisitionStage::SchemaValidation => "schema_validation",
        };
        let state = match value.state() {
            StageState::Ready => HealthState::Ready,
            StageState::Degraded => HealthState::Degraded,
            StageState::Failed => HealthState::Failed,
        };
        Self {
            stage,
            state,
            message: value.diagnostic().to_string(),
        }
    }
}
