use app_lib::best_match;
use warframe_acquisition::RewardCatalogEntry;

mod common;

fn names(cards: Vec<(String, f32)>) -> Vec<String> {
    cards.into_iter().map(|(name, _)| name).collect()
}

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
        names(app_lib::read_cards(&fixture, &pool()).unwrap()),
        vec![
            "Braton Prime Blueprint",
            "2X Forma Blueprint",
            "Burston Prime Stock",
            "Trumna Prime Blueprint",
        ]
    );
}

/// The same screen at 16:10, which is a Steam Deck's native 1280x800.
///
/// Warframe scales its HUD with window *height* and centres it horizontally, but the card geometry
/// was expressed as fractions of *width*. Those two agree at 16:9 and only at 16:9, so every
/// calibration fixture confirmed geometry that was wrong everywhere else. At 1280x800 the cards are
/// 178px wide and the width-based reader looked 36px left of slot 0 and 18px right of slot 3 -- a
/// fifth of a card -- which clipped the outer titles.
///
/// Reported from a Steam Deck AppImage: `Structured reward records were incomplete` on every poll
/// for the whole life of the screen, because one unmatched card fails the entire read.
///
/// The fixture is the 1920x1080 capture scaled by height and letterboxed, which is what the game
/// itself draws: same HUD, same height scaling, narrower window.
#[test]
fn a_16_10_screen_is_read_where_a_16_10_screen_actually_sits() {
    common::isolate_debug_log();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reward-screen-1280x800.png");
    let cards = app_lib::read_cards(&fixture, &pool()).unwrap();
    assert_eq!(
        names(cards.clone()),
        vec![
            "Braton Prime Blueprint",
            "2X Forma Blueprint",
            "Burston Prime Stock",
            "Trumna Prime Blueprint",
        ]
    );
    // Same reasoning as the three-card test: the closed-set match lands on the right name even from
    // a half-clipped crop, so only the score proves the geometry. Under the width-based geometry
    // the outer two cards read 0.85 and 0.75 here.
    for (name, score) in cards {
        assert!(score >= 0.9, "{name} read at only {score}");
    }
}

/// A squad of three opens three relics, and Warframe centres the card block on however many cards
/// there are -- so every card shifts right by half a card pitch, 121px at 1920. Reading a three-card
/// screen at the four-card positions puts slot 0's crop across the gutter and the left part of the
/// first title.
///
/// The live run of 2026-07-28 is exactly this. Slot 0 read `"Lavos Prim"` -- the left 98px of a
/// `Lavos Prime Blueprint` that had moved right -- which scored 0.47 against the 0.6 floor and threw
/// the whole screen away, on every poll, for the screen's entire life. No cards meant no advisor and
/// no overlay.
///
/// `fixtures/reward-screen-three-cards.png` is the four-card capture's own title strips re-laid at
/// the three-card positions, so the pixels being read are real ones.
#[test]
fn a_three_card_screen_is_read_where_a_three_card_screen_actually_sits() {
    common::isolate_debug_log();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reward-screen-three-cards.png");
    let cards = app_lib::read_cards(&fixture, &pool()).unwrap();
    assert_eq!(
        names(cards.clone()),
        vec![
            "Braton Prime Blueprint",
            "2X Forma Blueprint",
            "Burston Prime Stock",
        ]
    );
    // A clipped title still lands on the right name through the closed-set match, so the name alone
    // would pass against a crop that is half off the card. The score is what proves the geometry.
    //
    // 0.85 is the middle of a measured gap, not a tuned peak. All three cards read exactly here on
    // tesseract 5.5.2, and every card of every other fixture does too bar the wrapped screen's
    // speck at 0.954 -- but CI's tesseract 5.3.x differs by one character on `Burston Prime Stock`,
    // and that name is 17 normalised characters, so one edit is already 0.944 and two are 0.895.
    // 0.95 had room for neither.
    //
    // The other side of the gap is what misplaced geometry actually reads. Sweeping the crop
    // sideways from the true position: 1.0 out to +-16px, 0.90 at +-32px, 0.71-0.80 at +-48px,
    // 0.47-0.65 at +-64px, and the half-pitch 121px shift this test exists to catch reads 0.26-0.50
    // across its three slots. So 0.85 still fails any drift of a fifth of a card width or more, at
    // 0.35 clear of the failure it guards, while absorbing a two-character difference between OCR
    // builds. It is also the line `CROP_KEEP_BELOW` already draws around an anomalous read.
    for (name, score) in cards {
        assert!(score >= 0.85, "{name} read at only {score}");
    }
}

