use std::sync::atomic::AtomicBool;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, atomic::AtomicU64},
    time::Duration,
};

use app_lib::{
    LiveMemoryRewardState, MemoryRewardSource, RewardChoiceSource, RewardSourceCoordinator,
    RewardSourceDiagnostic, VisualRewardSource, reward_path_matches, rotate_choices_to_local,
    scan_player_record_until_ready, store_player_record_if_current,
};

mod common;

#[test]
fn memory_reward_ring_rotates_without_scrambling_screen_order() {
    let mut choices = vec![
        "Caliban Prime Chassis Blueprint".into(),
        "Paris Prime Lower Limb".into(),
        "Burston Prime Blueprint".into(),
        "Nautilus Prime Carapace".into(),
    ];

    rotate_choices_to_local(&mut choices, "Paris Prime Lower Limb");

    assert_eq!(
        choices,
        vec![
            "Paris Prime Lower Limb",
            "Burston Prime Blueprint",
            "Nautilus Prime Carapace",
            "Caliban Prime Chassis Blueprint",
        ]
    );
}
use warframe_acquisition::{
    RewardCatalogEntry, RewardMemoryScanner, RewardNeedle, RewardResolution,
};

struct Memory {
    resolution: RewardResolution,
    record_resolution: RewardResolution,
    baselines: usize,
    choices: usize,
    record_choices: usize,
}

impl MemoryRewardSource for Memory {
    fn baseline(&mut self, _candidates: &[RewardNeedle]) {
        self.baselines += 1;
    }

    fn choices(&mut self, _expected: usize) -> RewardResolution {
        self.choices += 1;
        self.resolution.clone()
    }

    fn player_records(
        &mut self,
        _responders: &[&str],
        _local_identity: Option<&str>,
        _local_choice: Option<&str>,
    ) -> RewardResolution {
        self.record_choices += 1;
        self.record_resolution.clone()
    }
}

struct Visual {
    names: Result<Vec<String>, &'static str>,
    calls: usize,
}

impl VisualRewardSource for Visual {
    fn choices(&mut self, _candidates: &[RewardCatalogEntry]) -> Result<Vec<String>, &'static str> {
        self.calls += 1;
        self.names.clone()
    }
}

fn candidates() -> Vec<RewardNeedle> {
    // Every test that reaches an instrumented path builds its inputs through these two helpers, so
    // redirecting the debug log here covers the file without a line in each test.
    common::isolate_debug_log();
    vec![RewardNeedle::new("A", ["/Lotus/A"]).unwrap()]
}

fn catalog() -> Vec<RewardCatalogEntry> {
    common::isolate_debug_log();
    ["A", "B", "C", "D"]
        .into_iter()
        .map(|name| RewardCatalogEntry {
            name: name.into(),
            ducats: 0,
        })
        .collect()
}

#[test]
fn confirmed_memory_wins_without_invoking_ocr() {
    let mut memory = Memory {
        resolution: RewardResolution::Confirmed {
            choices: vec!["A".into(), "B".into(), "C".into(), "D".into()],
            region_start: 10,
        },
        record_resolution: RewardResolution::Incomplete,
        baselines: 0,
        choices: 0,
        record_choices: 0,
    };
    let mut visual = Visual {
        names: Ok(vec![]),
        calls: 0,
    };
    let mut coordinator = RewardSourceCoordinator::new(false);

    coordinator.baseline(&mut memory, &candidates());
    let result = coordinator
        .choices(&mut memory, &mut visual, 4, &catalog())
        .unwrap();

    assert_eq!(memory.baselines, 1);
    assert_eq!(result.choices.source, RewardChoiceSource::Memory);
    assert_eq!(result.choices.names, vec!["A", "B", "C", "D"]);
    assert_eq!(result.diagnostic, RewardSourceDiagnostic::Ready);
    assert_eq!(visual.calls, 0);
    assert!(result.choices.elapsed < Duration::from_secs(1));
}

