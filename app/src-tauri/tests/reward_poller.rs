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

use app_lib::{PollerTiming, VisualRewardSource, spawn_reward_screen_poller_with};
use warframe_acquisition::RewardCatalogEntry;

mod common;

use common::isolate_debug_log;

/// Fast everywhere, so a test measures behaviour rather than wall-clock.
fn timing() -> PollerTiming {
    PollerTiming {
        interval: Duration::from_millis(1),
        watch_interval: Duration::from_millis(1),
        lifetime: Duration::from_secs(5),
    }
}

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
    [
        "Athodai Prime Blueprint",
        "Styanax Prime Chassis Blueprint",
        "Bronco Prime Barrel",
        "Gyre Prime Systems Blueprint",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

/// One reward screen's whole life: absent, then up for a while, then gone.
///
/// The poller arms minutes early and outlives the screen, so both ends matter -- it has to survive
/// a long absence before the cards, and notice the absence after them.
struct ScriptedScreen {
    absent_before: usize,
    showings: usize,
    calls: Arc<AtomicUsize>,
}

impl VisualRewardSource for ScriptedScreen {
    fn choices(&mut self, _candidates: &[RewardCatalogEntry]) -> Result<Vec<String>, &'static str> {
        self.calls.fetch_add(1, Ordering::AcqRel);
        if self.absent_before > 0 {
            self.absent_before -= 1;
            return Err("no Warframe window found");
        }
        if self.showings > 0 {
            self.showings -= 1;
            return Ok(cards());
        }
        Err("a reward card read as blank")
    }
}

struct Outcome {
    names: Option<Vec<String>>,
    gone: bool,
    calls: usize,
}

fn run(absent_before: usize, showings: usize) -> Outcome {
    isolate_debug_log();
    let reads = Arc::new(Mutex::new(None));
    let polling = Arc::new(AtomicBool::new(false));
    let gone = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let handle = spawn_reward_screen_poller_with(pool(), &reads, &polling, &gone, timing(), {
        let calls = Arc::clone(&calls);
        move || ScriptedScreen {
            absent_before,
            showings,
            calls,
        }
    })
    .expect("poller declined to arm");
    handle.join().expect("poller thread panicked");
    Outcome {
        names: reads.lock().unwrap().clone(),
        gone: gone.load(Ordering::Acquire),
        calls: calls.load(Ordering::Acquire),
    }
}

/// The whole point of the poller: keep looking until the screen appears. A fissure runs for
/// minutes before the cards render, so a poller that gives up after one miss never sees them.
#[test]
fn the_poller_keeps_reading_until_the_cards_appear() {
    let outcome = run(25, 4);
    assert_eq!(outcome.names.as_deref(), Some(cards().as_slice()));
    assert!(outcome.calls > 25, "gave up before the screen appeared");
}

/// A read on the very first poll has to be published too -- the sleep must not come first, or the
/// poller is blind for its first interval.
#[test]
fn the_first_poll_happens_immediately() {
    let outcome = run(0, 4);
    assert_eq!(outcome.names.as_deref(), Some(cards().as_slice()));
}

/// The reason the overlay used to linger: the shutdown line in EE.log arrives with the same flush
/// delay as everything else, so the overlay stayed up for seconds after the screen had gone.
/// Seeing the screen disappear is what takes it down now.
#[test]
fn the_poller_reports_the_screen_going_away() {
    let outcome = run(2, 4);
    assert_eq!(outcome.names.as_deref(), Some(cards().as_slice()));
    assert!(outcome.gone, "never noticed the screen close");
}