/// The four-card screen must keep reading as four. The layouts overlap -- a two-card block sits
/// exactly where a four-card block's middle two cards sit -- so a reader that tries counts in the
/// wrong order would report a four-card screen as two cards and drop half the rewards.
#[test]
fn a_four_card_screen_is_not_mistaken_for_a_narrower_one() {
    common::isolate_debug_log();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reward-screen-1920x1080.png");
    assert_eq!(app_lib::read_cards(&fixture, &pool()).unwrap().len(), 4);
}

/// Trying narrower layouts must not turn a pool gap into a half-answer. A squadmate's relic that
/// has not finished loading leaves its reward out of the pool, and that has happened live -- so a
/// four-card screen whose first card is unmatchable still has its middle two sitting exactly on the
/// two-card positions, where they read perfectly.
///
/// Publishing those two as the whole screen would advise on half a screen while looking certain.
/// No answer is the right answer here, and it is what this did before it had layouts to choose
/// between.
#[test]
fn a_reward_missing_from_the_pool_still_fails_closed() {
    common::isolate_debug_log();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reward-screen-1920x1080.png");
    let gap = pool()
        .into_iter()
        .filter(|entry| ["2X Forma Blueprint", "Burston Prime Stock"].contains(&&*entry.name))
        .collect::<Vec<_>>();
    assert!(
        app_lib::read_cards(&fixture, &gap).is_err(),
        "read two of four cards as a whole two-card screen"
    );
}

/// `fixtures/reward-crop-ornament.png` is a real prepared crop kept by the live run of 2026-07-28 --
/// the exact bytes tesseract was handed, so no reconstruction stands between the test and the bug.
///
/// The band reserves room above the title for a second line, and on this one-line title the game
/// had drawn something in that empty room. Under `--psm 6` tesseract returned the speck *instead of*
/// the title -- `"| @\nn |\n|"` -- so the card matched nothing and the whole screen was thrown away.
/// It failed that way on every poll for about nine seconds of a fifteen-second screen, then read
/// perfectly once the speck went. The overlay was not slow; it was blocked.
#[test]
fn an_ornament_above_the_title_does_not_swallow_the_read() {
    let crop = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reward-crop-ornament.png");
    let text = app_lib::ocr_crop(&crop).expect("tesseract is not available");
    let (name, score) = best_match(&text, &live_pool()).expect("a candidate always scores");
    assert_eq!(name, "Dual Zoren Prime Handle", "read {text:?}");
    assert!(score >= 0.95, "read {text:?} scored only {score}");
}

/// The pool of the run that produced the ornament crop, taken from its own trace. Rewards close
/// enough to be confusable are in deliberately.
fn live_pool() -> Vec<RewardCatalogEntry> {
    [
        ("Dual Zoren Prime Handle", 15),
        ("Braton Prime Receiver", 15),
        ("Gyre Prime Blueprint", 15),
        ("Valkyr Prime Blueprint", 15),
        ("Quassus Prime Blade", 45),
        ("Venato Prime Blueprint", 15),
        ("Forma Blueprint", 0),
    ]
    .into_iter()
    .map(|(name, ducats)| RewardCatalogEntry {
        name: name.to_owned(),
        ducats,
    })
    .collect()
}

/// The relic pool of the host run of 2026-07-27, whose screen is the wrapped-title fixture. The
/// two Caliban parts are in deliberately: a clipped read of the wrapped card still has to pick the
/// chassis blueprint over the bare blueprint.
fn wrapped_pool() -> Vec<RewardCatalogEntry> {
    [
        ("Caliban Prime Chassis Blueprint", 15),
        ("Caliban Prime Blueprint", 15),
        ("Caliban Prime Neuroptics Blueprint", 15),
        ("Bronco Prime Receiver", 15),
        ("Bronco Prime Barrel", 15),
        ("Forma Blueprint", 0),
    ]
    .into_iter()
    .map(|(name, ducats)| RewardCatalogEntry {
        name: name.to_owned(),
        ducats,
    })
    .collect()
}

