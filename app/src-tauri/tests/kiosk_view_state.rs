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
