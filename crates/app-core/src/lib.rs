#![forbid(unsafe_code)]

mod fake_session;

use std::path::Path;

use local_store::{SnapshotMeta, SqliteStore, StoreError};
use serde::Serialize;
use thiserror::Error;
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
        self.store
            .replace_collection(&session.snapshot, &session.meta)?;
        self.reward = RewardAdvisor::advise(session.rewards);
        self.health = HealthView::fake_session()?;
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
}

impl From<&warframe_domain::InventoryEntry> for CollectionItemView {
    fn from(entry: &warframe_domain::InventoryEntry) -> Self {
        Self {
            id: entry.item.id.as_str().to_owned(),
            name: entry.item.name.clone(),
            category: entry.item.category,
            quantity: entry.quantity,
            mastered: entry.mastered,
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
    #[serde(skip_serializing_if = "Option::is_none")]
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
    capture: BackendHealth,
    catalog: BackendHealth,
    market: BackendHealth,
    database: BackendHealth,
}

impl HealthView {
    fn phase_one() -> Result<Self, AppError> {
        Ok(Self {
            game_reader: BackendHealth::degraded("Phase 1 game reader not connected")?,
            capture: BackendHealth::degraded("Phase 1 capture not connected")?,
            catalog: BackendHealth::degraded("Phase 1 catalog not connected")?,
            market: BackendHealth::degraded("Phase 1 market not connected")?,
            database: BackendHealth::ready("SQLite database available", None)?,
        })
    }

    fn fake_session() -> Result<Self, AppError> {
        Ok(Self {
            game_reader: BackendHealth::ready(
                "Deterministic fake inventory loaded",
                Some("2000-01-01T00:00:00Z".to_owned()),
            )?,
            capture: BackendHealth::degraded("Fake session; capture not connected")?,
            catalog: BackendHealth::degraded("Fake session; live catalog not connected")?,
            market: BackendHealth::degraded("Fake session; live market not connected")?,
            database: BackendHealth::ready("SQLite database available", None)?,
        })
    }

    pub fn game_reader(&self) -> &BackendHealth {
        &self.game_reader
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
}
