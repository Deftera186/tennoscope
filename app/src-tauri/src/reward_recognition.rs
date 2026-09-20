//! Deep reward recognition: catalog-derived candidate pool, one background OCR reader,
//! recognition acceptance, early publication, screen closure and stale-result invalidation.
//!
//! Ownership model (architecture report #1):
//! - One worker thread owns the non-`Send` capture adapter. It constructs the adapter
//!   itself on each open epoch and drops it on close — the requesting thread never
//!   touches it. Capture takes no state mutex.
//! - `close` is nonblocking: it retires the epoch, invalidates pending/in-flight results
//!   and publication tokens, and asks the worker to drop its source. `shutdown`/`Drop`
//!   additionally wake and join the worker, so access revocation waits for in-flight
//!   capture and source destruction. `observe` and `poll` never wait for capture or join.
//! - Stale evidence cannot publish: every delivery carries the session epoch and the
//!   evidence revision it was captured under, and `poll`/`publish` re-validate both.
//!
//! The reward pool and the constraint context are swapped as whole snapshots; a delivery
//! is only accepted when the epoch and evidence revision it ran under are still current,
//! so a new epoch can never keep an old source or match pool.

use std::{
    sync::{
        Arc, Condvar, Mutex, MutexGuard,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use warframe_acquisition::{CatalogIndex, RelicRewardIndex, RewardCatalogEntry, RewardNeedle};

use crate::{reward_log::RewardLogEvent, reward_source::VisualRewardSource};

/// Public timing, matching the existing rates: two-second search cadence, 400ms watch
/// cadence after the cards are found, 45-minute watch lifetime, and 200ms/8s
/// log-triggered fast retries (off-thread).
#[derive(Clone, Copy, Debug)]
pub struct RecognitionTiming {
    pub interval: Duration,
    pub watch_interval: Duration,
    pub lifetime: Duration,
    pub retry_interval: Duration,
    pub retry_deadline: Duration,
}

impl RecognitionTiming {
    pub const fn live() -> Self {
        Self {
            interval: Duration::from_secs(2),
            watch_interval: Duration::from_millis(400),
            lifetime: Duration::from_secs(45 * 60),
            retry_interval: Duration::from_millis(200),
            retry_deadline: Duration::from_secs(8),
        }
    }
}

/// Consecutive routine misses before one is worth a warning.
///
/// Before cards are found the reader polls every two seconds, so fifteen uninterrupted routine
/// misses represent roughly thirty seconds without a usable reward read.
const ROUTINE_MISS_WARNING_STREAK: u32 = 15;

/// Whether this read failure deserves a warning, given how many times its reason has repeated.
///
/// Blank cards and pool misses are routine away from the reward screen, so only a sustained streak
/// warns. Other failures indicate broken capture and warn immediately. Each class warns only once
/// per uninterrupted streak.
pub(crate) fn poll_failure_is_worth_warning(reason: &str, consecutive: u32) -> bool {
    let routine = matches!(
        reason,
        "a reward card read as blank" | "reward card text did not match the relic pool"
    );
    if routine {
        consecutive == ROUTINE_MISS_WARNING_STREAK
    } else {
        consecutive == 1
    }
}

/// What one [`RewardRecognition::poll`] reports.
#[derive(Clone, Debug, Default)]
pub struct RecognitionUpdate {
    pub recognized: Option<RecognizedRewards>,
    pub hide: bool,
    pub failure: Option<FailureTrace>,
}

/// A recognized card set with its one-shot initial-publication token.
#[derive(Clone, Debug)]
pub struct RecognizedRewards {
    pub names: Vec<String>,
    pub elapsed: Duration,
    pub publication: RewardPublication,
}

/// A capture failure with its real reason and the real time the attempt took.
#[derive(Clone, Debug)]
pub struct FailureTrace {
    pub reason: String,
    pub elapsed: Duration,
}

/// A publication gate serialized with the epoch the recognition ran under.
///
/// `publish` declines once the module has moved to a later epoch or a later evidence
/// revision (a newer/closed screen), or after any earlier publication already ran for the
/// same epoch. It gates the initial overlay publication; delayed market effects use the
/// monitor generation guard instead, since the initial publish already consumes this token.
#[derive(Clone, Debug)]
pub struct RewardPublication {
    epoch: u64,
    revision: u64,
    gate: Arc<PublicationGate>,
}

impl RewardPublication {
    /// Run `effect` if this token is still the current session/evidence and nothing has
    /// published for it yet. Returns whether the effect ran.
    pub fn publish(&self, effect: impl FnOnce()) -> bool {
        self.gate.publish(self.epoch, self.revision, effect)
    }
}

/// The deep recognition module: background reader, candidate pool, constraints and the
/// publication gate.
pub struct RewardRecognition<S, F>
where
    S: VisualRewardSource + 'static,
    F: Fn() -> S + Send + Sync + 'static,
{
    shared: Arc<Shared>,
    /// The epoch that already published a recognition; duplicates for the same epoch are
    /// dropped, while a later epoch (after close) publishes afresh.
    resolved_epoch: Arc<AtomicU64>,
    /// Whether the game process is currently absent. While set, the worker holds no capture
    /// source and performs no captures; the pool is preserved so recognition resumes when the
    /// process returns. Only observed through `&mut` methods, so no atomics needed.
    suspended: bool,
    worker: Mutex<Option<JoinHandle<()>>>,
    /// The monitor generation gate lives in the monitor; this type parameter is part of
    /// the public contract (`make_source: F`).
    _source: std::marker::PhantomData<fn() -> S>,
    _factory: std::marker::PhantomData<fn() -> F>,
}

impl<S, F> RewardRecognition<S, F>
where
    S: VisualRewardSource + 'static,
    F: Fn() -> S + Send + Sync + 'static,
{
    /// Create the deep recognition module.
    ///
    /// `catalog`/`relics` derive the per-squad candidate pool on each baseline; `rewards`
    /// is the reward catalog used to translate candidates into OCR-match entries and to
    /// expose the pool through [`RewardRecognition::pool_entries`].
    pub fn new(
        catalog: Option<CatalogIndex>,
        relics: Option<RelicRewardIndex>,
        rewards: Vec<RewardCatalogEntry>,
        timing: RecognitionTiming,
        make_source: F,
    ) -> Self {
        let pool = Arc::new(Mutex::new(PoolSnapshot::default()));
        let constraint = Arc::new(Mutex::new(Constraint {
            context: None,
            revision: 1,
        }));
        let delivery = Arc::new(Mutex::new(None));
        let wake = Arc::new((Mutex::new(WakeState::default()), Condvar::new()));
        let epoch = Arc::new(AtomicU64::new(0));
        let revision = Arc::new(AtomicU64::new(1));
        let gate = Arc::new(PublicationGate {
            epoch: epoch.clone(),
            revision: revision.clone(),
            published: AtomicBool::new(false),
        });
        let resolved_epoch = Arc::new(AtomicU64::new(0));

        let shared = Arc::new(Shared {
            catalog,
            relics,
            rewards,
            timing,
            epoch: epoch.clone(),
            revision: revision.clone(),
            gate,
            constraint,
            pool,
            delivery,
            wake,
            stop: AtomicBool::new(false),
            paused: AtomicBool::new(false),
            make_source: Box::new(move || Box::new(make_source()) as Box<dyn VisualRewardSource>),
            retry_epoch: AtomicU64::new(0),
        });

        let worker_core = Arc::clone(&shared);
        let handle = std::thread::Builder::new()
            .name("reward-recognition".into())
            .spawn(move || worker_loop(worker_core))
            .expect("spawning the reward recognition worker");
        Self {
            shared,
            resolved_epoch,
            suspended: false,
            worker: Mutex::new(Some(handle)),
            _source: std::marker::PhantomData,
            _factory: std::marker::PhantomData,
        }
    }

    /// Update recognition context from a log event. Never performs a capture or a join.
    pub fn observe(&mut self, event: &RewardLogEvent) {
        match event {
            RewardLogEvent::BaselineRequested { relic_paths } => {
                self.set_pool(relic_paths);
                // A new baseline re-arms the worker's fast retry window so it starts
                // capturing for the freshly-painted screen right away.
                self.wake_worker();
            }
            RewardLogEvent::ChoicesReady {
                expected_choices,
                local_reward_path,
            } => {
                let context = ConstraintContext {
                    expected_choices: Some(*expected_choices),
                    local_reward: self.local_reward_name(local_reward_path),
                };
                self.set_constraint(Some(context));
            }
            RewardLogEvent::ResponsesComplete {
                screen_order,
                local_reward_path,
                ..
            } => {
                let context = ConstraintContext {
                    expected_choices: Some(screen_order.len()),
                    local_reward: self.local_reward_name(local_reward_path),
                };
                self.set_constraint(Some(context));
            }
            RewardLogEvent::Closed => self.close(),
            RewardLogEvent::RewardWindowOpened
            | RewardLogEvent::ResponderExpected { .. }
            | RewardLogEvent::ResponderReceived { .. } => {}
        }
    }

    /// Replace the candidate pool with the one this baseline's relics resolve to, and
    /// re-arm the worker's fast retry window.
    fn set_pool(&mut self, relic_paths: &[String]) {
        let shared = &self.shared;
        // A fresh baseline means the game is present and a new screen may follow.
        self.suspended = false;
        shared.paused.store(false, Ordering::Release);
        // First baseline opens the initial closed epoch; later baselines reuse the open epoch
        // so late relics grow the pool without resetting recognition.
        if shared.epoch.load(Ordering::Acquire) == 0 {
            shared.epoch.store(1, Ordering::Release);
        }
        let candidates = derive_candidates(shared, relic_paths);
        let entries = pool_entries(&candidates, &shared.rewards);
        let mut guard = lock(&shared.pool);
        guard.relics = relic_paths.to_vec();
        guard.candidates = candidates;
        guard.entries = entries;
        drop(guard);
        // A new pool retires in-flight captures: a read matched against the old pool must not
        // publish under the new one. Baselines arrive minutes before the screen, not after
        // publication, so retiring delayed effects here is safe in practice.
        let mut constraint = lock(&shared.constraint);
        constraint.revision = shared.revision.fetch_add(1, Ordering::AcqRel) + 1;
        drop(constraint);
        shared.retry_epoch.fetch_add(1, Ordering::AcqRel);
        self.wake_worker();
    }

    fn set_constraint(&mut self, context: Option<ConstraintContext>) {
        let shared = &self.shared;
        let mut guard = lock(&shared.constraint);
        guard.context = context;
        guard.revision = shared.revision.fetch_add(1, Ordering::AcqRel) + 1;
        drop(guard);
        // A constraint arrival kick-starts fast retries on the worker: the log announces
        // the rewards before Warframe paints the cards, so 200ms re-reads find them fast.
        shared.retry_epoch.fetch_add(1, Ordering::AcqRel);
        self.wake_worker();
    }

    fn local_reward_name(&self, local_reward_path: &Option<String>) -> Option<String> {
        let path = local_reward_path.as_deref()?;
        let pool = lock(&self.shared.pool);
        reward_from_path(path, &pool.candidates)
    }

    /// Nonblocking close: invalidate the active epoch, pending reads, constraints and
    /// candidate context, and retire publication tokens. The worker drops its source on
    /// its own thread; `shutdown` joins it first when revocation requires it.
    pub fn close(&mut self) {
        let shared = &self.shared;
        self.suspended = false;
        shared.paused.store(false, Ordering::Release);
        // Retire the epoch. In-flight captures see the new epoch and refuse to deliver.
        // At cold start (epoch 0) there is no session to retire: bumping would arm a
        // watch deadline the worker could expire before the first baseline opens.
        if shared.epoch.load(Ordering::Acquire) != 0 {
            let epoch = shared.epoch.fetch_add(1, Ordering::AcqRel) + 1;
            shared.gate.retire();
            let _ = epoch;
        }
        // Drop any pending delivery; nothing captured under the retired epoch is usable.
        *lock(&shared.delivery) = None;
        // Clear the constraint context: a stale roster must not gate a future session.
        let mut guard = lock(&shared.constraint);
        guard.context = None;
        guard.revision = shared.revision.fetch_add(1, Ordering::AcqRel) + 1;
        drop(guard);
        // Clear the pool: a new session must start from a fresh baseline. The epoch bump
        // above already re-arms resolved-once: the stored epoch no longer matches.
        let mut pool = lock(&shared.pool);
        pool.relics.clear();
        pool.candidates.clear();
        pool.entries.clear();
        drop(pool);
        // Wake the worker so it drops its source now rather than at the next capture.
        self.wake_worker();
    }

    /// Suspend recognition while the game process is absent: retire the epoch and drop the
    /// capture source without clearing the pool, so recognition resumes when the process
    /// returns. Idempotent: repeated ticks while absent do no further work. Never waits for
    /// capture or joins.
    pub fn suspend(&mut self) {
        if self.suspended {
            return;
        }
        self.suspended = true;
        let shared = &self.shared;
        let old_epoch = shared.epoch.load(Ordering::Acquire);
        if old_epoch != 0 {
            // A session is open. Carry an already-published screen across the pause so its
            // cards are not published a second time when the process returns.
            let new_epoch = shared.epoch.fetch_add(1, Ordering::AcqRel) + 1;
            if self.resolved_epoch.load(Ordering::Acquire) == old_epoch {
                self.resolved_epoch.store(new_epoch, Ordering::Release);
            }
            shared.gate.retire();
        }
        // At cold start (epoch 0) there is no session: no epoch bump, no deadline arm, no
        // resolved-epoch carry -- otherwise the first fissure would be suppressed as a
        // duplicate and a 45-minute watch would arm for a session that never opened.
        *lock(&shared.delivery) = None;
        shared.paused.store(true, Ordering::Release);
        // The worker drops its source on pause; the pool and constraints are preserved.
        self.wake_worker();
    }

    /// Resume after [`RewardRecognition::suspend`]: the process is present again, so the worker
    /// may capture against the preserved pool. No-op unless suspended. Never blocks.
    pub fn resume(&mut self) {
        if !self.suspended {
            return;
        }
        self.suspended = false;
        self.shared.paused.store(false, Ordering::Release);
        self.wake_worker();
    }

    /// Signal the worker to exit and join it. Terminal: the worker thread ends here.
    /// Normal screen/mission turnover uses `close`, which retires the epoch but keeps the
    /// worker alive for the next baseline.
    pub fn shutdown(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        self.wake_worker();
        if let Some(handle) = self.worker.lock().unwrap().take() {
            let _ = handle.join();
        }
    }

    /// One requested recognition result, or `None` when nothing new is available.
    ///
    /// Never waits for capture or for the worker; it drains the delivery slot and applies
    /// the current (possibly newer) constraints to what was recognized.
    pub fn poll(&mut self) -> RecognitionUpdate {
        let shared = &self.shared;
        let current_epoch = shared.epoch.load(Ordering::Acquire);
        let current_revision = shared.revision.load(Ordering::Acquire);

        let Some(delivery) = lock(&shared.delivery).take() else {
            return RecognitionUpdate::default();
        };
        // A delivery is only valid when the session epoch and evidence revision it was
        // captured under are still current — queued or in-flight results from a closed or
        // advanced epoch can never be delivered.
        if delivery.epoch != current_epoch || delivery.revision != current_revision {
            return RecognitionUpdate::default();
        }
        match delivery.kind {
            DeliveryKind::Recognized { names, elapsed } => {
                if self.resolved_epoch.load(Ordering::Acquire) == current_epoch {
                    return RecognitionUpdate::default();
                }
                let constraint = lock(&shared.constraint).clone();
                let context = constraint.context.as_ref();
                let expected = context.and_then(|c| c.expected_choices);
                if let Some(expected) = expected
                    && names.len() != expected
                {
                    // The screen has a different number of cards than the log asserts.
                    // Do not publish a known-mismatched set.
                    return RecognitionUpdate::default();
                }
                if let Some(local) = context.and_then(|c| c.local_reward.as_deref())
                    && !names.iter().any(|name| name == local)
                {
                    // The read did not include the reward EE.log already confirmed for the
                    // local player; drop it rather than show something wrong.
                    return RecognitionUpdate::default();
                }
                self.resolved_epoch.store(current_epoch, Ordering::Release);
                RecognitionUpdate {
                    recognized: Some(RecognizedRewards {
                        names,
                        elapsed,
                        publication: RewardPublication {
                            epoch: current_epoch,
                            revision: current_revision,
                            gate: Arc::clone(&shared.gate),
                        },
                    }),
                    hide: false,
                    failure: None,
                }
            }
            DeliveryKind::Failed { reason, elapsed } => RecognitionUpdate {
                recognized: None,
                hide: false,
                failure: Some(FailureTrace { reason, elapsed }),
            },
            DeliveryKind::Gone => RecognitionUpdate {
                recognized: None,
                hide: true,
                failure: None,
            },
        }
    }

    /// The currently active candidate needles (for gated memory diagnostics).
    pub fn candidates(&self) -> Vec<RewardNeedle> {
        lock(&self.shared.pool).candidates.clone()
    }

    /// The reward-catalog entries the current pool resolves to (for price warming).
    pub fn pool_entries(&self) -> Vec<RewardCatalogEntry> {
        lock(&self.shared.pool).entries.clone()
    }

    /// The full reward catalog this module was constructed with.
    pub fn rewards(&self) -> &[RewardCatalogEntry] {
        &self.shared.rewards
    }

    /// Whether the current epoch has already published a recognition.
    pub fn resolved(&self) -> bool {
        self.resolved_epoch.load(Ordering::Acquire) == self.shared.epoch.load(Ordering::Acquire)
            && self.shared.epoch.load(Ordering::Acquire) != 0
    }

    /// Wake the worker so it re-reads pool, constraints and cadence immediately.
    fn wake_worker(&self) {
        let (state, condvar) = &*self.shared.wake;
        let mut guard = state.lock().unwrap();
        guard.wake = guard.wake.saturating_add(1);
        condvar.notify_all();
    }
}

impl<S, F> Drop for RewardRecognition<S, F>
where
    S: VisualRewardSource + 'static,
    F: Fn() -> S + Send + Sync + 'static,
{
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn lock<T>(guard: &Mutex<T>) -> MutexGuard<'_, T> {
    guard.lock().unwrap()
}

/// Everything the worker and the requesting thread share. The capture adapter never
/// appears here: the worker constructs and drops it inside its own loop through
/// `make_source`, so a non-`Send` source never crosses a thread boundary.
struct Shared {
    catalog: Option<CatalogIndex>,
    relics: Option<RelicRewardIndex>,
    rewards: Vec<RewardCatalogEntry>,
    timing: RecognitionTiming,
    epoch: Arc<AtomicU64>,
    revision: Arc<AtomicU64>,
    gate: Arc<PublicationGate>,
    constraint: Arc<Mutex<Constraint>>,
    pool: Arc<Mutex<PoolSnapshot>>,
    delivery: Arc<Mutex<Option<Delivery>>>,
    wake: Arc<(Mutex<WakeState>, Condvar)>,
    /// Set once by `shutdown`; the worker exits instead of waiting for the next epoch.
    stop: AtomicBool,
    /// Set while the game process is absent; the worker drops its source and waits without
    /// capturing until `resume` clears it. The pool is preserved across the pause.
    paused: AtomicBool,
    /// Factory is `Send + Sync`; the source it produces is used only on the worker.
    make_source: Box<dyn Fn() -> Box<dyn VisualRewardSource> + Send + Sync>,
    /// Bumped whenever the monitor arms a new pool or asserts new constraints; the worker
    /// re-arms its fast retry window when this changes.
    retry_epoch: AtomicU64,
}
#[derive(Default)]
struct PoolSnapshot {
    relics: Vec<String>,
    candidates: Vec<RewardNeedle>,
    entries: Vec<RewardCatalogEntry>,
}
#[derive(Clone, Debug)]
struct ConstraintContext {
    /// The number of cards EE.log expects on screen, once it has said so.
    expected_choices: Option<usize>,
    /// The local player's reward choice name, once EE.log stated it.
    local_reward: Option<String>,
}

#[derive(Clone, Debug)]
struct Constraint {
    context: Option<ConstraintContext>,
    revision: u64,
}

#[derive(Default)]
struct WakeState {
    wake: usize,
}

#[derive(Clone, Debug)]
struct Delivery {
    epoch: u64,
    revision: u64,
    kind: DeliveryKind,
}

#[derive(Clone, Debug)]
enum DeliveryKind {
    Recognized {
        names: Vec<String>,
        elapsed: Duration,
    },
    Failed {
        reason: String,
        elapsed: Duration,
    },
    Gone,
}

/// Publication gate. `published` marks the epoch that already ran its effect, so only one
/// effect runs per epoch and a late (market) effect on the same epoch is declined.
#[derive(Debug)]
struct PublicationGate {
    epoch: Arc<AtomicU64>,
    revision: Arc<AtomicU64>,
    published: AtomicBool,
}

impl PublicationGate {
    /// Clear the one-shot published mark so the next epoch can publish. The epoch and revision
    /// themselves advance in `close`/`suspend`, which share these counters.
    fn retire(&self) {
        let _ = self.epoch.load(Ordering::Acquire);
        self.published.store(false, Ordering::Release);
    }

    fn publish(&self, epoch: u64, revision: u64, effect: impl FnOnce()) -> bool {
        if self.epoch.load(Ordering::Acquire) != epoch
            || self.revision.load(Ordering::Acquire) != revision
        {
            return false;
        }
        // Epoch and revision were current; only the first effect for this epoch runs.
        if self.published.swap(true, Ordering::AcqRel) {
            return false;
        }
        effect();
        true
    }
}

/// Derive the candidate needles this squad's relic paths resolve to.
fn derive_candidates(shared: &Shared, relic_paths: &[String]) -> Vec<RewardNeedle> {
    let Some(catalog) = shared.catalog.as_ref() else {
        return Vec::new();
    };
    let Some(relics) = shared.relics.as_ref() else {
        return Vec::new();
    };
    relics.candidates_for_projection_paths(relic_paths, catalog)
}

/// The narrow relic pool as catalog entries, so the visual source matches exactly the
/// rewards this squad's relics can produce. Duplicate choices (two relics with the same
/// reward) appear once.
fn pool_entries(
    candidates: &[RewardNeedle],
    rewards: &[RewardCatalogEntry],
) -> Vec<RewardCatalogEntry> {
    let mut entries = Vec::with_capacity(candidates.len());
    for needle in candidates {
        let name = needle.choice_name();
        if entries
            .iter()
            .any(|entry: &RewardCatalogEntry| entry.name == name)
        {
            continue;
        }
        let ducats = rewards
            .iter()
            .find(|entry| warframe_acquisition::reward_name_matches(&entry.name, name))
            .map_or(0, |entry| entry.ducats);
        entries.push(RewardCatalogEntry {
            name: name.to_owned(),
            ducats,
        });
    }
    entries
}

/// Map a log reward path onto the matching candidate's choice name.
///
/// `EE.log` announces rewards with StoreItems-style paths while the catalog stores
/// Types-style paths, so the equivalence is checked explicitly.
fn reward_from_path(path: &str, candidates: &[RewardNeedle]) -> Option<String> {
    candidates
        .iter()
        .find(|needle| {
            needle.internal_paths().iter().any(|candidate| {
                std::str::from_utf8(candidate)
                    .ok()
                    .is_some_and(|catalog_path| reward_path_matches(path, catalog_path))
            })
        })
        .map(|needle| needle.choice_name().to_owned())
}

fn reward_path_matches(log_path: &str, catalog_path: &str) -> bool {
    log_path == catalog_path
        || log_path
            .strip_prefix("/Lotus/StoreItems")
            .is_some_and(|suffix| catalog_path == format!("/Lotus{suffix}"))
}

fn worker_loop(shared: Arc<Shared>) {
    // Session-local worker state, reset on each epoch change.
    let mut source: Option<Box<dyn VisualRewardSource>> = None;
    let mut last_epoch = 0;
    let mut session_deadline = None;
    let mut found = false;
    let mut missed = 0_u32;
    let mut fast_epoch = shared.retry_epoch.load(Ordering::Acquire);
    let mut fast_deadline: Option<Instant> = None;
    let mut last_reason: Option<String> = None;
    let mut last_elapsed = Duration::ZERO;
    let mut fast_reported = true;
    // Warning streak for delivered failures (background cadence only).
    let mut warn_reason: Option<String> = None;
    let mut warn_streak: u32 = 0;
    // Set once the watch lifetime expires; cleared on epoch turnover. While set the worker
    // parks instead of capturing, so a same-epoch wake cannot resume deadline-free polling.
    let mut expired = false;

    loop {
        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        let epoch = shared.epoch.load(Ordering::Acquire);
        if epoch != last_epoch {
            // New session: drop the previous epoch's source and reset session state.
            source = None;
            found = false;
            missed = 0;
            fast_deadline = None;
            fast_reported = true;
            last_reason = None;
            warn_reason = None;
            warn_streak = 0;
            expired = false;
            last_epoch = epoch;
            if epoch == 0 {
                // Closed: the source is dropped here, on the worker thread.
                session_deadline = None;
            } else {
                session_deadline = Some(Instant::now() + shared.timing.lifetime);
                // A fresh baseline re-arms the fast retry window.
                fast_epoch = shared.retry_epoch.load(Ordering::Acquire);
                fast_deadline = Some(Instant::now() + shared.timing.retry_deadline);
                fast_reported = false;
            }
        }

        if epoch == 0 {
            // Retired. Wait for the next open epoch without polling.
            wait_for_wake(&shared, None);
            continue;
        }

        if shared.paused.load(Ordering::Acquire) {
            // Game process absent: hold no source and perform no captures. The pool is
            // preserved, so recognition resumes where it left off on `resume`.
            source = None;
            wait_for_wake(&shared, None);
            continue;
        }

        // The retry window re-arms when the monitor touches pool/constraints.
        let retry_now = shared.retry_epoch.load(Ordering::Acquire);
        if retry_now != fast_epoch {
            fast_epoch = retry_now;
            fast_deadline = Some(Instant::now() + shared.timing.retry_deadline);
            fast_reported = false;
        }

        if let Some(deadline) = session_deadline
            && Instant::now() >= deadline
        {
            // The 45-minute watch lifetime ended without the screen closing. Drop the
            // source and park until epoch turnover: resuming capture on the same epoch
            // would run with no deadline. Found and miss state belongs to the expired
            // screen, not the next one.
            source = None;
            session_deadline = None;
            found = false;
            missed = 0;
            expired = true;
            wait_for_wake(&shared, None);
            continue;
        }

        if expired {
            // Parked after lifetime expiry; only epoch turnover (close/suspend) resumes.
            wait_for_wake(&shared, None);
            continue;
        }

        let entries = lock(&shared.pool).entries.clone();
        if entries.is_empty() {
            // No pool: nothing to match against. Do not capture; a later non-empty pool
            // (next baseline) wakes the worker.
            let _ = wait_for_wake(&shared, Some(shared.timing.interval));
            continue;
        }

        if source.is_none() {
            source = Some((shared.make_source)());
        }

        let now = Instant::now();
        let in_fast = fast_deadline.is_some_and(|deadline| now < deadline);
        if !in_fast && fast_deadline.is_some() {
            // Fast window just exhausted: report the actual reason once, then resume.
            fast_deadline = None;
            if !found && !fast_reported {
                fast_reported = true;
                if let Some(reason) = last_reason.clone() {
                    let mut slot = lock(&shared.delivery);
                    *slot = Some(Delivery {
                        epoch: shared.epoch.load(Ordering::Acquire),
                        revision: shared.revision.load(Ordering::Acquire),
                        kind: DeliveryKind::Failed {
                            reason,
                            elapsed: last_elapsed,
                        },
                    });
                }
            }
        }
        let in_fast = fast_deadline.is_some_and(|deadline| Instant::now() < deadline);

        // Read the constraint snapshot this capture runs under.
        let constraint = lock(&shared.constraint).clone();
        let context = constraint.context.clone();
        let expected = context.as_ref().and_then(|c| c.expected_choices);
        let local = context.as_ref().and_then(|c| c.local_reward.clone());
        let capture_revision = constraint.revision;
        drop(constraint);
        let capture_epoch = shared.epoch.load(Ordering::Acquire);

        let started = Instant::now();
        let outcome = source.as_mut().map(|source| source.choices(&entries));
        let elapsed = started.elapsed();

        let deliver = |kind: DeliveryKind| {
            let mut slot = lock(&shared.delivery);
            // A pending Recognized read is the valuable signal: failures or Gone arriving before
            // the monitor drains it must not overwrite it. The monitor publishes on drain, and a
            // later Gone (or the log's Closed line) still hides the overlay.
            if matches!(kind, DeliveryKind::Recognized { .. }) {
                *slot = Some(Delivery {
                    epoch: capture_epoch,
                    revision: capture_revision,
                    kind,
                });
                return;
            }
            let overwrite = slot
                .as_ref()
                .is_none_or(|pending| !matches!(pending.kind, DeliveryKind::Recognized { .. }));
            if overwrite {
                *slot = Some(Delivery {
                    epoch: capture_epoch,
                    revision: capture_revision,
                    kind,
                });
            }
        };

        // Returns true when this attempt recognized cards.
        let recognized = match outcome {
            Some(Ok(names)) if names.len() >= 2 => {
                let size_mismatch = expected.is_some_and(|expected| names.len() != expected);
                let local_mismatch = local
                    .as_deref()
                    .is_some_and(|local| !names.iter().any(|name| name == local));
                if local_mismatch {
                    let reason = "the reward screen did not show the logged reward";
                    last_reason = Some(reason.into());
                    last_elapsed = elapsed;
                    if !in_fast {
                        deliver(DeliveryKind::Failed {
                            reason: reason.to_owned(),
                            elapsed,
                        });
                        (warn_reason, warn_streak) = if warn_reason.as_deref() == Some(reason) {
                            (warn_reason, warn_streak.saturating_add(1))
                        } else {
                            (Some(reason.to_owned()), 1)
                        };
                        if poll_failure_is_worth_warning(reason, warn_streak) {
                            log::warn!("[DEBUG-recognition] read failed: {reason}");
                        } else {
                            log::debug!(
                                "[DEBUG-recognition] read failed: {reason} (x{warn_streak})"
                            );
                        }
                    }
                    false
                } else if size_mismatch {
                    let reason = "the reward screen showed a different number of cards";
                    last_reason = Some(reason.into());
                    last_elapsed = elapsed;
                    if !in_fast {
                        deliver(DeliveryKind::Failed {
                            reason: reason.to_owned(),
                            elapsed,
                        });
                        (warn_reason, warn_streak) = if warn_reason.as_deref() == Some(reason) {
                            (warn_reason, warn_streak.saturating_add(1))
                        } else {
                            (Some(reason.to_owned()), 1)
                        };
                        if poll_failure_is_worth_warning(reason, warn_streak) {
                            log::warn!("[DEBUG-recognition] read failed: {reason}");
                        } else {
                            log::debug!(
                                "[DEBUG-recognition] read failed: {reason} (x{warn_streak})"
                            );
                        }
                    }
                    false
                } else {
                    // Retain the exact pool alongside the published names in the capture trace.
                    // At Info, so a stable build keeps it: a confident wrong read otherwise
                    // leaves no line saying which fissure the pool belonged to. Once per screen:
                    // later duplicates are dropped by resolved-once at poll time, so logging
                    // them would spam one line per watch interval while the screen stays up.
                    if !found {
                        let pool = lock(&shared.pool);
                        let relics = pool
                            .relics
                            .iter()
                            .map(|path| path.rsplit('/').next().unwrap_or(path))
                            .collect::<Vec<_>>();
                        log::info!(
                            "reward: published cards={names:?} pool={} relics={relics:?}",
                            pool.entries.len(),
                        );
                        drop(pool);
                    }
                    warn_reason = None;
                    warn_streak = 0;
                    deliver(DeliveryKind::Recognized { names, elapsed });
                    found = true;
                    missed = 0;
                    fast_deadline = None;
                    fast_reported = true;
                    true
                }
            }
            _ => {
                let reason = match outcome {
                    Some(Err(reason)) => reason.to_owned(),
                    _ => "a reward card read as blank".to_owned(),
                };
                last_reason = Some(reason.clone());
                last_elapsed = elapsed;
                if !in_fast {
                    deliver(DeliveryKind::Failed {
                        reason: reason.clone(),
                        elapsed,
                    });
                    (warn_reason, warn_streak) = if warn_reason.as_deref() == Some(&reason) {
                        (warn_reason, warn_streak.saturating_add(1))
                    } else {
                        (Some(reason.clone()), 1)
                    };
                    if poll_failure_is_worth_warning(&reason, warn_streak) {
                        log::warn!("[DEBUG-recognition] read failed: {reason}");
                    } else {
                        log::debug!("[DEBUG-recognition] read failed: {reason} (x{warn_streak})");
                    }
                }
                false
            }
        };

        if !recognized && found {
            // A blank/mismatched read after cards were seen may mean the screen closed.
            missed += 1;
            if missed >= 2 {
                log::debug!("[DEBUG-recognition] reward screen gone");
                deliver(DeliveryKind::Gone);
                found = false;
                missed = 0;
                source = None;
                // A fresh screen starts a fresh warning streak.
                warn_reason = None;
                warn_streak = 0;
            }
        }

        if shared.stop.load(Ordering::Acquire) {
            break;
        }
        let wait = if found {
            shared.timing.watch_interval
        } else if in_fast {
            shared.timing.retry_interval
        } else {
            shared.timing.interval
        };
        let _ = wait_for_wake(&shared, Some(wait));
    }
}

/// Wait for the given duration, or until the monitor `wake`s the worker. `duration` of
/// `None` waits indefinitely; returns the time spent.
fn wait_for_wake(shared: &Shared, duration: Option<Duration>) -> Duration {
    let (state, condvar) = &*shared.wake;
    let started = Instant::now();
    let mut guard = state.lock().unwrap();
    if guard.wake > 0 {
        guard.wake -= 1;
        return started.elapsed();
    }
    loop {
        let Some(timeout) = duration else {
            let mut guard = condvar
                .wait(guard)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if guard.wake > 0 {
                guard.wake -= 1;
            }
            return started.elapsed();
        };
        let elapsed = started.elapsed();
        let Some(remaining) = timeout.checked_sub(elapsed) else {
            return elapsed;
        };
        if remaining.is_zero() {
            return elapsed;
        }
        let (next, result) = condvar
            .wait_timeout(guard, remaining)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard = next;
        if result.timed_out() {
            return started.elapsed();
        }
        if guard.wake > 0 {
            guard.wake -= 1;
            return started.elapsed();
        }
        // Spurious wake; loop again against the remaining deadline.
    }
}
