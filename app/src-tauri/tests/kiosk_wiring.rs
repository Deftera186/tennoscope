use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use app_lib::{KioskSession, KioskState, KioskView};

const MODE_LINE: &str =
    "2026/08/23_12.00 InventoryTest.lua: InventoryTest - CurrMode: Selling Prime Parts\n";
const SWF_LINE: &str = "Created /Lotus/Interface/InventoryTest.swf\n";
const POPULATE_LINE: &str = "InventoryTest.lua: PopulateGrid()\n";
/// The game's own exit line, exactly as EE.log writes it (census: once per session).
const CLOSE_LINE: &str = "75584.766 Script [Info]: InventoryTest.lua: DBG: HudVis 0\n";

/// A spawn hook that records the flags it was handed, so the test can play the poller: consume
/// the re-anchor request, or deliver the stop signal.
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

/// The log's close line must hide the overlay, clear the published view, consume exactly once,
/// and reset the log machine so the *next* kiosk visit -- whose open markers print fresh into
/// the growing log -- arms from scratch instead of being swallowed by the previous session's
/// `open` state.
#[test]
fn the_logs_close_line_closes_and_a_reopen_rearms() {
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

    assert!(!session.take_close(&kiosk, &hide), "no verdict yet");
    assert!(
        kiosk.get().is_some(),
        "a stray poll must not tear down a live session"
    );

    session.observe(CLOSE_LINE.as_bytes(), &noop, &spawns.hook());
    assert!(session.take_close(&kiosk, &hide));
    assert!(
        kiosk.get().is_none(),
        "the payload must not outlive the window"
    );
    assert_eq!(tally_of(&hides), 1);
    assert!(
        !session.take_close(&kiosk, &hide),
        "the verdict is consumed once"
    );

    // A populate from the dead session must do nothing...
    session.observe(POPULATE_LINE.as_bytes(), &noop, &spawns.hook());
    assert_eq!(spawns.spawns(), 1);
    // ...and a fresh open re-arms with fresh flags.
    session.observe(MODE_LINE.as_bytes(), &noop, &spawns.hook());
    assert_eq!(spawns.spawns(), 2, "the second visit arms a new poller");
}

/// The end-to-end shape of a session, in raw log bytes: the open shows the overlay and starts
/// a poller, and the game's own exit line arms the close that the very next monitor tick
/// consumes -- no OCR, no miss streak, no wait. This is the whole reason the log owns
/// presence: it narrates both edges within ~50ms of the player's ESC.
#[test]
fn the_logs_exit_line_takes_the_overlay_down() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let kiosk = KioskState::new();
    let (hides, hide) = tally();

    session.observe(MODE_LINE.as_bytes(), &|| (), &spawns.hook());
    assert_eq!(spawns.spawns(), 1, "the open started a poller");

    session.observe(CLOSE_LINE.as_bytes(), &|| (), &spawns.hook());
    // The close arms on the log line and the monitor's next tick consumes it -- once.
    assert!(
        session.take_close(&kiosk, &hide),
        "the game's own exit line closed the session"
    );
    assert_eq!(tally_of(&hides), 1, "the overlay came down");
    assert!(
        !session.take_close(&kiosk, &hide),
        "consumed once; nothing left to tear down"
    );
}

/// And the next visit re-arms: a stop left over from the last session must never kill the new
/// poller on its first tick.
#[test]
fn a_second_visit_starts_a_fresh_poller() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let kiosk = KioskState::new();

    session.observe(SWF_LINE.as_bytes(), &|| (), &spawns.hook());
    session.observe(CLOSE_LINE.as_bytes(), &|| (), &spawns.hook());
    assert!(
        session.take_close(&kiosk, &|| ()),
        "the first visit closed off its own exit line"
    );

    session.observe(SWF_LINE.as_bytes(), &|| (), &spawns.hook());
    assert_eq!(spawns.spawns(), 2, "the second visit got its own poller");

    // The fresh poller's stop flag is clear: a tick later the session is still alive...
    let gone = spawns.gone.lock().unwrap()[1].clone();
    assert!(!gone.load(Ordering::Acquire));
    // ...and the second visit closes on its own exit line too.
    session.observe(CLOSE_LINE.as_bytes(), &|| (), &spawns.hook());
    assert!(session.take_close(&kiosk, &|| ()));
}

