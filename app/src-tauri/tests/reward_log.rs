use app_lib::{RewardLogEvent, RewardLogMachine};

#[test]
fn online_sequence_requests_one_baseline_and_emits_the_observed_choice_count() {
    let mut machine = RewardLogMachine::default();
    let lines = [
        "Resource load completed (/Lotus/Types/Game/Projections/T1VoidProjectionStyanaxPrimeDBronze)",
        "Resource load completed (/Lotus/Types/Game/Projections/T1VoidProjectionGyrePrimeDBronze)",
        "VoidProjections: OpenVoidProjectionRewardScreenRMI",
        "VoidProjections: Client got reward info from player-a",
        "VoidProjections: Still waiting on response from player-b",
        "VoidProjections: Client got reward info from player-b",
        "VoidProjections: Client got reward info from player-c",
        "VoidProjections: Client got reward info from player-d",
        "VoidProjections: Client has reward info for all players now",
        "ProjectionRewardChoice.lua: Got rewards",
    ];
    let events = lines
        .into_iter()
        .flat_map(|line| machine.observe_line(line))
        .collect::<Vec<_>>();

    assert_eq!(
        events,
        vec![
            RewardLogEvent::BaselineRequested {
                relic_paths: vec![
                    "/Lotus/Types/Game/Projections/T1VoidProjectionStyanaxPrimeDBronze".into(),
                    "/Lotus/Types/Game/Projections/T1VoidProjectionGyrePrimeDBronze".into(),
                ],
            },
            RewardLogEvent::ChoicesReady {
                expected_choices: 4,
            },
        ]
    );
}

#[test]
fn solo_reward_without_a_choice_screen_never_requests_memory_scanning() {
    let mut machine = RewardLogMachine::default();
    let events = [
        "Resource load completed (/Lotus/Types/Game/Projections/T1VoidProjectionStyanaxPrimeCBronze)",
        "VoidProjections: player gets reward /Lotus/StoreItems/Types/Recipes/Weapons/Test",
    ]
    .into_iter()
    .flat_map(|line| machine.observe_line(line))
    .collect::<Vec<_>>();

    assert!(events.is_empty());
}

#[test]
fn shutdown_closes_and_resets_the_reward_window() {
    let mut machine = RewardLogMachine::default();
    machine.observe_line("Resource load completed (/Lotus/Types/Game/Projections/First)");
    machine.observe_line("VoidProjections: OpenVoidProjectionRewardScreenRMI");

    assert_eq!(
        machine.observe_line("ProjectionRewardChoice.lua: Relic reward screen shut down"),
        vec![RewardLogEvent::Closed]
    );
    assert_eq!(
        machine.observe_line("VoidProjections: OpenVoidProjectionRewardScreenRMI"),
        vec![RewardLogEvent::BaselineRequested {
            relic_paths: Vec::new(),
        }]
    );
}
