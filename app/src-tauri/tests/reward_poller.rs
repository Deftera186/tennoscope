//! Drive the reward-screen poller against a scripted screen.
//!
//! The poller is what makes the overlay independent of EE.log's flush delay, and until now the
//! only way to execute a single line of it was to play a fissure. Four live runs produced no
//! overlay and no way to tell "never armed" from "armed but never read" from "read but rejected",
//! because every failure path inside the loop is silent and the loop was unreachable from a test.
//!
//! These drive it with a fake screen: instant, deterministic, and red on the behaviours the live
//! runs could not distinguish.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::Duration,
};

use app_lib::{VisualRewardSource, spawn_reward_screen_poller_with};
use warframe_acquisition::RewardCatalogEntry;

mod common;

const TICK: Duration = Duration::from_millis(1);
const LIFETIME: Duration = Duration::from_secs(5);

use common::isolate_debug_log;

fn pool() -> Vec<RewardCatalogEntry> {
    ["Lex Prime Barrel", "Forma Blueprint"]
        .into_iter()
        .map(|name| RewardCatalogEntry {
            name: name.to_owned(),
            ducats: 15,
        })
        .collect()
}

fn cards() -> Vec<String> {
    ["Athodai Prime Blueprint", "Styanax Prime Chassis Blueprint", "Bronco Prime Barrel", "Gyre Prime Systems Blueprint"]
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// A screen that fails a set number of times before showing the cards, which is what the real one
/// does: the poller arms minutes early, so almost every poll happens while no reward screen exists.
struct ScriptedScreen {
    failures_remaining: usize,
    calls: Arc<AtomicUsize>,
}

impl VisualRewardSource for ScriptedScreen {
    fn choices(&mut self, _candidates: &[RewardCatalogEntry]) -> Result<Vec<String>, &'static str> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        if self.failures_remaining > 0 {
            self.failures_remaining -= 1;
            return Err("no Warframe window found");
        }
        Ok(cards())
    }
}

fn run(failures: usize) -> (Option<Vec<String>>, usize) {
    isolate_debug_log();
    let reads = Arc::new(Mutex::new(None));
    let polling = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let handle = spawn_reward_screen_poller_with(
        pool(),
        &reads,
        &polling,
        TICK,
        LIFETIME,
        {
            let calls = Arc::clone(&calls);
            move || ScriptedScreen {
                failures_remaining: failures,
                calls,
            }
        },
    )
    .expect("poller declined to arm");
    handle.join().expect("poller thread panicked");
    let names = reads.lock().unwrap().clone();
    (names, calls.load(Ordering::Acquire))
}

/// The whole point of the poller: keep looking until the screen appears. A fissure runs for
/// minutes before the cards render, so a poller that gives up after one miss never sees them.
#[test]
fn the_poller_keeps_reading_until_the_cards_appear() {
    let (names, calls) = run(25);
    assert_eq!(names.as_deref(), Some(cards().as_slice()));
    assert_eq!(calls, 26, "should have polled through every failure");
}

/// A read on the very first poll has to be published too -- the sleep must not come first, or the
/// poller is blind for its first interval.
#[test]
fn the_first_poll_happens_immediately() {
    let (names, calls) = run(0);
    assert_eq!(names.as_deref(), Some(cards().as_slice()));
    assert_eq!(calls, 1);
}

/// Arming twice must not start a second thread. Relic loads are logged one per squad member, so
/// `BaselineRequested` fires several times within milliseconds on a real run.
#[test]
fn arming_twice_starts_only_one_poller() {
    isolate_debug_log();
    let reads = Arc::new(Mutex::new(None));
    let polling = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));

    let first = spawn_reward_screen_poller_with(pool(), &reads, &polling, TICK, LIFETIME, {
        let calls = Arc::clone(&calls);
        move || ScriptedScreen {
            failures_remaining: 3,
            calls,
        }
    });
    let second = spawn_reward_screen_poller_with(pool(), &reads, &polling, TICK, LIFETIME, {
        let calls = Arc::clone(&calls);
        move || ScriptedScreen {
            failures_remaining: 3,
            calls,
        }
    });

    assert!(first.is_some(), "the first arm should start a poller");
    assert!(second.is_none(), "the second arm started a duplicate poller");
    first.unwrap().join().expect("poller thread panicked");
}

/// An empty pool means the closed-set match has nothing to match against, so every read would be
/// rejected anyway. Arming would burn a capture every interval for 45 minutes for nothing.
#[test]
fn an_empty_pool_does_not_arm() {
    isolate_debug_log();
    let reads = Arc::new(Mutex::new(None));
    let polling = Arc::new(AtomicBool::new(false));
    let handle = spawn_reward_screen_poller_with(
        Vec::new(),
        &reads,
        &polling,
        TICK,
        LIFETIME,
        || ScriptedScreen {
            failures_remaining: 0,
            calls: Arc::new(AtomicUsize::new(0)),
        },
    );
    assert!(handle.is_none(), "armed a poller with nothing to match");
    assert!(
        !polling.load(Ordering::Acquire),
        "declining to arm must leave the flag clear, or the next fissure cannot arm either"
    );
}

/// The screen shutting down stops the poller, so it does not keep capturing between fissures.
#[test]
fn clearing_the_flag_stops_the_poller() {
    isolate_debug_log();
    let reads = Arc::new(Mutex::new(None));
    let polling = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let handle = spawn_reward_screen_poller_with(
        pool(),
        &reads,
        &polling,
        Duration::from_millis(20),
        LIFETIME,
        {
            let calls = Arc::clone(&calls);
            move || ScriptedScreen {
                failures_remaining: usize::MAX,
                calls,
            }
        },
    )
    .expect("poller declined to arm");

    polling.store(false, Ordering::Release);
    handle.join().expect("poller thread panicked");
    assert!(reads.lock().unwrap().is_none());
}
