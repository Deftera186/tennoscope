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
    CatalogItem, Category, Collection, DomainError, InventoryEntry, InventorySnapshot,
    RewardAdvisor, RewardCandidate, RewardView,
};
use warframe_market::{CredentialBacking, MarketItems, MarketOrder, OrderKind};
use warframe_status::Presence;

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
    pricing: Option<PricingProgress>,
    market_account: MarketAccountView,
    presence: PresenceView,
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
            pricing: None,
            market_account: MarketAccountView::unlinked(),
            presence: PresenceView::default(),
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

    /// How far through a live pricing pass we are, or `None` when none is running.
    ///
    /// Published rather than inferred because the page refresh is the only thing that knows its
    /// own total, and the collection's worth figure moves the whole time it is running. A reader
    /// watching a number climb with nothing to explain it has been given a moving target, not a
    /// valuation.
    pub fn set_pricing_progress(&mut self, pricing: Option<PricingProgress>) {
        self.pricing = pricing;
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
            // A ranked row will not show what comes back -- the market answers about rank 0 -- so
            // asking on its behalf spends a request to learn a number that is then discarded. The
            // unranked row of the same name, where one is owned, still asks for it.
            .filter(|entry| entry.rank.unwrap_or(0) == 0)
            .filter_map(|entry| prices.market_name(&entry.item.name))
            .collect::<Vec<_>>();
        names.sort();
        names.dedup();
        Ok(names)
    }

    /// The daily price table currently backing the collection view, if it has loaded.
    ///
    /// Returned by value (an `Arc` clone) rather than borrowed, so a caller can read it, drop the
    /// runtime lock, and spend a long time (a page's live pricing, ~16 seconds) working from the
    /// copy without holding the lock the 2.5-second view poll also needs.
    pub fn collection_prices(&self) -> Option<Arc<PriceTable>> {
        self.prices.clone()
    }

    /// The health rows as they stand, so a caller can read what a row already says before
    /// overwriting it with something less true.
    pub fn health(&self) -> &HealthView {
        &self.health
    }

    pub fn market_account(&self) -> &MarketAccountView {
        &self.market_account
    }

    /// Replace the presence the screen shows. Set from the socket's committed value, never from
    /// what the switch was moved to: the server is the authority on what other players see.
    pub fn set_presence(&mut self, presence: PresenceView) -> Result<AppView, AppError> {
        self.presence = presence;
        self.current_view()
    }

    /// Replace the account state and say so in the health row.
    pub fn set_market_account(&mut self, account: MarketAccountView) -> Result<AppView, AppError> {
        self.health.market_account = match account.link {
            // Nothing carried forward: with no account linked there is no fetch for a timestamp
            // to describe, and one left behind reads as though a link were still in place.
            LinkState::Unlinked => BackendHealth::idle("No warframe.market account linked", None)?,
            LinkState::NeedsRelink => {
                BackendHealth::degraded("The warframe.market credential was refused")?
            }
            LinkState::Linked => {
                let backing = match account.backing {
                    Some(CredentialBacking::Keyring) => "the OS keyring",
                    _ => "the local database",
                };
                BackendHealth::ready(
                    format!("warframe.market account linked; credential held in {backing}"),
                    account.fetched_at.clone(),
                )?
            }
        };
        self.market_account = account;
        self.current_view()
    }

    /// A fetch failed. The orders already held stay: the list is still true as of when it was
    /// fetched and its age is on the screen, and replacing a slightly old answer with none is not
    /// an improvement.
    pub fn record_market_account_failure(
        &mut self,
        message: impl Into<String>,
    ) -> Result<AppView, AppError> {
        let message = message.into();
        log::warn!("health: market account degraded — {message}");
        let last_success = self.health.market_account.last_success.clone();
        self.health.market_account =
            BackendHealth::new(HealthState::Degraded, message, last_success)?;
        self.current_view()
    }

    /// The stored collection, for a caller that has to join something against it.
    pub fn collection_for_reconciliation(&self) -> Result<Collection, AppError> {
        Ok(self.store.load_collection()?)
    }

    pub fn latest_snapshot_meta(&self) -> Result<Option<SnapshotMeta>, AppError> {
        Ok(self.store.latest_snapshot_meta()?)
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
                // A live check answers about the listing's default rank, which is rank 0: the
                // orders endpoint returns the cheapest sellers and those are unranked copies.
                // Measured 2026-07-30, `arcane_reaper` answers 10p unranked against 350p at rank 5.
                // The cache and the checked-price map are keyed by listing name alone, so both
                // numbers would otherwise land on every rank row of that name and quote a maxed
                // arcane at what an unranked one goes for.
                let unranked = entry.rank.unwrap_or(0) == 0;
                let live = market_name
                    .as_deref()
                    .filter(|_| unranked)
                    .zip(self.live.as_ref())
                    .and_then(|(name, cache)| cache.get(name));
                let stored =
                    market_name
                        .as_deref()
                        .zip(self.prices.as_ref())
                        .map(|(name, prices)| {
                            prices.ranked_price_for(name, entry.rank, entry.at_max_rank())
                        });
                // A price the player checked against warframe.market is persisted into the price
                // table so it outlives that cache's fifteen minutes: most relics have no dump price
                // at all, and everywhere else the checked number is the better of the two.
                // Presenting either as a dump price once the cache has dropped it would attribute
                // it to a file it did not come from.
                let checked = market_name
                    .as_deref()
                    .filter(|_| unranked)
                    .zip(self.prices.as_ref())
                    .is_some_and(|(name, prices)| prices.has_checked_price(name));
                // Resolving to a market name is the whole test for whether this item can be
                // priced at all, and it is a different fact from having a price: an unswept relic
                // resolves and shows a dash. The page control counts these, because counting
                // everything owned promised prices for items the backend drops before it makes a
                // single request.
                CollectionItemView::priced(
                    entry,
                    live.or(stored.and_then(|price| price.platinum)),
                    live.is_some() || checked,
                    market_name.is_some(),
                )
                // A live check answers for the listing, which is the unranked one, so it does not
                // settle a part-ranked copy. The bound stands until the copy is finished.
                .with_price_ceiling(stored.and_then(|price| price.ceiling))
                // Read from the same table whether or not this copy has a price, and never from the
                // live cache: the orders endpoint reports who is selling, not who bought.
                .with_monthly_trades(
                    market_name
                        .as_deref()
                        .zip(self.prices.as_ref())
                        .and_then(|(name, prices)| prices.monthly_trades(name)),
                )
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
                pricing: self.pricing,
            },
            reward: self.reward.clone(),
            health,
            market_account: MarketAccountView {
                presence: self.presence,
                ..self.market_account.clone()
            },
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
                // By path, not by id: a ranked row's id carries its rank, and the catalogue has
                // never heard of it. Resolving on the raw id left every ranked mod holding the
                // decoder's fallback label and no artwork.
                let Some(metadata) = catalog.resolve(entry.item.id.catalog_path()) else {
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
                let enriched =
                    InventoryEntry::new(item, entry.quantity).with_mastered(entry.mastered);
                // Enrichment is about the card, not the copies. Dropping the rank here put both
                // rows of a mod back on one price.
                Ok(match entry.rank {
                    Some(rank) => enriched.with_rank(rank, entry.max_rank),
                    None => enriched,
                })
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
        let message = message.into();
        log::warn!("health: catalog failed — {message}");
        self.health.catalog = BackendHealth::failed(message)?;
        self.current_view()
    }

    pub fn record_log_monitor_failure(
        &mut self,
        message: impl Into<String>,
    ) -> Result<AppView, AppError> {
        let message = message.into();
        // The monitor thread re-records this every poll; log only the transition into the state.
        if self.health.log_monitor.state() != HealthState::Failed {
            log::warn!("health: log monitor failed — {message}");
        }
        self.health.log_monitor = BackendHealth::failed(message)?;
        self.current_view()
    }

    pub fn record_log_monitor_degraded(
        &mut self,
        message: impl Into<String>,
    ) -> Result<AppView, AppError> {
        let message = message.into();
        if self.health.log_monitor.state() != HealthState::Degraded {
            log::warn!("health: log monitor degraded — {message}");
        }
        self.health.log_monitor = BackendHealth::degraded(message)?;
        self.current_view()
    }

    pub fn record_log_monitor_ready(&mut self) -> Result<AppView, AppError> {
        if self.health.log_monitor.state() != HealthState::Ready {
            log::info!("health: log monitor ready");
        }
        self.health.log_monitor = BackendHealth::ready("EE.log monitor ready", None)?;
        self.current_view()
    }

    pub fn record_game_process_ready(&mut self) -> Result<AppView, AppError> {
        log::info!("health: game process ready");
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
        let observed_at = observed_at.into();
        log::info!("health: capture ready — {observed_at}");
        self.health.capture =
            BackendHealth::ready("Reward screen observer ready", Some(observed_at))?;
        self.current_view()
    }

    pub fn record_capture_source_ready(
        &mut self,
        source: &str,
        elapsed_ms: u128,
        observed_at: impl Into<String>,
    ) -> Result<AppView, AppError> {
        let observed_at = observed_at.into();
        let source = if source.eq_ignore_ascii_case("memory") {
            "Memory"
        } else {
            "OCR"
        };
        log::info!("health: capture ready — {source} ({elapsed_ms} ms) — {observed_at}");
        self.health.capture = BackendHealth::ready(
            format!("{source} reward observer ready ({elapsed_ms} ms)"),
            Some(observed_at),
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
        let observed_at = observed_at.into();
        log::info!("health: market ready — {priced} priced — {observed_at}");
        self.health.market = BackendHealth::ready(
            format!("warframe.market pricing ready ({priced} priced)"),
            Some(observed_at),
        )?;
        self.current_view()
    }

    pub fn record_market_degraded(
        &mut self,
        message: impl Into<String>,
    ) -> Result<AppView, AppError> {
        let message = message.into();
        log::warn!("health: market degraded — {message}");
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
        log::info!("health: collection prices ready — {dump_date} — {priced} items");
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
        let message = message.into();
        log::warn!("health: collection prices degraded — {message}");
        let last_success = self.health.collection_prices.last_success.clone();
        self.health.collection_prices =
            BackendHealth::new(HealthState::Degraded, message, last_success)?;
        self.current_view()
    }

    pub fn record_capture_degraded(
        &mut self,
        message: impl Into<String>,
    ) -> Result<AppView, AppError> {
        let message = message.into();
        log::warn!("health: capture degraded — {message}");
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
    market_account: MarketAccountView,
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

    pub fn market_account(&self) -> &MarketAccountView {
        &self.market_account
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CollectionView {
    items: Vec<CollectionItemView>,
    total_entries: usize,
    snapshot: Option<SnapshotMeta>,
    /// A live pricing pass in flight: the player asked about the page in front of them.
    pricing: Option<PricingProgress>,
}

/// How far a live pricing pass has got. One statement for the whole page, whoever asked for it:
/// the requests come out of one shared budget, so two counters would describe one queue twice.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub struct PricingProgress {
    pub done: usize,
    pub total: usize,
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

    pub fn pricing(&self) -> Option<PricingProgress> {
        self.pricing
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
    /// The maxed quote for a copy stopped part-way up. Present only with `platinum`, and only when
    /// the market prices the two ends of the range but nothing between them.
    #[serde(skip_serializing_if = "Option::is_none")]
    platinum_ceiling: Option<u32>,
    /// The rank these copies carry, absent for the unranked stack and for anything that cannot be
    /// ranked at all.
    #[serde(skip_serializing_if = "Option::is_none")]
    rank: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_rank: Option<u32>,
    live: bool,
    /// Whether warframe.market can be asked about this item at all.
    priceable: bool,
    /// How many of these the market completes in a month. Absent when nobody traded one today,
    /// which for a holding means the same as none.
    #[serde(skip_serializing_if = "Option::is_none")]
    monthly_trades: Option<u32>,
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

    /// The top of the range for a part-ranked copy the market brackets but never quotes.
    pub fn platinum_ceiling(&self) -> Option<u32> {
        self.platinum_ceiling
    }

    pub fn rank(&self) -> Option<u32> {
        self.rank
    }

    pub fn max_rank(&self) -> Option<u32> {
        self.max_rank
    }

    /// Whether this price came from warframe.market just now, rather than from the daily dump.
    pub fn live(&self) -> bool {
        self.live
    }

    /// Whether warframe.market has a listing this item's name resolves to. Not the same as having
    /// a price: a relic no dump in the last month has traded is priceable and shows a dash.
    pub fn priceable(&self) -> bool {
        self.priceable
    }

    /// The market's appetite for these, over a month. The other half of what a stack is worth: a
    /// correct 2p unit price on 182 copies is still not 364p if the game trades two a month.
    pub fn monthly_trades(&self) -> Option<u32> {
        self.monthly_trades
    }

    fn priced(
        entry: &warframe_domain::InventoryEntry,
        platinum: Option<u32>,
        live: bool,
        priceable: bool,
    ) -> Self {
        Self {
            platinum,
            live,
            priceable,
            ..Self::from(entry)
        }
    }

    fn with_price_ceiling(mut self, ceiling: Option<u32>) -> Self {
        self.platinum_ceiling = ceiling;
        self
    }

    fn with_monthly_trades(mut self, traded: Option<u32>) -> Self {
        self.monthly_trades = traded;
        self
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
            platinum_ceiling: None,
            rank: entry.rank,
            max_rank: entry.max_rank,
            live: false,
            priceable: false,
            monthly_trades: None,
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
    /// The linked warframe.market account, kept apart from `market` -- that row answers "could we
    /// reach warframe.market for a price", and this one answers "is an account connected". One
    /// can be healthy while the other is not.
    market_account: BackendHealth,
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
            market_account: BackendHealth::idle("No warframe.market account linked", None)?,
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

    pub fn market_account(&self) -> &BackendHealth {
        &self.market_account
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

/// What the collection says about one order.
///
/// Four states rather than a boolean because "we disagree" and "we cannot say" are different
/// claims with different consequences, and collapsing them is how a stale snapshot turns into a
/// screen of confident accusations.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "state")]
pub enum OrderStatus {
    Ok,
    /// The collection holds none of this. The order outlived its item.
    Missing,
    /// The order lists more than the collection holds.
    Overshoot {
        owned: u32,
    },
    /// The comparison cannot be made, so no claim is offered and no fix is put on the row.
    Unverifiable,
}

/// One order with the collection's opinion of it.
#[derive(Clone, Debug, Serialize)]
pub struct ReconciledOrder {
    pub order: MarketOrder,
    /// warframe.market's English name for the item, for the row. Absent only if the item table is
    /// missing the entry entirely.
    pub name: Option<String>,
    pub status: OrderStatus,
}

/// Judge each order against the collection.
///
/// The rule: **a mismatch is claimed only when the snapshot is coherent and newer than the
/// order.** Everything else is `Unverifiable`, which carries no flag and no fix.
///
/// That restraint is not caution for its own sake. The application's stated failure posture is to
/// keep the last coherent inventory when the reader breaks, so a snapshot can be stale or absent
/// while looking exactly like a current one from here. Judging against one produces confident
/// accusations about orders that were never wrong -- each with a delete button beside it.
pub fn reconcile_orders(
    orders: &[MarketOrder],
    items: &MarketItems,
    collection: &Collection,
    snapshot: Option<&SnapshotMeta>,
) -> Vec<ReconciledOrder> {
    // Quantity per card, summed across ranks. A card at two ranks is two entries in the
    // collection and one listing on the market, so comparing against either entry alone would
    // call a legitimate order an overshoot.
    let mut owned: std::collections::HashMap<&str, u32> = std::collections::HashMap::new();
    for entry in collection.entries() {
        *owned.entry(entry.item.id.catalog_path()).or_default() += entry.quantity;
    }

    orders
        .iter()
        .map(|order| ReconciledOrder {
            order: order.clone(),
            name: items.name(&order.item_id).map(str::to_owned),
            status: status_for(order, items, &owned, snapshot),
        })
        .collect()
}

fn status_for(
    order: &MarketOrder,
    items: &MarketItems,
    owned: &std::collections::HashMap<&str, u32>,
    snapshot: Option<&SnapshotMeta>,
) -> OrderStatus {
    // Owning none of something is the ordinary state for something you are trying to buy.
    if order.kind != OrderKind::Sell {
        return OrderStatus::Unverifiable;
    }
    // A rank names a particular copy, and the collection stores each rank separately. Which copies
    // the order means is not answerable from the order, so a maxed arcane listed at rank 5 is not
    // evidence about the unranked stack of the same card.
    if order.rank.is_some_and(|rank| rank > 0) {
        return OrderStatus::Unverifiable;
    }
    let (Some(snapshot), Some(updated_at)) = (snapshot, order.updated_at.as_deref()) else {
        return OrderStatus::Unverifiable;
    };
    // A snapshot older than the order describes a world before the order changed, and cannot
    // contradict it. Both sides are reduced to an instant first: the two timestamps are not in the
    // same format and comparing them as text is silently, permanently wrong -- production snapshot
    // metadata carries Unix seconds ("1785507554") while orders carry RFC 3339
    // ("2026-07-30T10:00:00Z"), and "1" sorts before "2", so a text comparison would mark every
    // order unverifiable on every real installation and the feature would ship doing nothing.
    let (Some(observed), Some(updated)) =
        (instant_of(snapshot.observed_at()), instant_of(updated_at))
    else {
        return OrderStatus::Unverifiable;
    };
    if observed <= updated {
        return OrderStatus::Unverifiable;
    }
    let Some(path) = items.catalog_path(&order.item_id) else {
        return OrderStatus::Unverifiable;
    };
    // The path exists but does not name one collection row: a relic's base projection stands for
    // four refinements the collection stores separately, and a set's path names the built item
    // rather than the parts a seller actually holds. Asking whether either is owned always answers
    // no, so an unguarded comparison would flag most of a real account's listings as missing and
    // offer to delete them.
    if !items.comparable(&order.item_id) {
        return OrderStatus::Unverifiable;
    }
    match owned.get(path).copied().unwrap_or(0) {
        0 => OrderStatus::Missing,
        held if held < order.quantity => OrderStatus::Overshoot { owned: held },
        _ => OrderStatus::Ok,
    }
}

/// Whether an account is linked, and whether it still works.
///
/// `NeedsRelink` is separate from `Unlinked` because they call for different things from the
/// player: one is an invitation, the other is a repair. Presenting a refused credential as an
/// unlinked account also loses the fact that something used to work, which is the part worth
/// reporting.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkState {
    #[default]
    Unlinked,
    Linked,
    NeedsRelink,
}

/// The account section's whole state.
#[derive(Clone, Debug, Default, Serialize)]
pub struct MarketAccountView {
    pub link: LinkState,
    /// Which credential store holds the token. Reported because a database file and a keyring are
    /// not equally strong, and the player is entitled to know which one they got.
    pub backing: Option<CredentialBacking>,
    pub orders: Vec<ReconciledOrder>,
    pub fetched_at: Option<String>,
    /// What the visible sell orders are asking, in total.
    pub listed_platinum: u32,
    /// How many orders carry a claim. Not a count of orders worth looking at -- an unverifiable
    /// order is not a problem, and counting one would put a false alarm on the navigation of every
    /// machine that has not read the game yet.
    pub flagged: usize,
    /// The collection paths this account may publish a listing for.
    ///
    /// Sent rather than recomputed on the frontend because the rule is `path_is_comparable`, which
    /// is measured against warframe.market's own table and lives in the crate that parses it. A
    /// second implementation in TypeScript would be a copy of a rule that exists to keep two
    /// vocabularies apart, and it would drift.
    pub listable: Vec<String>,
    /// What warframe.market shows this account as, and how it is being chosen.
    ///
    /// Not part of what `linked` computes: presence comes from a socket with its own lifetime, and
    /// an order fetch that reset it would blank the switch every refresh. `current_view` stamps it
    /// on instead, from the one place that holds it.
    pub presence: PresenceView,
}

/// The presence switch, as the screen needs it.
///
/// Two statuses rather than one, because they genuinely differ for the second or two a fresh
/// socket takes to be answered. Reporting only the committed one made the switch appear to ignore
/// the press: the reply to the click still said offline, and the choice did not appear to take
/// until the next poll seconds later.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct PresenceView {
    /// What the server last said it committed. `None` is offline: no socket, or one that has not
    /// been answered yet.
    pub status: Option<Presence>,
    /// What was last asked for. The switch marks this, so a press registers on the press, while
    /// `status` disagreeing is what the screen reports as not yet confirmed.
    pub wanted: Option<Presence>,
    /// Whether the status is being followed from the game reader rather than chosen by hand.
    pub auto: bool,
}

impl MarketAccountView {
    pub fn unlinked() -> Self {
        Self::default()
    }

    pub fn needs_relink() -> Self {
        Self {
            link: LinkState::NeedsRelink,
            ..Self::default()
        }
    }

    pub fn linked(
        backing: CredentialBacking,
        orders: Vec<ReconciledOrder>,
        fetched_at: String,
    ) -> Self {
        // A hidden order is offered to nobody and a buy order is money going out, so neither is
        // part of what this account is asking for.
        //
        // `platinum` prices one trade rather than one unit: a bulk listing of 300 relics at 18p
        // per six is asking 900p, not 5,400p. Measured against the live API, where roughly a third
        // of the orders on a traded relic carry `perTrade: 6`, so multiplying the two figures
        // straight together would overstate the headline number several times over.
        let listed_platinum = orders
            .iter()
            .filter(|entry| entry.order.visible && entry.order.kind == OrderKind::Sell)
            .map(|entry| {
                entry
                    .order
                    .platinum
                    .saturating_mul(entry.order.quantity / entry.order.per_trade.max(1))
            })
            .sum();
        let flagged = orders
            .iter()
            .filter(|entry| {
                matches!(
                    entry.status,
                    OrderStatus::Missing | OrderStatus::Overshoot { .. }
                )
            })
            .count();
        Self {
            link: LinkState::Linked,
            backing: Some(backing),
            orders,
            fetched_at: Some(fetched_at),
            listed_platinum,
            flagged,
            listable: Vec::new(),
            presence: PresenceView::default(),
        }
    }

    /// Which of the collection's paths can be listed, computed once against the item table.
    ///
    /// Separate from `linked` because the two callers that build a view already hold the table and
    /// the collection, and threading both through every construction site would make an argument
    /// list out of what is one join.
    #[must_use]
    pub fn with_listable(mut self, items: &MarketItems, collection: &Collection) -> Self {
        self.listable = collection
            .entries()
            .map(|entry| entry.item.id.catalog_path())
            .filter(|path| items.market_id_for_path(path).is_some())
            .map(str::to_owned)
            .collect();
        self.listable.dedup();
        self
    }
}

/// Seconds since the Unix epoch for either timestamp form this application produces.
///
/// Two forms exist and both are load-bearing. Production snapshot metadata records
/// `SystemTime`-derived Unix seconds; `SnapshotMeta::fake` and warframe.market both use RFC 3339.
/// The frontend already absorbs the same split in `freshness.ts`, which is how it went unnoticed.
///
/// Parsed by hand rather than by taking a date dependency for one inequality. Only the ordering
/// matters here, so this needs to be monotonic rather than calendar-exact: leap seconds and the
/// fractional part are ignored, and an offset other than `Z` is treated as unparseable rather than
/// guessed at.
fn instant_of(value: &str) -> Option<i64> {
    /// Seconds the epoch form is allowed to name: 2001 to 2603. Wide enough that no real clock
    /// leaves it, narrow enough that a millisecond value cannot pass as a second one.
    const PLAUSIBLE: std::ops::RangeInclusive<i64> = 1_000_000_000..=20_000_000_000;

    let value = value.trim();
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        // Bounded rather than parsed bare. A writer emitting milliseconds would otherwise read as
        // an instant tens of thousands of years out, which is newer than every order there will
        // ever be -- so a stale snapshot would judge, confidently and always.
        return value
            .parse()
            .ok()
            .filter(|seconds| PLAUSIBLE.contains(seconds));
    }
    let (date, rest) = value.split_once('T')?;
    // Anything not stated in UTC is left unparsed. A wrong guess about an offset moves an order
    // across the snapshot boundary, which turns "we cannot say" into a confident accusation.
    if !rest.ends_with('Z') {
        return None;
    }
    let time = rest.trim_end_matches('Z');
    let mut date_parts = date.split('-');
    let year: i64 = date_parts.next()?.parse().ok()?;
    let month: i64 = date_parts.next()?.parse().ok()?;
    let day: i64 = date_parts.next()?.parse().ok()?;
    let mut time_parts = time.split(':');
    let hour: i64 = time_parts.next()?.parse().ok()?;
    let minute: i64 = time_parts.next()?.parse().ok()?;
    let second: i64 = time_parts
        .next()
        .unwrap_or("0")
        .split('.')
        .next()?
        .parse()
        .ok()?;
    // Hinnant's days-from-civil, the standard algorithm, exact for every date this will see.
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

#[cfg(test)]
mod instant_tests {
    use super::instant_of;

    #[test]
    fn epoch_seconds_are_taken_as_written() {
        assert_eq!(instant_of("1785492000"), Some(1_785_492_000));
    }

    #[test]
    fn rfc_3339_utc_is_the_same_instant_as_its_epoch_seconds() {
        assert_eq!(instant_of("2026-07-31T10:00:00Z"), Some(1_785_492_000));
    }

    #[test]
    fn a_fractional_second_is_dropped_rather_than_rejected() {
        assert_eq!(instant_of("2026-07-31T10:00:00.482Z"), Some(1_785_492_000));
    }

    /// The branch that must never start guessing. An assumed offset moves an order across the
    /// snapshot boundary, turning "we cannot say" into a confident accusation with a delete
    /// button beside it.
    #[test]
    fn an_offset_other_than_utc_is_not_guessed_at() {
        assert_eq!(instant_of("2026-07-31T10:00:00+02:00"), None);
        assert_eq!(instant_of("2026-07-31T10:00:00-05:00"), None);
    }

    #[test]
    fn a_missing_seconds_field_reads_as_the_minute() {
        assert_eq!(instant_of("2026-07-31T10:00Z"), Some(1_785_492_000));
    }

    /// Milliseconds would otherwise parse as an instant tens of thousands of years out, which is
    /// newer than every order there will ever be -- so a stale snapshot would judge, always.
    #[test]
    fn a_millisecond_value_is_refused_rather_than_read_as_seconds() {
        assert_eq!(instant_of("1785492000123"), None);
    }

    #[test]
    fn implausibly_small_digit_strings_are_refused() {
        assert_eq!(instant_of("0"), None);
        assert_eq!(instant_of("42"), None);
    }

    #[test]
    fn malformed_input_yields_no_instant() {
        assert_eq!(instant_of(""), None);
        assert_eq!(instant_of("   "), None);
        assert_eq!(instant_of("yesterday"), None);
        assert_eq!(instant_of("2026-07-31"), None);
        assert_eq!(instant_of("2026-07-31Tten o'clockZ"), None);
    }
}
