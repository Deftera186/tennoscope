use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use app_lib::monitor::{KIOSK_CLOSE_GRACE, KioskSession};
use app_lib::{KioskState, KioskView};

const MODE_LINE: &str =
    "2026/08/23_12.00 InventoryTest.lua: InventoryTest - CurrMode: Selling Prime Parts\n";
const SWF_LINE: &str = "Created /Lotus/Interface/InventoryTest.swf\n";
const POPULATE_LINE: &str = "InventoryTest.lua: PopulateGrid()\n";
/// The game's own exit line, exactly as EE.log writes it (census: once per session).
const CLOSE_LINE: &str = "75584.766 Script [Info]: InventoryTest.lua: DBG: HudVis 0\n";
/// A foreign subscription exactly as EE.log writes it when the sale-confirm popup takes input.
const POPUP_LINE: &str = "173.558 Input [Info]: Subscribing for /Lotus/Interface/ItemInfoPopup.swf with input filter /Lotus/Types/Input/ItemInfoPopupInputFilter\n";

/// A spawn hook that records the flags it was handed, so the test can play the poller: consume
/// the re-anchor request, or deliver the stop signal.
#[derive(Default)]
struct SpawnLog {
    calls: Mutex<usize>,
    reanchor: Mutex<Vec<Arc<AtomicBool>>>,
    gone: Mutex<Vec<Arc<AtomicBool>>>,
}