#[test]
fn empty_candidate_baseline_clears_the_previous_reward_run() {
    let mut memory = Memory {
        resolution: RewardResolution::Incomplete,
        record_resolution: RewardResolution::Incomplete,
        baselines: 0,
        choices: 0,
        record_choices: 0,
    };
    let mut coordinator = RewardSourceCoordinator::new(false);

    coordinator.baseline(&mut memory, &[]);

    assert_eq!(memory.baselines, 1);
}

#[test]
fn preparing_structured_candidates_replaces_the_previous_run_without_a_fingerprint() {
    let mut state =
        LiveMemoryRewardState::new(RewardMemoryScanner::new(256, 4096, Duration::from_secs(1)));

    state.prepare_candidates(&candidates());
    assert_eq!(state.candidates()[0].choice_name(), "A");

    let next = vec![RewardNeedle::new("B", ["/Lotus/B"]).unwrap()];
    state.prepare_candidates(&next);

    assert_eq!(state.candidates().len(), 1);
    assert_eq!(state.candidates()[0].choice_name(), "B");
}

#[test]
fn current_background_player_record_is_stored() {
    let generation = AtomicU64::new(7);
    let records = Arc::new(Mutex::new(BTreeMap::new()));

    store_player_record_if_current(
        7,
        &generation,
        "remote-a",
        RewardResolution::Confirmed {
            choices: vec!["Forma Blueprint".into()],
            region_start: 1,
        },
        &records,
    );

    assert_eq!(
        records.lock().unwrap().get("remote-a").map(String::as_str),
        Some("Forma Blueprint")
    );
}

#[test]
fn stale_background_player_record_is_discarded() {
    let generation = AtomicU64::new(8);
    let records = Arc::new(Mutex::new(BTreeMap::new()));

    store_player_record_if_current(
        7,
        &generation,
        "remote-a",
        RewardResolution::Confirmed {
            choices: vec!["Forma Blueprint".into()],
            region_start: 1,
        },
        &records,
    );

    assert!(records.lock().unwrap().is_empty());
}

#[test]
fn ambiguous_background_player_record_is_discarded() {
    let generation = AtomicU64::new(7);
    let records = Arc::new(Mutex::new(BTreeMap::new()));

    store_player_record_if_current(
        7,
        &generation,
        "remote-a",
        RewardResolution::Confirmed {
            choices: vec!["Forma Blueprint".into(), "Tiberon Prime Receiver".into()],
            region_start: 1,
        },
        &records,
    );

    assert!(records.lock().unwrap().is_empty());
}

#[test]
fn early_player_record_scan_retries_until_warframe_materializes_the_reward() {
    let generation = AtomicU64::new(7);
    let mut attempts = 0;

    let resolution = scan_player_record_until_ready(7, &generation, Duration::from_secs(1), || {
        attempts += 1;
        if attempts < 3 {
            RewardResolution::Incomplete
        } else {
            RewardResolution::Confirmed {
                choices: vec!["Vasto Prime Blueprint".into()],
                region_start: 1,
            }
        }
    });

    assert_eq!(attempts, 3);
    assert_eq!(
        resolution,
        RewardResolution::Confirmed {
            choices: vec!["Vasto Prime Blueprint".into()],
            region_start: 1,
        }
    );
}

#[test]
fn player_record_retry_stops_when_the_mission_generation_changes() {
    let generation = AtomicU64::new(8);
    let mut attempts = 0;

    let resolution = scan_player_record_until_ready(7, &generation, Duration::from_secs(1), || {
        attempts += 1;
        RewardResolution::Incomplete
    });

    assert_eq!(attempts, 0);
    assert_eq!(resolution, RewardResolution::Incomplete);
}