/// `fixtures/reward-screen-wrapped-title.png` is a real captured reward screen from the host run of
/// 2026-07-27, masked to the title band the same way the fixture above is. Slot 0 is "Caliban Prime
/// Chassis Blueprint", long enough to wrap onto two lines.
///
/// The title box used to start below that first line's ascenders and end inside the divider below
/// the card, so this screen read as "Caliban Flime Gnassis Blueprint 4". Note what that does *not*
/// do: it does not fail. The closed-set match absorbed the damage and still returned the right
/// name, at 0.83 against a floor of 0.6, which is why five live runs went by without the geometry
/// being questioned. The score is the only thing that moves, so the score is what is asserted.
#[test]
fn a_title_that_wraps_to_two_lines_is_not_clipped() {
    common::isolate_debug_log();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reward-screen-wrapped-title.png");
    let cards = app_lib::read_cards(&fixture, &wrapped_pool()).unwrap();
    assert_eq!(
        names(cards.clone()),
        vec![
            "Caliban Prime Chassis Blueprint",
            "Bronco Prime Receiver",
            "Forma Blueprint",
            "Caliban Prime Blueprint",
        ]
    );
    // Every card on these screens reads exactly, bar one edit: sparse-text segmentation reports the
    // speck above this fixture's slot 3 as a leading `a`, which `normalise` cannot drop the way it
    // drops punctuation, so `Caliban Prime Blueprint` lands at 0.954. Reading the speck is the price
    // of never losing the title to it -- see `ocr_crop`.
    //
    // 0.94 is therefore the floor, not 0.95: it leaves a card room for that speck *and* a tesseract
    // build that differs by a character, while still sitting far above the 0.83 that the clipped
    // title box used to produce, which is the failure this assertion exists to catch.
    for (name, score) in cards {
        assert!(score >= 0.94, "{name} read at only {score}");
    }
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
        let cards = reader
            .join()
            .expect("reader panicked")
            .expect("read failed");
        assert_eq!(names(cards), expected);
    }
}

/// Exercises the real chain against a running game: window discovery, screen capture, cropping and
/// tesseract. Ignored by default because it needs Warframe on screen.
///
/// Run with `cargo test -p tennoscope --test reward_ocr -- --ignored --nocapture`. Outside a
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
    assert_ne!(outcome, Err("tesseract is not available"));
    assert_ne!(
        outcome,
        Err("could not enumerate windows"),
        "xcap found nothing"
    );
    assert_ne!(
        outcome,
        Err("the game window is not on any monitor"),
        "the window rectangle did not map to a monitor"
    );
    // The one that matters on Windows: GDI capture of a D3D swapchain returns a black frame rather
    // than an error, so a capture that "succeeds" and reads nothing is the failure to look for.
    assert_ne!(outcome, Err("could not capture the game window"));
}

/// The preprocess that isolates the title from the card art used to be an ImageMagick invocation.
/// It is Rust now, and these are the properties it had to keep, stated as pixels rather than as
/// "it still reads the fixtures" -- the fixture reads above already cover that end to end, but they
/// cannot say *which* step drifted when one of them fails.
///
/// `-colorspace gray` in ImageMagick 7 is a Rec.709 weighted sum of the gamma-encoded bytes, which
/// is not what `image`'s `to_luma8` computes (that is Rec.601): red is 54 not 76. Getting this
/// wrong shifts every pixel of near-white text against dark art by enough to move the threshold.
#[test]
fn luma_matches_the_imagemagick_weighting_it_replaced() {
    assert_eq!(app_lib::luma(255, 0, 0), 54);
    assert_eq!(app_lib::luma(0, 255, 0), 182);
    assert_eq!(app_lib::luma(0, 0, 255), 18);
    assert_eq!(app_lib::luma(255, 255, 255), 255);
    assert_eq!(app_lib::luma(0, 0, 0), 0);
}

/// `-normalize` is `-contrast-stretch 2%x1%`: it discards the darkest 2% and brightest 1% of the
/// histogram before stretching, rather than mapping the literal min and max. That clipping is the
/// whole point -- a single stray white pixel in a crop would otherwise pin the top of the range and
/// leave the actual text well below the threshold.
#[test]
fn normalize_clips_the_histogram_tails_before_stretching() {
    // 1000 pixels: 5 black, 990 spread over 100..=150, 5 white. The tails are inside the clip, so
    // the stretch has to run on 100..=150 and ignore the outliers entirely.
    let mut pixels = vec![0_u8; 5];
    pixels.extend((0..990).map(|index| 100 + (index * 50 / 989) as u8));
    pixels.extend([255_u8; 5]);
    let stretched = app_lib::normalize_contrast(&pixels);
    assert_eq!(stretched[0], 0, "clipped darks stay at the bottom");
    assert_eq!(stretched[999], 255, "clipped lights stay at the top");
    assert_eq!(stretched[5], 0, "the darkest unclipped value maps to 0");
    assert_eq!(
        stretched[994], 255,
        "the brightest unclipped value maps to 255"
    );
    let middle = stretched[500];
    assert!(
        (120..=136).contains(&middle),
        "the midpoint should land near mid-grey, not at {middle}"
    );
}

