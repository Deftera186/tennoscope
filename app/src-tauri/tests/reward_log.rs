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
        "ProjectionRewardChoice.lua: Missing icon data!",
        "ProjectionRewardChoice.lua: Missing icon data!",
        "ProjectionRewardChoice.lua: Missing icon data!",
        "ProjectionRewardChoice.lua: Missing icon data!",
        "ProjectionsCountdown.lua: Initialize timer nil 15",
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
                local_reward_path: None,
            },
        ]
    );
}

#[test]
fn multi_player_relic_paths_request_a_baseline_before_the_reward_window() {
    let mut machine = RewardLogMachine::default();

    assert!(
        machine
            .observe_line("loaded (/Lotus/Types/Game/Projections/First)")
            .is_empty()
    );
    assert_eq!(
        machine.observe_line("loaded (/Lotus/Types/Game/Projections/Second)"),
        vec![RewardLogEvent::BaselineRequested {
            relic_paths: vec![
                "/Lotus/Types/Game/Projections/First".into(),
                "/Lotus/Types/Game/Projections/Second".into(),
            ],
        }]
    );
    assert!(
        machine
            .observe_line("VoidProjections: OpenVoidProjectionRewardScreenRMI")
            .is_empty()
    );
}

#[test]
fn rendered_card_count_overrides_the_number_of_network_responders() {
    let mut machine = RewardLogMachine::default();
    let events = [
        "VoidProjections: OpenVoidProjectionRewardScreenRMI",
        "VoidProjections: Client got reward info from player-a",
        "VoidProjections: Client got reward info from player-b",
        "VoidProjections: Client got reward info from player-c",
        "VoidProjections: Client got reward info from player-d",
        "VoidProjections: Client has reward info for all players now",
        "ProjectionRewardChoice.lua: Got rewards",
        "ProjectionRewardChoice.lua: Missing icon data!",
        "ProjectionRewardChoice.lua: Missing icon data!",
        "ProjectionRewardChoice.lua: Missing icon data!",
        "ProjectionsCountdown.lua: Initialize timer nil 15",
    ]
    .into_iter()
    .flat_map(|line| machine.observe_line(line))
    .collect::<Vec<_>>();

    assert_eq!(
        events,
        vec![RewardLogEvent::ChoicesReady {
            expected_choices: 3,
            local_reward_path: None,
        }]
    );
}

#[test]
fn host_sequence_emits_the_rendered_choice_count() {
    let mut machine = RewardLogMachine::default();
    let events = [
        "VoidProjections: OpenVoidProjectionRewardScreen - PostMigration: 0",
        "VoidProjections: Host got reward info from player-a",
        "VoidProjections: Host got reward info from player-b",
        "VoidProjections: Host got reward info from player-c",
        "VoidProjections: Host got reward info from player-d",
        "VoidProjections: Host has reward info for all players now!",
        "ProjectionRewardChoice.lua: Got rewards",
        "ProjectionRewardChoice.lua: Missing icon data!",
        "ProjectionRewardChoice.lua: Missing icon data!",
        "ProjectionRewardChoice.lua: Missing icon data!",
        "ProjectionRewardChoice.lua: Missing icon data!",
        "ProjectionsCountdown.lua: Initialize timer nil 15",
    ]
    .into_iter()
    .flat_map(|line| machine.observe_line(line))
    .collect::<Vec<_>>();

    assert_eq!(
        events,
        vec![RewardLogEvent::ChoicesReady {
            expected_choices: 4,
            local_reward_path: None,
        }]
    );
}

#[test]
fn choices_include_the_explicitly_logged_local_reward_path() {
    let mut machine = RewardLogMachine::default();
    let events = [
        "VoidProjections: OpenVoidProjectionRewardScreenRMI",
        "VoidProjections: player gets reward /Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint",
        "VoidProjections: Client got reward info from player-a",
        "VoidProjections: Client got reward info from player-b",
        "VoidProjections: Client has reward info for all players now",
        "ProjectionRewardChoice.lua: Missing icon data!",
        "ProjectionRewardChoice.lua: Missing icon data!",
        "ProjectionsCountdown.lua: Initialize timer nil 15",
    ]
    .into_iter()
    .flat_map(|line| machine.observe_line(line))
    .collect::<Vec<_>>();

    assert_eq!(
        events,
        vec![RewardLogEvent::ChoicesReady {
            expected_choices: 2,
            local_reward_path: Some("/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint".into()),
        }]
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
    assert!(
        machine
            .observe_line("VoidProjections: OpenVoidProjectionRewardScreenRMI")
            .is_empty()
    );
}

#[test]
fn byte_stream_preserves_lines_split_across_monitor_reads() {
    let mut machine = RewardLogMachine::default();

    assert!(
        machine
            .observe_bytes(b"OpenVoidProjectionReward")
            .is_empty()
    );
    let events = machine.observe_bytes(b"ScreenRMI\n");

    assert!(events.is_empty());
}