#[test]
fn incomplete_memory_falls_back_to_ocr() {
    let mut memory = Memory {
        resolution: RewardResolution::Incomplete,
        record_resolution: RewardResolution::Incomplete,
        baselines: 0,
        choices: 0,
        record_choices: 0,
    };
    let mut visual = Visual {
        names: Ok(vec!["A".into(), "B".into(), "C".into(), "D".into()]),
        calls: 0,
    };

    let result = RewardSourceCoordinator::new(false)
        .choices(&mut memory, &mut visual, 4, &catalog())
        .unwrap();

    assert_eq!(result.choices.source, RewardChoiceSource::Ocr);
    assert_eq!(result.diagnostic, RewardSourceDiagnostic::MemoryFallback);
    assert_eq!(visual.calls, 1);
}

#[test]
fn ocr_accepts_the_rendered_three_choice_count() {
    let mut memory = Memory {
        resolution: RewardResolution::Incomplete,
        record_resolution: RewardResolution::Incomplete,
        baselines: 0,
        choices: 0,
        record_choices: 0,
    };
    let mut visual = Visual {
        names: Ok(vec!["A".into(), "B".into(), "C".into()]),
        calls: 0,
    };

    let result = RewardSourceCoordinator::new(false)
        .choices(&mut memory, &mut visual, 3, &catalog())
        .unwrap();

    assert_eq!(result.choices.names.len(), 3);
    assert_eq!(result.choices.source, RewardChoiceSource::Ocr);
}

#[test]
fn incomplete_ocr_is_not_published_as_a_reward_set() {
    let mut memory = Memory {
        resolution: RewardResolution::Incomplete,
        record_resolution: RewardResolution::Incomplete,
        baselines: 0,
        choices: 0,
        record_choices: 0,
    };
    let mut visual = Visual {
        names: Ok(vec!["A".into()]),
        calls: 0,
    };

    let result =
        RewardSourceCoordinator::new(false).choices(&mut memory, &mut visual, 4, &catalog());

    assert!(result.is_none());
    assert_eq!(visual.calls, 1);
}

#[test]
fn validation_mode_reports_memory_and_ocr_disagreement() {
    let mut memory = Memory {
        resolution: RewardResolution::Confirmed {
            choices: vec!["A".into(), "B".into(), "C".into(), "D".into()],
            region_start: 10,
        },
        record_resolution: RewardResolution::Incomplete,
        baselines: 0,
        choices: 0,
        record_choices: 0,
    };
    let mut visual = Visual {
        names: Ok(vec!["D".into(), "C".into(), "B".into(), "A".into()]),
        calls: 0,
    };

    let result = RewardSourceCoordinator::new(true)
        .choices(&mut memory, &mut visual, 4, &catalog())
        .unwrap();

    assert_eq!(result.choices.source, RewardChoiceSource::Memory);
    assert_eq!(result.diagnostic, RewardSourceDiagnostic::Disagreement);
    assert_eq!(visual.calls, 1);
}

#[test]
fn solo_choice_events_invoke_neither_source() {
    let mut memory = Memory {
        resolution: RewardResolution::Incomplete,
        record_resolution: RewardResolution::Incomplete,
        baselines: 0,
        choices: 0,
        record_choices: 0,
    };
    let mut visual = Visual {
        names: Ok(vec![]),
        calls: 0,
    };

    let result =
        RewardSourceCoordinator::new(false).choices(&mut memory, &mut visual, 1, &catalog());

    assert!(result.is_none());
    assert_eq!(memory.choices, 0);
    assert_eq!(visual.calls, 0);
}

#[test]
fn confirmed_player_records_publish_immediately_without_ocr() {
    let mut memory = Memory {
        resolution: RewardResolution::Incomplete,
        record_resolution: RewardResolution::Confirmed {
            choices: vec!["A".into(), "B".into(), "C".into(), "D".into()],
            region_start: 0,
        },
        baselines: 0,
        choices: 0,
        record_choices: 0,
    };

    let result = RewardSourceCoordinator::new(true)
        .player_record_choices(
            &mut memory,
            &["remote-a", "remote-b", "remote-c", "local"],
            Some("local"),
            Some("A"),
        )
        .unwrap();

    assert_eq!(result.choices.names, vec!["A", "B", "C", "D"]);
    assert_eq!(result.choices.source, RewardChoiceSource::Memory);
    assert_eq!(result.diagnostic, RewardSourceDiagnostic::Ready);
    assert_eq!(memory.record_choices, 1);
    assert_eq!(memory.choices, 0);
}

