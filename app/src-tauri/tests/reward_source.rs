use std::time::Duration;

use app_lib::{
    MemoryRewardSource, RewardChoiceSource, RewardSourceCoordinator, RewardSourceDiagnostic,
    VisualRewardSource,
};
use warframe_acquisition::{RewardCatalogEntry, RewardNeedle, RewardResolution};

struct Memory {
    resolution: RewardResolution,
    baselines: usize,
    choices: usize,
    anchors: Vec<Option<String>>,
}

impl MemoryRewardSource for Memory {
    fn baseline(&mut self, _candidates: &[RewardNeedle]) {
        self.baselines += 1;
    }

    fn choices(&mut self, _expected: usize, local_choice: Option<&str>) -> RewardResolution {
        self.choices += 1;
        self.anchors.push(local_choice.map(str::to_owned));
        self.resolution.clone()
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
        baselines: 0,
        choices: 0,
        anchors: Vec::new(),
    };
    let mut visual = Visual {
        names: Ok(vec![]),
        calls: 0,
    };
    let mut coordinator = RewardSourceCoordinator::new(false);

    coordinator.baseline(&mut memory, &candidates());
    let result = coordinator
        .choices(&mut memory, &mut visual, 4, Some("A"), &catalog())
        .unwrap();

    assert_eq!(memory.baselines, 1);
    assert_eq!(memory.anchors, vec![Some("A".into())]);
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
        baselines: 0,
        choices: 0,
        anchors: Vec::new(),
    };
    let mut visual = Visual {
        names: Ok(vec!["A".into(), "B".into(), "C".into(), "D".into()]),
        calls: 0,
    };

    let result = RewardSourceCoordinator::new(false)
        .choices(&mut memory, &mut visual, 4, None, &catalog())
        .unwrap();

    assert_eq!(result.choices.source, RewardChoiceSource::Ocr);
    assert_eq!(result.diagnostic, RewardSourceDiagnostic::MemoryFallback);
    assert_eq!(visual.calls, 1);
}

#[test]
fn ocr_accepts_the_rendered_three_choice_count() {
    let mut memory = Memory {
        resolution: RewardResolution::Incomplete,
        baselines: 0,
        choices: 0,
        anchors: Vec::new(),
    };
    let mut visual = Visual {
        names: Ok(vec!["A".into(), "B".into(), "C".into()]),
        calls: 0,
    };

    let result = RewardSourceCoordinator::new(false)
        .choices(&mut memory, &mut visual, 3, None, &catalog())
        .unwrap();

    assert_eq!(result.choices.names.len(), 3);
    assert_eq!(result.choices.source, RewardChoiceSource::Ocr);
}

#[test]
fn incomplete_ocr_is_not_published_as_a_reward_set() {
    let mut memory = Memory {
        resolution: RewardResolution::Incomplete,
        baselines: 0,
        choices: 0,
        anchors: Vec::new(),
    };
    let mut visual = Visual {
        names: Ok(vec!["A".into()]),
        calls: 0,
    };

    let result =
        RewardSourceCoordinator::new(false).choices(&mut memory, &mut visual, 4, None, &catalog());

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
        baselines: 0,
        choices: 0,
        anchors: Vec::new(),
    };
    let mut visual = Visual {
        names: Ok(vec!["D".into(), "C".into(), "B".into(), "A".into()]),
        calls: 0,
    };

    let result = RewardSourceCoordinator::new(true)
        .choices(&mut memory, &mut visual, 4, None, &catalog())
        .unwrap();

    assert_eq!(result.choices.source, RewardChoiceSource::Memory);
    assert_eq!(result.diagnostic, RewardSourceDiagnostic::Disagreement);
    assert_eq!(visual.calls, 1);
}

#[test]
fn solo_choice_events_invoke_neither_source() {
    let mut memory = Memory {
        resolution: RewardResolution::Incomplete,
        baselines: 0,
        choices: 0,
        anchors: Vec::new(),
    };
    let mut visual = Visual {
        names: Ok(vec![]),
        calls: 0,
    };

    let result =
        RewardSourceCoordinator::new(false).choices(&mut memory, &mut visual, 1, None, &catalog());

    assert!(result.is_none());
    assert_eq!(memory.choices, 0);
    assert_eq!(visual.calls, 0);
}