/// A flat crop has no range to stretch. ImageMagick leaves it alone rather than dividing by zero,
/// and so must this -- a divide by zero here would be a panic on a black frame, which is exactly
/// what a capture taken a moment too early looks like.
#[test]
fn normalize_leaves_a_flat_image_alone() {
    assert_eq!(app_lib::normalize_contrast(&[128; 64]), vec![128; 64]);
    assert_eq!(app_lib::normalize_contrast(&[]), Vec::<u8>::new());
}

/// `-threshold 74%` keeps everything strictly above 74% of full scale, and `-negate` follows it
/// because tesseract is trained on dark-on-light. The two are one step here.
#[test]
fn threshold_keeps_text_and_inverts_for_tesseract() {
    // 74% of 255 is 188.7, so 188 is below the cut and 189 is above it.
    assert_eq!(
        app_lib::threshold_inverted(&[188, 189, 0, 255]),
        vec![255, 0, 255, 0]
    );
}

/// Windows has no package manager to lean on, and asking a player to install Tesseract before the
/// overlay works is exactly the extra step this bundle exists to remove. So the installer ships the
/// binary next to the app and the OCR path prefers it -- but only when it is really there, because
/// a developer build has no bundled copy and must still find the one on PATH.
#[test]
fn ocr_prefers_the_bundled_tesseract_over_the_one_on_path() {
    let empty = tempfile::tempdir().expect("temp dir");
    assert_eq!(
        app_lib::tesseract_program(empty.path()),
        std::path::PathBuf::from("tesseract"),
        "with nothing bundled the OCR path must fall back to PATH"
    );

    let bundled = tempfile::tempdir().expect("temp dir");
    let directory = bundled.path().join("tesseract");
    std::fs::create_dir_all(&directory).expect("create bundle dir");
    let program = directory.join(app_lib::TESSERACT_EXECUTABLE);
    std::fs::write(&program, b"").expect("write stub");
    assert_eq!(app_lib::tesseract_program(bundled.path()), program);
}

/// A scaled desktop hands the capture back at the framebuffer's size, not the logical size the
/// window rect is quoted in. Every crop is a fraction of the frame, so a frame 1.5x too big puts
/// the title band a third of a card too low and every read comes back blank -- which from outside
/// is indistinguishable from OCR simply not working, and is the shape of the reports that survived
/// two geometry fixes.
///
/// This is the reason `capture_game_window` resamples a mismatched capture back to the region it
/// asked for: reading the oversized frame fails, reading it after the resample does not.
#[test]
fn a_capture_at_the_wrong_scale_must_be_resampled_before_it_reads() {
    common::isolate_debug_log();
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/reward-screen-1920x1080.png");
    let screen = image::open(&fixture).expect("the fixture decodes");
    let scaled = image::DynamicImage::ImageRgba8(image::imageops::resize(
        &screen.to_rgba8(),
        2880,
        1620,
        image::imageops::FilterType::Lanczos3,
    ));
    // What the live path does with an oversized capture: paste it into a window-sized frame,
    // which clips rather than scales. The result is the top-left corner magnified, and the title
    // band is nowhere near where the fractions say it is. (A frame that is merely scaled reads
    // fine -- every fraction is of the frame's own height -- so it is the clip that does the
    // damage, and the clip is unavoidable once the frame has to be window sized.)
    let mut clipped = image::RgbaImage::new(1920, 1080);
    image::imageops::replace(&mut clipped, &scaled.to_rgba8(), 0, 0);
    assert!(
        app_lib::read_cards_in(&image::DynamicImage::ImageRgba8(clipped), &pool()).is_err(),
        "an unresampled 1.5x capture must not read"
    );
    let restored = image::DynamicImage::ImageRgba8(image::imageops::resize(
        &scaled.to_rgba8(),
        1920,
        1080,
        image::imageops::FilterType::Lanczos3,
    ));
    assert_eq!(
        names(app_lib::read_cards_in(&restored, &pool()).unwrap()).len(),
        4
    );
}
