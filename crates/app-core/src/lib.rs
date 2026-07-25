#![forbid(unsafe_code)]

mod fake_session;

use std::path::Path;

use local_store::{SnapshotMeta, SqliteStore, StoreError};
use serde::Serialize;
use thiserror::Error;
use warframe_acquisition::{
    AcquisitionFailure, AcquisitionResult, AcquisitionStage, CatalogLoadSource, StageState,
};
use warframe_domain::{
    Category, DomainError, InventorySnapshot, RewardAdvisor, RewardCandidate, RewardView,
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
        })
    }

    pub fn current_view(&self) -> Result<AppView, AppError> {
        let collection = self.store.load_collection()?;
        let mut items = collection
            .entries()
            .map(CollectionItemView::from)
            .collect::<Vec<_>>();
        items.sort_by(|left, right| left.id.cmp(&right.id));
        let total_entries = items.len();
        let mut health = self.health.clone();
        health.database = BackendHealth::ready("SQLite database available", None)?;
        Ok(AppView {
            collection: CollectionView {
                items,
                total_entries,
            },
            reward: self.reward.clone(),
            health,
        })
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
}

impl CollectionView {
    pub fn items(&self) -> &[CollectionItemView] {
        &self.items
    }

    pub fn total_entries(&self) -> usize {
        self.total_entries
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
}

impl From<&warframe_domain::InventoryEntry> for CollectionItemView {
    fn from(entry: &warframe_domain::InventoryEntry) -> Self {
        Self {
            id: entry.item.id.as_str().to_owned(),
            name: entry.item.name.clone(),
            category: entry.item.category,
            quantity: entry.quantity,
            mastered: entry.mastered,
            image_url: entry
                .item
                .image_name
                .as_ref()
                .map(|name| format!("https://cdn.warframestat.us/img/{name}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthState {
    Ready,
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
    database: BackendHealth,
    acquisition_stages: Vec<AcquisitionStageView>,
}

impl HealthView {
    fn phase_one() -> Result<Self, AppError> {
        Ok(Self {
            game_reader: BackendHealth::degraded("Phase 1 game reader not connected")?,
            log_monitor: BackendHealth::degraded("Waiting for Warframe EE.log")?,
            capture: BackendHealth::degraded("Phase 1 capture not connected")?,
            catalog: BackendHealth::degraded("Phase 1 catalog not connected")?,
            market: BackendHealth::degraded("Phase 1 market not connected")?,
            database: BackendHealth::ready("SQLite database available", None)?,
            acquisition_stages: Vec::new(),
        })
    }

    fn reset_phase_one_integrations(&mut self) -> Result<(), AppError> {
        self.capture = BackendHealth::degraded("Phase 1 capture not connected")?;
        self.catalog = BackendHealth::degraded("Phase 1 catalog not connected")?;
        self.market = BackendHealth::degraded("Phase 1 market not connected")?;
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