impl SpawnLog {
    fn hook(
        &self,
    ) -> impl Fn(u64, &Arc<AtomicBool>, &Arc<AtomicBool>) -> std::thread::JoinHandle<()> + '_ {
        move |_, reanchor, gone| {
            if let Ok(mut calls) = self.calls.lock() {
                *calls += 1;
            }
            if let Ok(mut slot) = self.reanchor.lock() {
                slot.push(Arc::clone(reanchor));
            }
            if let Ok(mut slot) = self.gone.lock() {
                slot.push(Arc::clone(gone));
            }
            std::thread::spawn(|| ())
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
/// Monitor-tick timestamps are injected so a test can sit on either side of the close grace
/// window without sleeping: `t0` for "right now", `after_grace(t0)` for "the close markers
/// stayed silent past the window".
fn after_grace(close_at: Instant) -> Instant {
    close_at + KIOSK_CLOSE_GRACE + Duration::from_millis(50)
}

/// Opening the kiosk shows the overlay and spawns exactly one poller, even when both open
/// markers (mode line and SWF creation) arrive in one batch -- the session must not double-arm
/// on the second marker.
#[test]
fn opening_spawns_one_poller_and_shows() {
    let kiosk = KioskState::new();
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let (shows, show) = tally();
    let t0 = Instant::now();
    session.observe(
        [MODE_LINE, SWF_LINE].concat().as_bytes(),
        &kiosk,
        &show,
        &spawns.hook(),
        t0,
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
    let kiosk = KioskState::new();
    let t0 = Instant::now();
    session.observe(MODE_LINE.as_bytes(), &kiosk, &noop, &spawns.hook(), t0);
    let reanchor = spawns.reanchor.lock().unwrap()[0].clone();
    assert!(reanchor.swap(false, Ordering::AcqRel), "open armed it");

    session.observe(POPULATE_LINE.as_bytes(), &kiosk, &noop, &spawns.hook(), t0);
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
    session.observe(POPULATE_LINE.as_bytes(), &kiosk, &noop, &spawns.hook(), t0);
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

    let t0 = Instant::now();
    let settled = after_grace(t0);
    session.observe(MODE_LINE.as_bytes(), &kiosk, &noop, &spawns.hook(), t0);
    kiosk.set(KioskView {
        epoch: 4,
        ..KioskView::default()
    });

    assert!(!session.take_close(&kiosk, &hide, t0), "no verdict yet");
    assert!(
        kiosk.get().is_some(),
        "a stray poll must not tear down a live session"
    );

    session.observe(CLOSE_LINE.as_bytes(), &kiosk, &noop, &spawns.hook(), t0);
    assert!(
        !session.take_close(&kiosk, &hide, t0),
        "a lone close is a maybe until the window passes"
    );
    assert!(session.take_close(&kiosk, &hide, settled));
    assert!(
        kiosk.get().is_none(),
        "the payload must not outlive the window"
    );
    assert_eq!(tally_of(&hides), 1);
    assert!(
        !session.take_close(&kiosk, &hide, settled),
        "the verdict is consumed once"
    );

    // A populate from the dead session must do nothing...
    session.observe(
        POPULATE_LINE.as_bytes(),
        &kiosk,
        &noop,
        &spawns.hook(),
        settled,
    );
    assert_eq!(spawns.spawns(), 1);
    // ...and a fresh open re-arms with fresh flags.
    session.observe(MODE_LINE.as_bytes(), &kiosk, &noop, &spawns.hook(), settled);
    assert_eq!(spawns.spawns(), 2, "the second visit arms a new poller");
}

/// The end-to-end shape of a session, in raw log bytes: the open shows the overlay and starts
/// a poller, and the game's own exit line arms the close that a later monitor tick consumes
/// once the grace window passes -- no OCR, no miss streak. This is the whole reason the log
/// owns presence: it narrates both edges within ~50ms of the player's ESC.
#[test]
fn the_logs_exit_line_takes_the_overlay_down() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let kiosk = KioskState::new();
    let (hides, hide) = tally();
    let t0 = Instant::now();
    let settled = after_grace(t0);

    session.observe(MODE_LINE.as_bytes(), &kiosk, &|| (), &spawns.hook(), t0);
    assert_eq!(spawns.spawns(), 1, "the open started a poller");

    session.observe(CLOSE_LINE.as_bytes(), &kiosk, &|| (), &spawns.hook(), t0);
    // The close arms on the log line; the monitor's ticks inside the window must not act on
    // it, and the first tick past the window consumes it -- once.
    assert!(
        !session.take_close(&kiosk, &hide, t0),
        "inside the window the overlay stays up"
    );
    assert!(
        session.take_close(&kiosk, &hide, settled),
        "the game's own exit line closed the session"
    );
    assert_eq!(tally_of(&hides), 1, "the overlay came down");
    assert!(
        !session.take_close(&kiosk, &hide, settled),
        "consumed once; nothing left to tear down"
    );
}

/// Closing is a UI edge, not a worker-join barrier. OCR may still be inside Tesseract when
/// EE.log reports `HudVis 0`; the payload and window must disappear before that worker exits,
/// and the monitor must remain free to process the game.
#[test]
fn close_hides_and_returns_before_a_blocked_poller_exits() {
    let mut session = KioskSession::new();
    let kiosk = Arc::new(KioskState::new());
    kiosk.set(KioskView::default());
    let release = Arc::new(AtomicBool::new(false));
    let release_for_worker = Arc::clone(&release);
    let spawn = move |_: u64, _: &Arc<AtomicBool>, _: &Arc<AtomicBool>| {
        let release = Arc::clone(&release_for_worker);
        std::thread::spawn(move || {
            while !release.load(Ordering::Acquire) {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        })
    };
    let t0 = Instant::now();
    session.observe(MODE_LINE.as_bytes(), &kiosk, &|| (), &spawn, t0);
    session.observe(CLOSE_LINE.as_bytes(), &kiosk, &|| (), &spawn, t0);
    // The verdict the closer thread consumes is the first tick past the window: it must
    // still clear, hide, and return without joining the worker parked in OCR.
    let settled = after_grace(t0);
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let kiosk_for_close = Arc::clone(&kiosk);
    let closer = std::thread::spawn(move || {
        let hidden = AtomicBool::new(false);
        let closed = session.take_close(
            kiosk_for_close.as_ref(),
            &|| {
                hidden.store(true, Ordering::Release);
            },
            settled,
        );
        done_tx
            .send((
                closed,
                hidden.load(Ordering::Acquire),
                kiosk_for_close.get().is_none(),
            ))
            .ok();
    });
    let teardown_before_release = done_rx.recv_timeout(std::time::Duration::from_secs(1)).ok();

    release.store(true, Ordering::Release);
    closer.join().expect("close thread exits");
    assert_eq!(
        teardown_before_release,
        Some((true, true, true)),
        "close must clear, hide, and return without joining the blocked poller"
    );
}

/// And the next visit re-arms: a stop left over from the last session must never kill the new
/// poller on its first tick.
#[test]
fn a_second_visit_starts_a_fresh_poller() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let kiosk = KioskState::new();

    let t0 = Instant::now();
    let settled = after_grace(t0);
    session.observe(SWF_LINE.as_bytes(), &kiosk, &|| (), &spawns.hook(), t0);
    session.observe(CLOSE_LINE.as_bytes(), &kiosk, &|| (), &spawns.hook(), t0);
    assert!(
        session.take_close(&kiosk, &|| (), settled),
        "the first visit closed off its own exit line"
    );

    session.observe(SWF_LINE.as_bytes(), &kiosk, &|| (), &spawns.hook(), settled);
    assert_eq!(spawns.spawns(), 2, "the second visit got its own poller");

    // The fresh poller's stop flag is clear: a tick later the session is still alive...
    let gone = spawns.gone.lock().unwrap()[1].clone();
    assert!(!gone.load(Ordering::Acquire));
    // ...and the second visit closes on its own exit line too.
    let t1 = settled + Duration::from_millis(10);
    session.observe(CLOSE_LINE.as_bytes(), &kiosk, &|| (), &spawns.hook(), t1);
    assert!(session.take_close(&kiosk, &|| (), after_grace(t1)));
}

/// The monitor consuming a close must not erase the poller's stop signal.
///
/// These are two different readers of one event: the monitor tears the window down once the
/// close stays silent past the grace window, and the poller thread stops looking. When they
/// shared one consuming flag the monitor always won -- it set the flag and swapped it back
/// four lines later in the same tick, while the thread only reads it every 60-400ms and can
/// be parked inside tesseract for far longer.
/// The thread then never saw it and ran out its whole 45-minute lifetime: on 2026-08-23 every
/// visit leaked a poller that kept capturing, publishing views and streaming scroll deltas over
/// later sessions -- nineteen in one evening, two of them alive at once, double-counting the
/// scroll the overlay accumulates and drifting the chips off their tiles.
#[test]
fn a_consumed_close_still_tells_the_poller_to_stop() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let kiosk = KioskState::new();

    let t0 = Instant::now();
    session.observe(MODE_LINE.as_bytes(), &kiosk, &|| (), &spawns.hook(), t0);
    let gone = spawns.gone.lock().unwrap()[0].clone();

    session.observe(CLOSE_LINE.as_bytes(), &kiosk, &|| (), &spawns.hook(), t0);
    assert!(
        session.take_close(&kiosk, &|| (), after_grace(t0)),
        "the monitor's teardown"
    );
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

    let t0 = Instant::now();
    let settled = after_grace(t0);
    session.observe(MODE_LINE.as_bytes(), &kiosk, &|| (), &spawns.hook(), t0);
    let first = spawns.gone.lock().unwrap()[0].clone();
    session.observe(CLOSE_LINE.as_bytes(), &kiosk, &|| (), &spawns.hook(), t0);
    assert!(session.take_close(&kiosk, &|| (), settled));

    session.observe(
        MODE_LINE.as_bytes(),
        &kiosk,
        &|| (),
        &spawns.hook(),
        settled,
    );
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
    session.adopt_log_tail(tail.as_bytes(), &kiosk, &show, &spawns.hook());
    assert_eq!(spawns.spawns(), 1, "the session in progress got a poller");
    assert_eq!(tally_of(&shows), 1, "and the overlay came up");
    assert!(
        spawns.reanchor.lock().unwrap()[0].load(Ordering::Acquire),
        "the adopted session still reads from a full anchor"
    );

    // And it ends the ordinary way: the machine must know it is open, or the exit line for a
    // session we joined late would be ignored.
    let t0 = Instant::now();
    session.observe(CLOSE_LINE.as_bytes(), &kiosk, &show, &spawns.hook(), t0);
    assert!(
        session.take_close(&kiosk, &hide, after_grace(t0)),
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

    let kiosk = KioskState::new();
    let tail = [MODE_LINE, SWF_LINE, POPULATE_LINE, CLOSE_LINE].concat();
    session.adopt_log_tail(tail.as_bytes(), &kiosk, &show, &spawns.hook());
    assert_eq!(spawns.spawns(), 0, "that visit is over");
    assert_eq!(tally_of(&shows), 0);

    // A tail holding no kiosk markers at all is the same nothing.
    session.adopt_log_tail(
        b"82000.0 Sys [Info]: chatter\n",
        &kiosk,
        &show,
        &spawns.hook(),
    );
    assert_eq!(spawns.spawns(), 0);
}

/// Re-resolving the log path (the monitor does it whenever the pid or path moves) must not arm
/// a second poller over a session that is already running.
#[test]
fn adopting_twice_does_not_double_arm() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let (shows, show) = tally();

    let kiosk = KioskState::new();
    let tail = [MODE_LINE, SWF_LINE].concat();
    session.adopt_log_tail(tail.as_bytes(), &kiosk, &show, &spawns.hook());
    session.adopt_log_tail(tail.as_bytes(), &kiosk, &show, &spawns.hook());
    assert_eq!(spawns.spawns(), 1);
    assert_eq!(tally_of(&shows), 1);
}

/// A close and the next open landing in ONE batch of log bytes is the same visit continuing.
///
/// EE.log reaches the monitor in chunks, so the sale-confirm rebuild -- close markers, then
/// the open markers again -- can land in a single `observe` call. Within one batch a rebuild
/// is indistinguishable from an instant close-and-reopen, and continuity is the safe side of
/// that ambiguity: nothing retires, the poller keeps reading, the published view stays up,
/// and the overlay never blinks. (The old code retired on the close and re-armed on the open,
/// wiping the view and churning the poller on every sale.)
#[test]
fn a_close_and_reopen_in_one_batch_keeps_the_same_visit() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let (shows, show) = tally();
    let kiosk = KioskState::new();
    let (hides, hide) = tally();
    let t0 = Instant::now();