#[test]
fn store_items_log_paths_match_catalog_type_paths() {
    assert!(reward_path_matches(
        "/Lotus/StoreItems/Types/Recipes/Weapons/WeaponParts/PrimeDaikyuUpperLimb",
        "/Lotus/Types/Recipes/Weapons/WeaponParts/PrimeDaikyuUpperLimb",
    ));
}

#[test]
fn accumulated_player_records_are_assembled_with_local_reward_first() {
    let records = std::collections::BTreeMap::from([
        ("remote-a".to_owned(), "Orthos Prime Blueprint".to_owned()),
        (
            "remote-b".to_owned(),
            "Revenant Prime Chassis Blueprint".to_owned(),
        ),
        ("remote-c".to_owned(), "Braton Prime Barrel".to_owned()),
    ]);

    assert_eq!(
        app_lib::assemble_player_record_choices(
            &["local", "remote-a", "remote-b", "remote-c"],
            Some("local"),
            Some("Cedo Prime Stock"),
            &records,
        ),
        Some(vec![
            "Cedo Prime Stock".into(),
            "Orthos Prime Blueprint".into(),
            "Revenant Prime Chassis Blueprint".into(),
            "Braton Prime Barrel".into(),
        ])
    );
}

#[test]
fn a_finished_early_scan_releases_the_identity_for_the_real_response() {
    let active = std::sync::Mutex::new(std::collections::BTreeSet::from([
        "remote-player".to_owned()
    ]));

    app_lib::release_player_record_scan("remote-player", &active);

    assert!(active.lock().unwrap().insert("remote-player".to_owned()));
}

/// The client-mode path. Memory cannot attribute the cards, so the screen supplies all four and the
/// log's local reward is the check that the read is sane.
#[test]
fn visual_choices_publish_when_they_contain_the_logged_local_reward() {
    let mut visual = Visual {
        names: Ok(vec!["A".into(), "B".into(), "C".into(), "D".into()]),
        calls: 0,
    };
    let result = RewardSourceCoordinator::new(false)
        .visual_choices(
            &mut visual,
            &catalog(),
            4,
            Some("C"),
            Duration::from_millis(50),
            &AtomicBool::new(false),
        )
        .expect("a read containing the local reward publishes");
    assert_eq!(result.choices.names, ["A", "B", "C", "D"]);
    assert_eq!(result.choices.source, RewardChoiceSource::Ocr);
    assert_eq!(result.diagnostic, RewardSourceDiagnostic::MemoryFallback);
    assert_eq!(visual.calls, 1);
}

#[test]
fn visual_choices_are_dropped_when_the_logged_local_reward_is_absent() {
    let mut visual = Visual {
        names: Ok(vec!["A".into(), "B".into(), "C".into(), "D".into()]),
        calls: 0,
    };
    // The log is exact about the local player's reward, so a read missing it is wrong somewhere.
    assert!(
        RewardSourceCoordinator::new(false)
            .visual_choices(
                &mut visual,
                &catalog(),
                4,
                Some("Z"),
                Duration::ZERO,
                &AtomicBool::new(false)
            )
            .is_none()
    );
}

#[test]
fn visual_choices_are_dropped_when_the_card_count_is_wrong() {
    let mut visual = Visual {
        names: Ok(vec!["A".into(), "B".into()]),
        calls: 0,
    };
    assert!(
        RewardSourceCoordinator::new(false)
            .visual_choices(
                &mut visual,
                &catalog(),
                4,
                None,
                Duration::ZERO,
                &AtomicBool::new(false)
            )
            .is_none()
    );
}