/// The monitor consuming a close must not erase the poller's stop signal.
///
/// These are two different readers of one event: the monitor tears the window down on its next
/// tick, and the poller thread stops looking. When they shared one consuming flag the monitor
/// always won -- it set the flag and swapped it back four lines later in the same tick, while
/// the thread only reads it every 60-400ms and can be parked inside tesseract for far longer.
/// The thread then never saw it and ran out its whole 45-minute lifetime: on 2026-08-23 every
/// visit leaked a poller that kept capturing, publishing views and streaming scroll deltas over
/// later sessions -- nineteen in one evening, two of them alive at once, double-counting the
/// scroll the overlay accumulates and drifting the chips off their tiles.
#[test]
fn a_consumed_close_still_tells_the_poller_to_stop() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let kiosk = KioskState::new();

    session.observe(MODE_LINE.as_bytes(), &|| (), &spawns.hook());
    let gone = spawns.gone.lock().unwrap()[0].clone();

    session.observe(CLOSE_LINE.as_bytes(), &|| (), &spawns.hook());
    assert!(session.take_close(&kiosk, &|| ()), "the monitor's teardown");
    assert!(
        gone.load(Ordering::Acquire),
        "the poller must still be told to stop after the monitor consumed the close"
    );
}

/// A quick reopen must not revive the poller the previous visit stopped. Flags that outlived a
/// session made this a race the player can lose by clicking fast: clearing the shared stop for
/// the new poller un-stopped the old one too, if it had not happened to tick in between.
#[test]
fn a_reopen_cannot_revive_the_previous_poller() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let kiosk = KioskState::new();

    session.observe(MODE_LINE.as_bytes(), &|| (), &spawns.hook());
    let first = spawns.gone.lock().unwrap()[0].clone();
    session.observe(CLOSE_LINE.as_bytes(), &|| (), &spawns.hook());
    assert!(session.take_close(&kiosk, &|| ()));

    session.observe(MODE_LINE.as_bytes(), &|| (), &spawns.hook());
    let second = spawns.gone.lock().unwrap()[1].clone();
    assert!(
        first.load(Ordering::Acquire),
        "the first visit's poller stays stopped for good"
    );
    assert!(
        !second.load(Ordering::Acquire),
        "the new poller starts free to look"
    );
}

/// The app can be started with the kiosk already on screen. Presence is edge-triggered from
/// the log and the live tail starts at EOF -- replaying an evening of history is what produced
/// the 2026-08-22 ghost report -- so the open marker for the session in progress was never
/// seen, and the overlay stayed dark until the player closed and reopened the screen by hand.
/// Folding the log's recent tail once, at attach, recovers the state we were not there for.
#[test]
fn a_kiosk_already_open_at_attach_is_adopted() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let (shows, show) = tally();
    let kiosk = KioskState::new();
    let (hides, hide) = tally();

    let tail = [
        "82315.203 Sys [Info]: some unrelated chatter\n",
        MODE_LINE,
        SWF_LINE,
        POPULATE_LINE,
        "82315.500 Sys [Info]: more chatter while the player reads\n",
    ]
    .concat();
    session.adopt_log_tail(tail.as_bytes(), &show, &spawns.hook());
    assert_eq!(spawns.spawns(), 1, "the session in progress got a poller");
    assert_eq!(tally_of(&shows), 1, "and the overlay came up");
    assert!(
        spawns.reanchor.lock().unwrap()[0].load(Ordering::Acquire),
        "the adopted session still reads from a full anchor"
    );

    // And it ends the ordinary way: the machine must know it is open, or the exit line for a
    // session we joined late would be ignored.
    session.observe(CLOSE_LINE.as_bytes(), &show, &spawns.hook());
    assert!(
        session.take_close(&kiosk, &hide),
        "the adopted session closes on its own exit line"
    );
    assert_eq!(tally_of(&hides), 1);
}

/// A tail whose last word on the subject is a close means the player is not in the kiosk: the
/// markers in it are history, and history must not arm anything.
#[test]
fn a_tail_that_ends_closed_is_not_adopted() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let (shows, show) = tally();

    let tail = [MODE_LINE, SWF_LINE, POPULATE_LINE, CLOSE_LINE].concat();
    session.adopt_log_tail(tail.as_bytes(), &show, &spawns.hook());
    assert_eq!(spawns.spawns(), 0, "that visit is over");
    assert_eq!(tally_of(&shows), 0);

    // A tail holding no kiosk markers at all is the same nothing.
    session.adopt_log_tail(b"82000.0 Sys [Info]: chatter\n", &show, &spawns.hook());
    assert_eq!(spawns.spawns(), 0);
}