/// A screen that is still up must not be reported as gone. Cards read blank often enough
/// mid-screen that a single miss would take the overlay down while the player is still looking.
#[test]
fn one_blank_read_mid_screen_does_not_close_the_overlay() {
    struct Flickering {
        polls: Arc<AtomicUsize>,
        gone_after: usize,
    }
    impl VisualRewardSource for Flickering {
        fn choices(
            &mut self,
            _candidates: &[RewardCatalogEntry],
        ) -> Result<Vec<String>, &'static str> {
            let poll = self.polls.fetch_add(1, Ordering::AcqRel) + 1;
            if poll > self.gone_after {
                return Err("a reward card read as blank");
            }
            // Every third read comes back blank while the screen is still up.
            if poll.is_multiple_of(3) {
                return Err("a reward card read as blank");
            }
            Ok(cards())
        }
    }

    isolate_debug_log();
    let reads = Arc::new(Mutex::new(None));
    let polling = Arc::new(AtomicBool::new(false));
    let gone = Arc::new(AtomicBool::new(false));
    let polls = Arc::new(AtomicUsize::new(0));
    const GONE_AFTER: usize = 12;
    let handle = spawn_reward_screen_poller_with(pool(), &reads, &polling, &gone, timing(), {
        let polls = Arc::clone(&polls);
        move || Flickering {
            polls,
            gone_after: GONE_AFTER,
        }
    })
    .expect("poller declined to arm");

    handle.join().expect("poller thread panicked");
    assert_eq!(reads.lock().unwrap().as_deref(), Some(cards().as_slice()));
    assert!(
        gone.load(Ordering::Acquire),
        "should still close once the screen really goes"
    );
    // The discriminating part: a single blank must not have ended it. Closing on the first miss
    // would stop at the third poll, while the screen was still up.
    assert!(
        polls.load(Ordering::Acquire) > GONE_AFTER,
        "closed the overlay while the screen was still up"
    );
}

/// Arming twice must not start a second thread. Relic loads are logged one per squad member, so
/// `BaselineRequested` fires several times within milliseconds on a real run.
#[test]
fn arming_twice_starts_only_one_poller() {
    isolate_debug_log();
    let reads = Arc::new(Mutex::new(None));
    let polling = Arc::new(AtomicBool::new(false));
    let gone = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));

    let first = spawn_reward_screen_poller_with(pool(), &reads, &polling, &gone, timing(), {
        let calls = Arc::clone(&calls);
        move || ScriptedScreen {
            absent_before: 3,
            showings: 2,
            calls,
        }
    });
    let second = spawn_reward_screen_poller_with(pool(), &reads, &polling, &gone, timing(), {
        let calls = Arc::clone(&calls);
        move || ScriptedScreen {
            absent_before: 3,
            showings: 2,
            calls,
        }
    });

    assert!(first.is_some(), "the first arm should start a poller");
    assert!(
        second.is_none(),
        "the second arm started a duplicate poller"
    );
    first.unwrap().join().expect("poller thread panicked");
}

/// An empty pool means the closed-set match has nothing to match against, so every read would be
/// rejected anyway. Arming would burn a capture every interval for 45 minutes for nothing.
#[test]
fn an_empty_pool_does_not_arm() {
    isolate_debug_log();
    let reads = Arc::new(Mutex::new(None));
    let polling = Arc::new(AtomicBool::new(false));
    let gone = Arc::new(AtomicBool::new(false));
    let handle =
        spawn_reward_screen_poller_with(Vec::new(), &reads, &polling, &gone, timing(), || {
            ScriptedScreen {
                absent_before: 0,
                showings: 1,
                calls: Arc::new(AtomicUsize::new(0)),
            }
        });
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
    let gone = Arc::new(AtomicBool::new(false));
    let calls = Arc::new(AtomicUsize::new(0));
    let handle = spawn_reward_screen_poller_with(
        pool(),
        &reads,
        &polling,
        &gone,
        PollerTiming {
            interval: Duration::from_millis(20),
            watch_interval: Duration::from_millis(20),
            lifetime: Duration::from_secs(5),
        },
        {
            let calls = Arc::clone(&calls);
            move || ScriptedScreen {
                absent_before: usize::MAX,
                showings: 0,
                calls,
            }
        },
    )
    .expect("poller declined to arm");

    polling.store(false, Ordering::Release);
    handle.join().expect("poller thread panicked");
    assert!(reads.lock().unwrap().is_none());
}