#[test]
fn a_failed_capture_publishes_nothing() {
    let mut visual = Visual {
        names: Err("no Warframe window found"),
        calls: 0,
    };
    assert!(
        RewardSourceCoordinator::new(false)
            .visual_choices(
                &mut visual,
                &catalog(),
                4,
                Some("A"),
                Duration::ZERO,
                &AtomicBool::new(false)
            )
            .is_none()
    );
}

struct SlowVisual {
    failures: usize,
    calls: usize,
    names: Vec<String>,
}

impl VisualRewardSource for SlowVisual {
    fn choices(&mut self, _candidates: &[RewardCatalogEntry]) -> Result<Vec<String>, &'static str> {
        self.calls += 1;
        if self.calls <= self.failures {
            // What an unpainted reward screen looks like to the matcher.
            return Err("a reward card read as blank");
        }
        Ok(self.names.clone())
    }
}

/// The log announces the rewards about three milliseconds before Warframe paints the cards, so the
/// first capture reads an empty screen. Retry until the cards exist rather than giving up on the
/// first blank read.
#[test]
fn visual_choices_retry_until_the_cards_are_painted() {
    let mut visual = SlowVisual {
        failures: 2,
        calls: 0,
        names: vec!["A".into(), "B".into(), "C".into(), "D".into()],
    };
    let result = RewardSourceCoordinator::new(false)
        .visual_choices(
            &mut visual,
            &catalog(),
            4,
            Some("C"),
            Duration::from_millis(1_500),
            &AtomicBool::new(false),
        )
        .expect("a later attempt sees the painted cards");
    assert_eq!(result.choices.names, ["A", "B", "C", "D"]);
    assert_eq!(visual.calls, 3, "should have retried past the blank reads");
}

#[test]
fn visual_choices_give_up_at_the_deadline() {
    let mut visual = SlowVisual {
        failures: usize::MAX,
        calls: 0,
        names: Vec::new(),
    };
    assert!(
        RewardSourceCoordinator::new(false)
            .visual_choices(
                &mut visual,
                &catalog(),
                4,
                None,
                Duration::from_millis(250),
                &AtomicBool::new(false)
            )
            .is_none()
    );
    assert!(visual.calls >= 2, "should have retried before giving up");
}

/// The reason a live overlay sat over the game for seconds after the rewards had gone.
///
/// This retry runs on the monitor thread, and that thread is also the one that notices the screen
/// disappear and takes the overlay down. EE.log's flush delay means the retry is routinely entered
/// after the screen has already closed, and it used to grind the whole eight-second deadline first
/// -- with the monitor blocked behind it, holding up a hide it had already been told to perform.
#[test]
fn a_screen_that_has_already_gone_stops_the_retry_instead_of_blocking_the_monitor() {
    struct NeverPaints {
        attempts: Arc<AtomicU64>,
    }
    impl app_lib::VisualRewardSource for NeverPaints {
        fn choices(
            &mut self,
            _candidates: &[RewardCatalogEntry],
        ) -> Result<Vec<String>, &'static str> {
            self.attempts
                .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
            Err("a reward card read as blank")
        }
    }

    let attempts = Arc::new(AtomicU64::new(0));
    let mut visual = NeverPaints {
        attempts: Arc::clone(&attempts),
    };
    let gone = AtomicBool::new(true);
    let started = std::time::Instant::now();

    let result = RewardSourceCoordinator::new(false).visual_choices(
        &mut visual,
        &catalog(),
        4,
        None,
        Duration::from_secs(8),
        &gone,
    );

    assert!(result.is_none());
    assert_eq!(
        attempts.load(std::sync::atomic::Ordering::Acquire),
        0,
        "must not even capture once when the screen is known to be gone"
    );
    assert!(
        started.elapsed() < Duration::from_millis(500),
        "held the monitor thread for {:?} against an eight-second deadline",
        started.elapsed()
    );
}
