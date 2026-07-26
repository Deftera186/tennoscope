use app_lib::best_match;
use warframe_acquisition::RewardCatalogEntry;

mod common;

fn pool() -> Vec<RewardCatalogEntry> {
    // The relic pool from the labelled 2026-07-26 run whose screen produced the reads below.
    [
        ("Braton Prime Blueprint", 15),
        ("2X Forma Blueprint", 0),
        ("Burston Prime Stock", 15),
        ("Trumna Prime Blueprint", 15),
        ("Braton Prime Receiver", 15),
        ("Burston Prime Receiver", 15),
        ("Trumna Prime Barrel", 45),
        ("Paris Prime Lower Limb", 45),
    ]
    .into_iter()
    .map(|(name, ducats)| RewardCatalogEntry {
        name: name.to_owned(),
        ducats,
    })
    .collect()
}

/// These are the exact strings tesseract produced from the captured reward screen. The divider
/// under each card leaks into the crop and garbles the tail, which is precisely the noise the
/// closed-set match has to absorb.
#[test]
fn garbled_card_reads_still_land_on_the_right_reward() {
    let pool = pool();
    for (read, expected) in [
        ("Braton Prime Blueprint FF", "Braton Prime Blueprint"),
        ("Braton Prime Blueprint |", "Braton Prime Blueprint"),
        ("Braton Prime Blueprint A", "Braton Prime Blueprint"),
        ("2 X Forma Blueprint W\\:", "2X Forma Blueprint"),
        ("2 X Forma Blueprint e\\", "2X Forma Blueprint"),
        ("2 X Forma Blueprint Aw", "2X Forma Blueprint"),
        ("Burston Prime Stock", "Burston Prime Stock"),
        ("Trumna Prime Blueprint", "Trumna Prime Blueprint"),
    ] {
        let (name, score) = best_match(read, &pool).expect("a candidate always scores");
        assert_eq!(name, expected, "read {read:?}");
        assert!(score >= 0.6, "read {read:?} scored only {score}");
    }
}

/// Rewards that differ by one word are the case a loose match would get wrong, so they have to
/// stay separable: Braton Prime Blueprint against Braton Prime Receiver, Burston against Braton.
#[test]
fn near_identical_rewards_stay_separable() {
    let pool = pool();
    for (read, expected) in [
        ("Braton Prime Receiver", "Braton Prime Receiver"),
        ("Burston Prime Receiver", "Burston Prime Receiver"),
        ("Trumna Prime Barrel", "Trumna Prime Barrel"),
    ] {
        let (name, _) = best_match(read, &pool).expect("a candidate always scores");
        assert_eq!(name, expected, "read {read:?}");
    }
}

#[test]
fn an_empty_or_unreadable_card_matches_nothing() {
    assert!(best_match("", &pool()).is_none());
    assert!(best_match("   \n", &pool()).is_none());
}

/// A card that is not in the pool at all must score below the publish floor rather than snapping
/// to the nearest name, so a bad capture fails closed instead of showing a plausible wrong reward.
#[test]
fn text_from_outside_the_pool_scores_below_the_floor() {
    let (_, score) = best_match("Orokin Catalyst Blueprint", &pool()).expect("scores");
    assert!(score < 0.6, "unrelated text scored {score}");
}

/// `fixtures/reward-screen-1920x1080.png` is a real captured reward screen with everything outside
/// the card title band blanked, which keeps the 1920x1080 geometry exact while shrinking the file.
/// The user confirmed the cards left to right as Braton Prime Blueprint, 2 X Forma Blueprint,
/// Burston Prime Stock, Trumna Prime Blueprint.
///
/// Needs ImageMagick and tesseract, the same tools the live path shells out to.
#[test]
fn the_calibrated_geometry_reads_a_real_reward_screen() {
    common::isolate_debug_log();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reward-screen-1920x1080.png");
    assert_eq!(
        app_lib::read_cards(&fixture, &pool()).unwrap(),
        vec![
            "Braton Prime Blueprint",
            "2X Forma Blueprint",
            "Burston Prime Stock",
            "Trumna Prime Blueprint",
        ]
    );
}

/// Two readers are live at once whenever the log-triggered retry overlaps the poller, which is
/// exactly during the reward screen. They used to share one capture path and one crop path, so
/// each deleted the other's file mid-read and the reads failed precisely when they were needed.
#[test]
fn concurrent_reads_do_not_corrupt_each_other() {
    common::isolate_debug_log();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reward-screen-1920x1080.png");
    let expected = vec![
        "Braton Prime Blueprint",
        "2X Forma Blueprint",
        "Burston Prime Stock",
        "Trumna Prime Blueprint",
    ];

    let readers = (0..8)
        .map(|_| {
            let fixture = fixture.clone();
            std::thread::spawn(move || app_lib::read_cards(&fixture, &pool()))
        })
        .collect::<Vec<_>>();

    for reader in readers {
        let names = reader
            .join()
            .expect("reader panicked")
            .expect("read failed");
        assert_eq!(names, expected);
    }
}

/// Exercises the real shell-out chain against a running game: window discovery, PPM capture,
/// header parsing, cropping and tesseract. Ignored by default because it needs Warframe on screen.
///
/// Run with `cargo test -p warframe-helper --test reward_ocr -- --ignored --nocapture`. Outside a
/// reward screen the cards will not match, which is the correct answer; what is being checked is
/// that the failure is a match failure and not a broken capture.
#[test]
#[ignore = "needs a running Warframe window"]
fn live_capture_reaches_the_game_window() {
    let mut source = app_lib::ScreenRewardSource::new();
    let outcome =
        <app_lib::ScreenRewardSource as app_lib::VisualRewardSource>::choices(&mut source, &pool());
    println!("live capture outcome: {outcome:?}");
    assert_ne!(
        outcome,
        Err("no Warframe window found"),
        "window discovery failed"
    );
    assert_ne!(outcome, Err("import is not available"));
    assert_ne!(outcome, Err("magick is not available"));
    assert_ne!(outcome, Err("tesseract is not available"));
    assert_ne!(
        outcome,
        Err("capture was not a PNG or PPM"),
        "PPM header parsing failed"
    );
    assert_ne!(outcome, Err("could not capture the game window"));
}
