use std::time::Duration;

use app_lib::{
    MemoryRewardSource, RewardChoiceSource, RewardSourceCoordinator, RewardSourceDiagnostic,
    VisualRewardSource, reward_path_matches, rotate_choices_to_local,
};

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
use warframe_acquisition::{RewardCatalogEntry, RewardNeedle, RewardResolution};

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
    vec![RewardNeedle::new("A", ["/Lotus/A"]).unwrap()]
}

fn catalog() -> Vec<RewardCatalogEntry> {
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
