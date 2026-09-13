//! Shared timing policy for expensive Screen Observer backend attempts.
//!
//! Capability probes use [`LatchingAvailability`]: a success remains true for the process,
//! failures expire after a bounded interval, and retry probes admit one caller. Failed capture
//! operations use [`RetryCooldown`]: success clears the deadline, failure starts the same bounded
//! cooldown without turning a working capability into a permanent latch.

use std::sync::Mutex;
use std::time::{Duration, Instant};

/// A capability answer that latches once it turns positive.
///
/// `probe` is supplied by the caller at each ask rather than stored, so this module never learns
/// what a Wayland registry or a D-Bus name is: the whole of the retry, latching and
/// single-prober policy is here, and each backend contributes only its own probe.
///
/// The probe deliberately runs outside the lock. It opens a Wayland connection or waits on a
/// D-Bus reply, and holding the mutex across that would stall the poller behind an unrelated
/// backend's handshake. `probing` is what keeps the released lock from admitting a second prober.
pub struct LatchingAvailability {
    state: Mutex<Option<State>>,
    retry_interval: Duration,
}

#[derive(Clone, Copy)]
struct State {
    available: bool,
    checked_at: Instant,
    probing: bool,
}

impl LatchingAvailability {
    pub const fn new(retry_interval: Duration) -> Self {
        Self {
            state: Mutex::new(None),
            retry_interval,
        }
    }

    /// The cached answer, probing only when the cache cannot answer.
    ///
    /// The first ask probes while holding the lock, so a concurrent first caller waits for the
    /// real answer instead of being told "unavailable". That matters because a spurious negative
    /// downgrades the caller to the next backend in precedence, and on KDE that means the portal
    /// and its visible screen-chooser dialog. Retry probes run with the lock released -- there is
    /// a cached answer to serve meanwhile, and `probing` keeps a second prober out.
    ///
    /// A poisoned lock is recovered rather than propagated: a panicked prober leaves a
    /// well-formed state behind, and taking the capture path down over it would cost the player
    /// the reward reader for the rest of the session.
    pub fn available(&self, probe: impl FnOnce() -> bool) -> bool {
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let Some(state) = guard.as_mut() else {
            let probed = probe();
            *guard = Some(State {
                available: probed,
                checked_at: Instant::now(),
                probing: false,
            });
            return probed;
        };
        if !state.claim_probe(Instant::now(), self.retry_interval) {
            return state.available;
        }
        drop(guard);

        let probed = probe();
        let mut guard = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *guard = Some(State {
            available: probed,
            checked_at: Instant::now(),
            probing: false,
        });
        probed
    }
}
/// A bounded cooldown after a failed or rate-limited operation.
///
/// Unlike [`LatchingAvailability`], success does not latch a capability answer. It clears the last
/// failure so the next ordinary operation remains eligible. [`Self::claim_retry`] atomically starts
/// the interval as it admits a caller, matching shared operations whose attempts themselves are
/// rate-limited; per-reader operations can check [`Self::retry_ready`] and record their outcome.
pub struct RetryCooldown {
    failed_at: Mutex<Option<Instant>>,
    retry_interval: Duration,
}

impl RetryCooldown {
    pub const fn new(retry_interval: Duration) -> Self {
        Self {
            failed_at: Mutex::new(None),
            retry_interval,
        }
    }

    /// Whether an operation may run now, without claiming a shared retry.
    pub fn retry_ready(&self) -> bool {
        self.retry_ready_at(Instant::now())
    }

    pub(crate) fn retry_ready_at(&self, now: Instant) -> bool {
        self.failed_at
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .is_none_or(|failed_at| now.saturating_duration_since(failed_at) >= self.retry_interval)
    }

    /// Admit one shared caller and start its interval atomically.
    pub fn claim_retry(&self) -> bool {
        self.claim_retry_at(Instant::now())
    }

    pub(crate) fn claim_retry_at(&self, now: Instant) -> bool {
        let mut failed_at = self
            .failed_at
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failed_at
            .is_some_and(|failed_at| now.saturating_duration_since(failed_at) < self.retry_interval)
        {
            return false;
        }
        *failed_at = Some(now);
        true
    }

    /// Start a cooldown from a failed operation.
    pub fn failed(&self) {
        self.failed_at(Instant::now());
    }