    session.observe(MODE_LINE.as_bytes(), &kiosk, &show, &spawns.hook(), t0);
    let poller = spawns.gone.lock().unwrap()[0].clone();
    kiosk.set(KioskView {
        epoch: 41,
        ..KioskView::default()
    });
    let visit = kiosk.active_session();

    session.observe(
        [CLOSE_LINE, MODE_LINE, POPULATE_LINE].concat().as_bytes(),
        &kiosk,
        &show,
        &spawns.hook(),
        t0,
    );
    assert_eq!(spawns.spawns(), 1, "no second poller for the same visit");
    assert!(
        !poller.load(Ordering::Acquire),
        "the running poller was never stopped"
    );
    assert_eq!(kiosk.active_session(), visit, "still the same visit");
    assert!(
        kiosk.get().is_some(),
        "the published view stayed up through the rebuild"
    );

    assert!(
        !session.take_close(&kiosk, &hide, t0),
        "the kiosk is on screen: nothing to tear down"
    );
    assert!(
        !session.take_close(&kiosk, &hide, after_grace(t0)),
        "the cancelled teardown stays cancelled"
    );
    assert_eq!(tally_of(&hides), 0, "the overlay never blinked");
    assert_eq!(tally_of(&shows), 1, "never re-shown: it never came down");
}