/// Re-resolving the log path (the monitor does it whenever the pid or path moves) must not arm
/// a second poller over a session that is already running.
#[test]
fn adopting_twice_does_not_double_arm() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let (shows, show) = tally();

    let tail = [MODE_LINE, SWF_LINE].concat();
    session.adopt_log_tail(tail.as_bytes(), &show, &spawns.hook());
    session.adopt_log_tail(tail.as_bytes(), &show, &spawns.hook());
    assert_eq!(spawns.spawns(), 1);
    assert_eq!(tally_of(&shows), 1);
}

/// A close and the next open landing in ONE batch of log bytes must still arm the new visit.
///
/// EE.log reaches the monitor in chunks, so closing the kiosk and reopening it inside a tick
/// puts both markers in the same `observe` call. The close only *arms* a teardown -- the window
/// work is the monitor's, on its next tick -- so the open that follows used to be dropped by
/// the `poller_active` guard meant for duplicate open markers, and the player got a kiosk with
/// no overlay at all until they closed and opened it slowly enough. The overlay is already up
/// and stays up: the reopen cancels the pending teardown instead of flashing it.
#[test]
fn a_close_and_reopen_in_one_batch_rearms_without_a_flash() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let (shows, show) = tally();
    let kiosk = KioskState::new();
    let (hides, hide) = tally();

    session.observe(MODE_LINE.as_bytes(), &show, &spawns.hook());
    let first = spawns.gone.lock().unwrap()[0].clone();

    session.observe(
        [CLOSE_LINE, MODE_LINE, POPULATE_LINE].concat().as_bytes(),
        &show,
        &spawns.hook(),
    );
    assert_eq!(spawns.spawns(), 2, "the reopen got its own poller");
    assert!(first.load(Ordering::Acquire), "the old poller was stopped");
    let second = spawns.gone.lock().unwrap()[1].clone();
    assert!(
        !second.load(Ordering::Acquire),
        "the new one is free to look"
    );
    assert!(
        spawns.reanchor.lock().unwrap()[1].load(Ordering::Acquire),
        "and it anchors its first read"
    );

    assert!(
        !session.take_close(&kiosk, &hide),
        "the kiosk is on screen: nothing to tear down"
    );
    assert_eq!(tally_of(&hides), 0, "the overlay never blinked");
    assert_eq!(tally_of(&shows), 2);
}

/// The same batch ending on a close still tears the overlay down -- the player closed the
/// kiosk, reopened it and closed it again faster than one tick.
#[test]
fn a_batch_that_ends_closed_still_tears_down() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let kiosk = KioskState::new();
    let (hides, hide) = tally();

    session.observe(MODE_LINE.as_bytes(), &|| (), &spawns.hook());
    kiosk.set(KioskView::default());
    session.observe(
        [CLOSE_LINE, MODE_LINE, CLOSE_LINE].concat().as_bytes(),
        &|| (),
        &spawns.hook(),
    );
    assert!(
        session.take_close(&kiosk, &hide),
        "the last word in the batch was a close"
    );
    assert_eq!(tally_of(&hides), 1);
    assert!(kiosk.get().is_none(), "and the payload went with it");
    let second = spawns.gone.lock().unwrap()[1].clone();
    assert!(
        second.load(Ordering::Acquire),
        "the short-lived visit's poller was stopped too"
    );
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
    let gone = spawns.gone.lock().unwrap()[0].clone();
    session.close(&kiosk, &hide);
    assert!(kiosk.get().is_none());
    assert_eq!(tally_of(&hides), 1, "process death hides the overlay");
    assert!(
        gone.load(Ordering::Acquire),
        "a dead game stops the poller too: there is nothing left to capture"
    );

    // Closing again is a no-op, and a later open still works.
    session.close(&kiosk, &hide);
    assert_eq!(tally_of(&hides), 1);
    session.observe(MODE_LINE.as_bytes(), &noop, &spawns.hook());
    assert_eq!(spawns.spawns(), 2);
}
