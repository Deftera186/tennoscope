use app_lib::{MAX_CARDS, reward_overlay_geometry, warframe_window_from_xwininfo_tree};

/// The overlay has to line up with the game's four reward cards, which on a 1920x1080 screen span
/// x=478 to x=1444 and bottom out at y=525 under the player-name row. It used to be sized at 75% of
/// the screen and placed at 56% of the height, which made it 1440px wide against the cards' 966 and
/// put it roughly 75px below them.
#[test]
fn reward_overlay_sits_under_the_game_reward_cards() {
    let hd = reward_overlay_geometry(1920, 1080, 0, 0, MAX_CARDS);
    assert_eq!((hd.x, hd.width), (478, 966), "must span the card block");
    assert_eq!(
        (hd.y, hd.height),
        (530, 156),
        "must sit just below the cards"
    );

    // Same screen, second monitor: the block is offset but keeps its size.
    let offset = reward_overlay_geometry(1920, 1080, 1920, 0, MAX_CARDS);
    assert_eq!((offset.x, offset.y), (2398, 530));
    assert_eq!((offset.width, offset.height), (966, 156));

    // Scaling is proportional, with no clamp to break the alignment.
    let ultrawide = reward_overlay_geometry(3440, 1440, 1920, 0, MAX_CARDS);
    assert_eq!((ultrawide.x, ultrawide.width), (2776, 1731));
    assert_eq!((ultrawide.y, ultrawide.height), (707, 208));
}

/// Warframe centres the card block on the number of cards, so a smaller squad's cards are both
/// narrower and further right. A strip pinned to the four-card block would hang half a card off the
/// left of a three-card screen -- and it stays centred on the same point whatever the count, which
/// is the property worth asserting because it is the one that survives a re-calibration.
#[test]
fn the_overlay_narrows_with_the_squad() {
    for (cards, expected) in [(4, (478, 966)), (3, (599, 724)), (2, (720, 482))] {
        let strip = reward_overlay_geometry(1920, 1080, 0, 0, cards);
        assert_eq!(
            (strip.x, strip.width),
            expected,
            "{cards} cards must span their own block"
        );
        assert_eq!(
            strip.x + strip.width as i32 / 2,
            961,
            "{cards} cards must stay centred on the block's centre"
        );
    }
}

/// The locator is what makes the overlay work off sway: `xwininfo` reads the X root window tree,
/// which every window manager has and which under Wayland holds the game's XWayland window with
/// the compositor's own layout coordinates.
///
/// The tree below is real `xwininfo -root -tree` output shape: XWayland's own 1x1 and 10x10 helper
/// windows, a Wine helper carrying the game's title, and the game itself on a second monitor.
#[test]
fn xwininfo_locator_targets_the_real_warframe_window() {
    let tree = r#"
xwininfo: Window id: 0x352 (the root window) (has no name)

  Root window id: 0x352 (the root window) (has no name)
  Parent window id: 0x0 (none)
     6 children:
     0x200005 (has no name): ()  1x1+-100+-100  +-100+-100
     0x200004 "wlroots wm": ()  10x10+0+0  +0+0
     0x1400002 "Warframe": ("Warframe" "steam_app_230410")  1x1+0+0  +0+0
     0x1400003 "Warframe": ("Warframe" "steam_app_230410")  1920x1080+1920+0  +1920+0
     0x1600001 "TennoScope": ("tennoscope" "TennoScope")  1180x760+100+100  +100+100
"#;
    let (id, rect) = warframe_window_from_xwininfo_tree(tree).unwrap();
    assert_eq!(id, "0x1400003", "the 1x1 helper must not win");
    assert_eq!(
        (rect.x, rect.y, rect.width, rect.height),
        (1920, 0, 1920, 1080),
        "must read the absolute position, not the parent-relative one"
    );
    let overlay = reward_overlay_geometry(rect.width, rect.height, rect.x, rect.y, MAX_CARDS);
    assert_eq!((overlay.x, overlay.y), (2398, 530));

    // A window left of the primary output reports a negative offset, which prints as `+-1920`.
    let left = r#"     0x1400003 "Warframe": ("Warframe" "warframe.x64.exe")  1920x1080+-1920+0  +-1920+0"#;
    let (_, rect) = warframe_window_from_xwininfo_tree(left).unwrap();
    assert_eq!((rect.x, rect.y), (-1920, 0));

    assert!(
        warframe_window_from_xwininfo_tree("     0x1600001 \"TennoScope\": ()  1180x760+0+0  +0+0")
            .is_none(),
        "no game window means no rectangle, not somebody else's"
    );
}