/// The same batch ending on a close still tears the overlay down -- the teardown only waits
/// for the grace window, and a batch whose last word is a close leaves one pending. (The
/// middle open cancels the first close's deadline and the last close re-arms it; no second
/// poller ever spawns, so there is no short-lived visit to stop.)
#[test]
fn a_batch_that_ends_closed_still_tears_down() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let kiosk = KioskState::new();
    let (hides, hide) = tally();
    let t0 = Instant::now();

    session.observe(MODE_LINE.as_bytes(), &kiosk, &|| (), &spawns.hook(), t0);
    kiosk.set(KioskView::default());
    session.observe(
        [CLOSE_LINE, MODE_LINE, CLOSE_LINE].concat().as_bytes(),
        &kiosk,
        &|| (),
        &spawns.hook(),
        t0,
    );
    assert_eq!(spawns.spawns(), 1, "the visit never re-armed");
    assert!(
        !session.take_close(&kiosk, &hide, t0),
        "inside the window the overlay stays up"
    );
    assert!(
        session.take_close(&kiosk, &hide, after_grace(t0)),
        "the last word in the batch was a close"
    );
    assert_eq!(tally_of(&hides), 1);
    assert!(kiosk.get().is_none(), "and the payload went with it");
    let poller = spawns.gone.lock().unwrap()[0].clone();
    assert!(
        poller.load(Ordering::Acquire),
        "the visit's poller was stopped too"
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
    let (retirements, retire) = tally();

    let t0 = Instant::now();
    session.observe(MODE_LINE.as_bytes(), &kiosk, &noop, &spawns.hook(), t0);
    kiosk.set(KioskView::default());
    let gone = spawns.gone.lock().unwrap()[0].clone();
    session.close(&kiosk, &hide, &retire);
    assert!(kiosk.get().is_none());
    assert_eq!(tally_of(&hides), 1, "process death hides the overlay");
    assert_eq!(
        tally_of(&retirements),
        1,
        "process death retires the retained webview before a later show"
    );
    assert!(
        gone.load(Ordering::Acquire),
        "a dead game stops the poller too: there is nothing left to capture"
    );

    // Closing again is a no-op, and a later open still works.
    session.close(&kiosk, &hide, &retire);
    assert_eq!(tally_of(&hides), 1);
    assert_eq!(tally_of(&retirements), 1);
    session.observe(MODE_LINE.as_bytes(), &kiosk, &noop, &spawns.hook(), t0);
    assert_eq!(spawns.spawns(), 2);
}

