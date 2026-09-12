use app_lib::{KioskState, KioskView, kiosk_overlay_geometry};

/// The poller publishes whole epochs and the `/kiosk` window pulls the latest one through the
/// `get_kiosk_view` command, so the state cell only has to keep the most recent view and hand
/// back a copy: readers must never be able to mutate what another reader holds.
#[test]
fn state_holds_the_latest_view_and_hands_out_copies() {
    let state = KioskState::new();
    assert_eq!(state.get().map(|view| view.epoch), None, "starts empty");

    let first = KioskView {
        epoch: 7,
        ..KioskView::default()
    };
    state.set(first.clone());
    assert_eq!(state.get().map(|view| view.epoch), Some(7));

    let mut held = state.get().unwrap();
    held.epoch = 99;
    assert_eq!(
        state.get().map(|view| view.epoch),
        Some(7),
        "a held copy must not write back"
    );

    state.set(KioskView {
        epoch: 8,
        ..KioskView::default()
    });
    assert_eq!(state.get().map(|view| view.epoch), Some(8));
}

/// Closing the kiosk empties the cell so a stale payload can never render over whatever the game
/// drew next.
#[test]
fn clear_empties_the_state() {
    let state = KioskState::new();
    state.set(KioskView {
        epoch: 1,
        ..KioskView::default()
    });
    state.clear();
    assert!(state.get().is_none());
}

/// A retired poller can finish OCR after the player has closed and reopened the kiosk. The state
/// accepts mutations only from the active session, so an old frame cannot overwrite the new visit.
#[test]
fn a_retired_session_cannot_publish_over_a_reopened_kiosk() {
    let state = KioskState::new();
    let first = state.begin_session();
    assert!(state.set_if_current(
        first,
        KioskView {
            epoch: 3,
            ..KioskView::default()
        },
        || (),
    ));
    state.end_session(first);
    let second = state.begin_session();
    assert!(state.set_if_current(
        second,
        KioskView {
            epoch: 8,
            ..KioskView::default()
        },
        || (),
    ));

    assert!(!state.set_if_current(
        first,
        KioskView {
            epoch: 4,
            ..KioskView::default()
        },
        || (),
    ));
    assert_eq!(state.get().map(|view| view.epoch), Some(8));
}

/// A retired worker's non-view events obey the same close/reopen ordering as view publication.
#[test]
fn a_retired_session_cannot_emit_after_reopen() {
    let state = KioskState::new();
    let first = state.begin_session();
    state.end_session(first);
    let second = state.begin_session();
    let emitted = std::sync::atomic::AtomicBool::new(false);

    assert!(!state.run_if_current(first, || {
        emitted.store(true, std::sync::atomic::Ordering::Release);
    }));
    assert!(!emitted.load(std::sync::atomic::Ordering::Acquire));
    assert_eq!(state.active_session(), Some(second));
}

/// Publishing a view and announcing its session are one lifecycle edge. A close/reopen that
/// starts while the announcement is in flight must wait, or the retired id can arrive after the
/// new visit and make the frontend reject that visit's scroll events.
#[test]
fn publication_side_effect_finishes_before_a_new_session_can_begin() {
    let state = std::sync::Arc::new(KioskState::new());
    let first = state.begin_session();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let (reopen_started_tx, reopen_started_rx) = std::sync::mpsc::channel();
    let order = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));

    let publisher = {
        let state = std::sync::Arc::clone(&state);
        let order = std::sync::Arc::clone(&order);
        std::thread::spawn(move || {
            assert!(state.set_if_current(
                first,
                KioskView {
                    epoch: 4,
                    ..KioskView::default()
                },
                || {
                    entered_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    order.lock().unwrap().push("published");
                },
            ));
        })
    };
    entered_rx.recv().unwrap();

    let reopener = {
        let state = std::sync::Arc::clone(&state);
        let order = std::sync::Arc::clone(&order);
        std::thread::spawn(move || {
            reopen_started_tx.send(()).unwrap();
            state.end_session(first);
            state.begin_session();
            order.lock().unwrap().push("reopened");
        })
    };
    reopen_started_rx.recv().unwrap();
    release_tx.send(()).unwrap();
    publisher.join().unwrap();
    reopener.join().unwrap();

    assert_eq!(*order.lock().unwrap(), ["published", "reopened"]);
}

/// Unlike the reward strip, which only has to span the card block, the kiosk chips ride a
/// scrolling grid: the window must own the game's whole rect and keep following it onto any
/// monitor, because scroll-following moves DOM inside the window rather than the window itself.
#[test]
fn the_kiosk_overlay_covers_the_whole_game_window() {
    let hd = kiosk_overlay_geometry(1920, 1080, 0, 0);
    assert_eq!((hd.x, hd.y, hd.width, hd.height), (0, 0, 1920, 1080));

    let second_monitor = kiosk_overlay_geometry(1920, 1080, 1920, 0);
    assert_eq!(
        (second_monitor.x, second_monitor.y),
        (1920, 0),
        "absolute position, not parent-relative"
    );
    assert_eq!((second_monitor.width, second_monitor.height), (1920, 1080));

    let deck = kiosk_overlay_geometry(1280, 800, -1280, 0);
    assert_eq!(
        (deck.x, deck.y, deck.width, deck.height),
        (-1280, 0, 1280, 800)
    );
}
