use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use app_lib::{KioskSession, KioskState, KioskView};

const MODE_LINE: &str =
    "2026/08/23_12.00 InventoryTest.lua: InventoryTest - CurrMode: Selling Prime Parts\n";
const SWF_LINE: &str = "Created /Lotus/Interface/InventoryTest.swf\n";
const POPULATE_LINE: &str = "InventoryTest.lua: PopulateGrid()\n";

/// A spawn hook that records the flags it was handed, so the test can play the poller: consume
/// the re-anchor request, or deliver the gone verdict.
#[derive(Default)]
struct SpawnLog {
    calls: Mutex<usize>,
    reanchor: Mutex<Vec<Arc<AtomicBool>>>,
    gone: Mutex<Vec<Arc<AtomicBool>>>,
}

impl SpawnLog {
    fn hook(&self) -> impl Fn(&Arc<AtomicBool>, &Arc<AtomicBool>) + '_ {
        move |reanchor, gone| {
            if let Ok(mut calls) = self.calls.lock() {
                *calls += 1;
            }
            if let Ok(mut slot) = self.reanchor.lock() {
                slot.push(Arc::clone(reanchor));
            }
            if let Ok(mut slot) = self.gone.lock() {
                slot.push(Arc::clone(gone));
            }
        }
    }

    fn spawns(&self) -> usize {
        self.calls.lock().map(|calls| *calls).unwrap_or(0)
    }
}

/// A counting hook: `tally()` hands out closures that bump a shared count.
fn tally() -> (Arc<Mutex<usize>>, impl Fn() + use<>) {
    let count = Arc::new(Mutex::new(0_usize));
    let bump = {
        let count = Arc::clone(&count);
        move || {
            if let Ok(mut count) = count.lock() {
                *count += 1;
            }
        }
    };
    (count, bump)
}

fn tally_of(count: &Arc<Mutex<usize>>) -> usize {
    count.lock().map(|count| *count).unwrap_or(0)
}

/// Opening the kiosk shows the overlay and spawns exactly one poller, even when both open
/// markers (mode line and SWF creation) arrive in one batch -- the session must not double-arm
/// on the second marker.
#[test]
fn opening_spawns_one_poller_and_shows() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let (shows, show) = tally();
    session.observe(
        [MODE_LINE, SWF_LINE].concat().as_bytes(),
        &show,
        &spawns.hook(),
    );
    assert_eq!(spawns.spawns(), 1, "one poller, not one per marker");
    assert_eq!(tally_of(&shows), 1, "one show");
    let reanchor = spawns.reanchor.lock().unwrap()[0].clone();
    assert!(
        reanchor.load(Ordering::Acquire),
        "the first read must be a full anchor"
    );
}

/// A repopulation (open, filter change, basket edit) asks the running poller to re-anchor; with
/// no poller running it is nothing, because the log machine ignores populate lines before the
/// first open marker anyway.
#[test]
fn populate_re_requests_the_anchor_while_polling() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let noop = || ();
    session.observe(MODE_LINE.as_bytes(), &noop, &spawns.hook());
    let reanchor = spawns.reanchor.lock().unwrap()[0].clone();
    assert!(reanchor.swap(false, Ordering::AcqRel), "open armed it");

    session.observe(POPULATE_LINE.as_bytes(), &noop, &spawns.hook());
    assert!(
        reanchor.load(Ordering::Acquire),
        "PopulateGrid must re-arm the anchor request"
    );
    assert_eq!(
        spawns.spawns(),
        1,
        "repopulation never spawns a second poller"
    );

    reanchor.store(false, Ordering::Release);
    session.observe(POPULATE_LINE.as_bytes(), &noop, &spawns.hook());
    assert!(reanchor.load(Ordering::Acquire));
}

/// The poller's miss-streak verdict is the only close signal there is: it must hide the overlay,
/// clear the published view, and reset the log machine so the *next* kiosk visit -- whose open
/// markers print fresh into the growing log -- arms from scratch instead of being swallowed by
/// the previous session's `open` state.
#[test]
fn poller_gone_closes_and_a_reopen_rearms() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let noop = || ();
    let kiosk = KioskState::new();
    let (hides, hide) = tally();

    session.observe(MODE_LINE.as_bytes(), &noop, &spawns.hook());
    kiosk.set(KioskView {
        epoch: 4,
        ..KioskView::default()
    });
    let gone = spawns.gone.lock().unwrap()[0].clone();

    assert!(!session.poller_gone(&kiosk, &hide), "no verdict yet");
    assert!(
        kiosk.get().is_some(),
        "a stray poll must not tear down a live session"
    );

    gone.store(true, Ordering::Release);
    assert!(session.poller_gone(&kiosk, &hide));
    assert!(
        kiosk.get().is_none(),
        "the payload must not outlive the window"
    );
    assert_eq!(tally_of(&hides), 1);
    assert!(
        !session.poller_gone(&kiosk, &hide),
        "the verdict is consumed once"
    );

    // A populate from the dead session must do nothing...
    session.observe(POPULATE_LINE.as_bytes(), &noop, &spawns.hook());
    assert_eq!(spawns.spawns(), 1);
    // ...and a fresh open re-arms with fresh flags.
    session.observe(MODE_LINE.as_bytes(), &noop, &spawns.hook());
    assert_eq!(spawns.spawns(), 2, "the second visit arms a new poller");
}

/// The game process dying takes the kiosk with it even though the poller may never deliver a
/// verdict (capture fails read as misses, but the process vanishing can outrun the streak).
#[test]
fn process_death_closes_the_session() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let noop = || ();
    let kiosk = KioskState::new();
    let (hides, hide) = tally();

    session.observe(MODE_LINE.as_bytes(), &noop, &spawns.hook());
    kiosk.set(KioskView::default());
    session.close(&kiosk, &hide);
    assert!(kiosk.get().is_none());
    assert_eq!(tally_of(&hides), 1, "process death hides the overlay");

    // Closing again is a no-op, and a later open still works.
    session.close(&kiosk, &hide);
    assert_eq!(tally_of(&hides), 1);
    session.observe(MODE_LINE.as_bytes(), &noop, &spawns.hook());
    assert_eq!(spawns.spawns(), 2);
}