/// Portal capture is shared by reward and kiosk readers. Process teardown may close it only
/// after the kiosk reader has observed its stop flag and returned from any in-flight capture.
#[test]
fn process_death_joins_the_kiosk_poller_before_portal_teardown() {
    let session = &mut KioskSession::new();
    let kiosk = KioskState::new();
    let order = Arc::new(Mutex::new(Vec::new()));
    let spawn = {
        let order = Arc::clone(&order);
        move |_: u64, _reanchor: &Arc<AtomicBool>, gone: &Arc<AtomicBool>| {
            let gone = Arc::clone(gone);
            let order = Arc::clone(&order);
            std::thread::spawn(move || {
                while !gone.load(Ordering::Acquire) {
                    std::thread::yield_now();
                }
                order.lock().unwrap().push("poller stopped");
            })
        }
    };

    session.observe(MODE_LINE.as_bytes(), &kiosk, &|| (), &spawn, Instant::now());
    session.close(&kiosk, &|| (), &|| ());
    order.lock().unwrap().push("portal closed");

    assert_eq!(
        order.lock().unwrap().as_slice(),
        ["poller stopped", "portal closed"]
    );
}

/// A sale confirm rebuilds the kiosk screen mid-visit: EE.log shows `Saving profile`, then
/// `HudVis 0` plus a foreign subscription, then the full open markers and `PopulateGrid()`
/// about a second later -- the player never leaves. Those transient close markers must not
/// take the overlay down when the rebuild follows in later batches.
#[test]
fn a_sale_confirm_rebuild_split_across_batches_keeps_the_overlay_up() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let kiosk = KioskState::new();
    let (hides, hide) = tally();
    let (shows, show) = tally();

    let t0 = Instant::now();
    // The rebuild lands ~1s after the close markers, inside the window.
    let rebuilding = t0 + Duration::from_millis(1000);
    session.observe(MODE_LINE.as_bytes(), &kiosk, &show, &spawns.hook(), t0);
    kiosk.set(KioskView {
        epoch: 7,
        ..KioskView::default()
    });
    let visit = kiosk.active_session();

    // The profile save and the transient close markers land in one batch...
    session.observe(
        [CLOSE_LINE, POPUP_LINE].concat().as_bytes(),
        &kiosk,
        &show,
        &spawns.hook(),
        t0,
    );
    // ...and the monitor ticks that follow must not tear down: a lone close is a
    // maybe-rebuild, not a verdict.
    assert!(
        !session.take_close(&kiosk, &hide, rebuilding),
        "the overlay must ride out the rebuild window"
    );
    assert_eq!(tally_of(&hides), 0, "the window never blinked");
    assert!(kiosk.get().is_some(), "the published view stays up");
    assert_eq!(spawns.spawns(), 1, "no second poller for the same visit");
    assert_eq!(kiosk.active_session(), visit, "the visit is continuous");

    // ...then the rebuild markers arrive a second later and the visit continues.
    session.observe(
        [MODE_LINE, SWF_LINE, POPULATE_LINE].concat().as_bytes(),
        &kiosk,
        &show,
        &spawns.hook(),
        rebuilding,
    );
    assert!(
        !session.take_close(&kiosk, &hide, rebuilding),
        "the rebuild cancels the pending teardown"
    );
    assert_eq!(tally_of(&hides), 0, "still no blink");
    assert_eq!(spawns.spawns(), 1, "the running poller was never stopped");
    assert_eq!(kiosk.active_session(), visit, "still the same visit");
    assert_eq!(tally_of(&shows), 1, "never re-shown: it never came down");
}