/// xcap replaced `xwininfo` and `import`, and its window list is a flat list of candidates rather
/// than a tree of lines -- but the selection problem is the same one, so it is the same function.
///
/// Wine spawns 1x1 helper windows carrying the game's own title, and on Windows the launcher and
/// the game share a process family, so "the first window called Warframe" is not the game. The
/// largest match is.
#[test]
fn the_game_window_is_chosen_by_size_not_by_order() {
    let candidates = [
        // A Wine helper, and on Windows a minimized window, both report as tiny.
        ("Warframe", 1, 1, 0, 0),
        ("Warframe", 1920, 1080, 1920, 0),
        ("TennoScope", 1180, 760, 100, 100),
    ];
    let chosen =
        app_lib::largest_warframe_window(candidates.iter().map(|&(title, width, height, x, y)| {
            (
                title.to_owned(),
                app_lib::WindowRect {
                    x,
                    y,
                    width,
                    height,
                },
            )
        }))
        .expect("the game window is in the list");
    assert_eq!(
        (chosen.x, chosen.y, chosen.width, chosen.height),
        (1920, 0, 1920, 1080)
    );
}

/// A window list with no game in it must come back empty rather than picking the largest of
/// whatever else is open -- the overlay would otherwise land on the user's browser.
#[test]
fn a_desktop_without_the_game_yields_no_window() {
    let candidates = [("TennoScope", 1180, 760), ("Firefox", 1920, 1080)];
    assert!(
        app_lib::largest_warframe_window(candidates.iter().map(|&(title, width, height)| {
            (
                title.to_owned(),
                app_lib::WindowRect {
                    x: 0,
                    y: 0,
                    width,
                    height,
                },
            )
        }))
        .is_none()
    );
}

/// The title match has to survive the launcher, which titles its own window "Warframe" too but at
/// a size no game window ever has, and it must not match a browser tab that merely mentions the
/// game. Exact title, largest window.
#[test]
fn only_an_exact_title_match_counts_as_the_game() {
    for title in ["Warframe - Google Chrome", "warframe", "Warframe Launcher"] {
        assert!(
            app_lib::largest_warframe_window(std::iter::once((
                title.to_owned(),
                app_lib::WindowRect {
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                },
            )))
            .is_none(),
            "{title:?} is not the game window"
        );
    }
}

/// Exclusive fullscreen is the one placement this app cannot win on Windows: the game owns the
/// display, its window never appears in the enumeration the overlay measures against, and nothing
/// short of a swapchain hook draws above it. So a missing rect is the signal, and the panel has to
/// say the one thing that fixes it rather than reporting a generic capture problem.
///
/// Linux does not share the failure -- an override-redirect window sits above a Wine fullscreen
/// game -- so the notice must stay off that platform entirely.
#[test]
fn a_missing_game_window_asks_for_borderless_only_where_that_is_the_cure() {
    assert_eq!(
        app_lib::borderless_notice(true),
        None,
        "found: nothing to say"
    );
    let missing = app_lib::borderless_notice(false);
    if cfg!(windows) {
        assert!(
            missing.is_some_and(|notice| notice.contains("Borderless")),
            "the notice must name the display mode that fixes it, got {missing:?}"
        );
    } else {
        assert_eq!(missing, None, "override-redirect already covers fullscreen");
    }
}