    pub(crate) fn failed_at(&self, now: Instant) {
        *self
            .failed_at
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(now);
    }

    /// Clear the failure cooldown.
    pub fn succeeded(&self) {
        *self
            .failed_at
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    }
}

impl State {
    fn claim_probe(&mut self, now: Instant, retry_interval: Duration) -> bool {
        if self.available
            || self.probing
            || now.saturating_duration_since(self.checked_at) < retry_interval
        {
            return false;
        }
        self.probing = true;
        true
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    use super::LatchingAvailability;

    const RETRY: Duration = Duration::from_secs(5);

    #[test]
    fn latches_a_success_for_the_life_of_the_process() {
        let probes = AtomicUsize::new(0);
        let cache = LatchingAvailability::new(RETRY);
        let probe = || {
            probes.fetch_add(1, Ordering::Relaxed);
            true
        };

        assert!(cache.available(probe));
        assert!(cache.available(probe));
        assert!(cache.available(probe));

        assert_eq!(
            probes.load(Ordering::Relaxed),
            1,
            "a compositor that can capture is not asked twice"
        );
    }

    #[test]
    fn retries_a_failure_only_after_the_interval() {
        let probes = AtomicUsize::new(0);
        let cache = LatchingAvailability::new(RETRY);
        let probe = || {
            probes.fetch_add(1, Ordering::Relaxed);
            false
        };

        assert!(!cache.available(probe));
        assert_eq!(probes.load(Ordering::Relaxed), 1);

        // Every poll inside the interval is answered from the cache, so a compositor that is
        // simply absent does not put a handshake in each 400 ms poll.
        assert!(!cache.available(probe));
        assert!(!cache.available(probe));
        assert_eq!(probes.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn a_failure_becomes_available_when_the_compositor_appears() {
        let succeed = AtomicUsize::new(0);
        let cache = LatchingAvailability::new(Duration::ZERO);
        let probe = || succeed.load(Ordering::Relaxed) > 0;

        assert!(!cache.available(probe));
        succeed.store(1, Ordering::Relaxed);
        assert!(
            cache.available(probe),
            "a zero interval retries immediately, so a compositor that starts late is found"
        );
    }
    #[test]
    fn failed_operation_cools_down_until_the_retry_interval_expires() {
        let start = std::time::Instant::now();
        let cooldown = super::RetryCooldown::new(RETRY);

        cooldown.failed_at(start);

        assert!(!cooldown.retry_ready_at(start));
        assert!(!cooldown.retry_ready_at(start + RETRY / 2));
        assert!(cooldown.retry_ready_at(start + RETRY));
    }

    #[test]
    fn successful_operation_clears_a_failure_cooldown() {
        let start = std::time::Instant::now();
        let cooldown = super::RetryCooldown::new(RETRY);
        cooldown.failed_at(start);
        assert!(!cooldown.retry_ready_at(start));

        cooldown.succeeded();

        assert!(cooldown.retry_ready_at(start));
    }

    #[test]
    fn cooldown_allows_only_one_concurrent_retry() {
        let start = std::time::Instant::now();
        let cooldown = super::RetryCooldown::new(RETRY);
        cooldown.failed_at(start);

        assert!(cooldown.claim_retry_at(start + RETRY));
        assert!(
            !cooldown.claim_retry_at(start + RETRY),
            "a claimed retry excludes another caller until its outcome is recorded"
        );
    }

    #[test]
    fn a_concurrent_first_ask_waits_for_the_real_answer() {
        // A spurious "unavailable" would drop the caller to the next backend in precedence, so
        // the ask that arrives while the very first probe is still running must block, not guess.
        let (probing, arrived) = mpsc::channel();
        let cache = LatchingAvailability::new(RETRY);

        thread::scope(|scope| {
            let first = scope.spawn(|| {
                cache.available(|| {
                    probing.send(()).expect("second ask is scoped alongside");
                    true
                })
            });
            arrived.recv().expect("first ask reaches its probe");
            let second = scope.spawn(|| cache.available(|| false));

            assert!(first.join().expect("first ask does not panic"));
            assert!(
                second.join().expect("second ask does not panic"),
                "the second ask sees the probed answer, never a placeholder negative"
            );
        });
    }
}