/// A close with no rebuild after it still takes the overlay down: the grace window delays
/// the verdict, it does not cancel it. This is the genuine-exit half of the flap pair above.
#[test]
fn silence_past_the_grace_window_tears_the_session_down() {
    let session = &mut KioskSession::new();
    let spawns = SpawnLog::default();
    let kiosk = KioskState::new();
    let (hides, hide) = tally();
    let t0 = Instant::now();

    session.observe(MODE_LINE.as_bytes(), &kiosk, &|| (), &spawns.hook(), t0);
    kiosk.set(KioskView::default());
    let poller = spawns.gone.lock().unwrap()[0].clone();
    session.observe(CLOSE_LINE.as_bytes(), &kiosk, &|| (), &spawns.hook(), t0);

    assert!(
        !session.take_close(&kiosk, &hide, t0),
        "inside the window the overlay lingers"
    );
    assert_eq!(tally_of(&hides), 0);
    assert!(kiosk.get().is_some(), "and so does the published view");
    assert!(
        session.take_close(&kiosk, &hide, after_grace(t0)),
        "silence past the window is the verdict"
    );
    assert_eq!(tally_of(&hides), 1, "the overlay came down");
    assert!(kiosk.get().is_none(), "and the payload went with it");
    assert!(
        poller.load(Ordering::Acquire),
        "the visit's poller was stopped"
    );
    assert!(
        !session.take_close(&kiosk, &hide, after_grace(t0)),
        "consumed once; nothing left to tear down"
    );
}
