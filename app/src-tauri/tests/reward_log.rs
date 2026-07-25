use app_lib::{RewardLogEvent, RewardLogMachine};

#[test]
fn reward_window_activity_tracks_open_and_close() {
    let mut machine = RewardLogMachine::default();
    assert!(!machine.reward_window_open());

    machine.observe_line("VoidProjections: OpenVoidProjectionRewardScreenRMI");
    assert!(machine.reward_window_open());

    machine.observe_line("ProjectionRewardChoice.lua: Relic reward screen shut down");
    assert!(!machine.reward_window_open());
}

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
        .filter(is_render_lifecycle_event)
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
    assert_eq!(
        machine.observe_line("VoidProjections: OpenVoidProjectionRewardScreenRMI"),
        vec![RewardLogEvent::RewardWindowOpened]
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
    .filter(is_render_lifecycle_event)
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
    .filter(is_render_lifecycle_event)
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
    .filter(is_render_lifecycle_event)
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
    assert_eq!(
        machine.observe_line("VoidProjections: OpenVoidProjectionRewardScreenRMI"),
        vec![RewardLogEvent::RewardWindowOpened]
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

    assert_eq!(events, vec![RewardLogEvent::RewardWindowOpened]);
}

#[test]
fn live_response_sequence_emits_ordered_responders_and_completes_before_rendering() {
    let mut machine = RewardLogMachine::default();
    let events = [
        "14189.789 Sys [Info]: VoidProjections: OpenVoidProjectionRewardScreenRMI",
        "14189.989 Sys [Info]: VoidProjections: Client got reward info from de1e7ed00000000000000005",
        "14190.058 Sys [Info]: VoidProjections: Client got reward info from de1e7ed0000000000000000a",
        "14190.062 Sys [Info]: VoidProjections: Client got reward info from de1e7ed00000000000000004",
        "14190.095 Sys [Info]: VoidProjections: de1e7ed00000000000000006 gets reward /Lotus/StoreItems/Types/Recipes/Weapons/WeaponParts/PrimeDaikyuUpperLimb",
        "14190.095 Sys [Info]: VoidProjections: Client got reward info from de1e7ed00000000000000006",
        "14190.336 Sys [Info]: VoidProjections: Client has reward info for all players now",
    ]
    .into_iter()
    .flat_map(|line| machine.observe_line(line))
    .collect::<Vec<_>>();

    assert_eq!(
        events,
        vec![
            RewardLogEvent::RewardWindowOpened,
            RewardLogEvent::ResponderReceived {
                identity: "de1e7ed00000000000000005".into(),
                is_local: false,
            },
            RewardLogEvent::ResponderReceived {
                identity: "de1e7ed0000000000000000a".into(),
                is_local: false,
            },
            RewardLogEvent::ResponderReceived {
                identity: "de1e7ed00000000000000004".into(),
                is_local: false,
            },
            RewardLogEvent::ResponderReceived {
                identity: "de1e7ed00000000000000006".into(),
                is_local: true,
            },
            RewardLogEvent::ResponsesComplete {
                responders: vec![
                    "de1e7ed00000000000000005".into(),
                    "de1e7ed0000000000000000a".into(),
                    "de1e7ed00000000000000004".into(),
                    "de1e7ed00000000000000006".into(),
                ],
                screen_order: vec![
                    "de1e7ed00000000000000006".into(),
                    "de1e7ed00000000000000005".into(),
                    "de1e7ed0000000000000000a".into(),
                    "de1e7ed00000000000000004".into(),
                ],
                local_reward_path: Some(
                    "/Lotus/StoreItems/Types/Recipes/Weapons/WeaponParts/PrimeDaikyuUpperLimb"
                        .into(),
                ),
                local_identity: Some("de1e7ed00000000000000006".into()),
            },
        ]
    );
}

#[test]
fn opening_reward_screen_initializes_candidates_for_one_unique_relic() {
    let mut machine = RewardLogMachine::default();
    let relic = "/Lotus/Types/Game/Projections/LithA1Bronze";

    assert!(
        machine
            .observe_line(&format!("LoadResource {relic}"))
            .is_empty()
    );

    assert_eq!(
        machine.observe_line("VoidProjections: OpenVoidProjectionRewardScreenRMI"),
        vec![
            RewardLogEvent::RewardWindowOpened,
            RewardLogEvent::BaselineRequested {
                relic_paths: vec![relic.into()],
            },
        ]
    );
}

#[test]
fn waiting_list_ring_rotates_to_the_local_players_screen_order() {
    let mut machine = RewardLogMachine::default();
    let complete = [
        "VoidProjections: OpenVoidProjectionRewardScreenRMI",
        "VoidProjections: Client got reward info from de1e7ed00000000000000010",
        "VoidProjections: Still waiting on response from de1e7ed0000000000000000f",
        "VoidProjections: Still waiting on response from de1e7ed00000000000000006",
        "VoidProjections: Still waiting on response from de1e7ed00000000000000009",
        "VoidProjections: de1e7ed00000000000000006 gets reward /Lotus/StoreItems/Types/Recipes/Weapons/WeaponParts/BratonPrimeBarrel",
        "VoidProjections: Client got reward info from de1e7ed00000000000000006",
        "VoidProjections: Client got reward info from de1e7ed00000000000000009",
        "VoidProjections: Client got reward info from de1e7ed0000000000000000f",
        "VoidProjections: Client has reward info for all players now",
    ]
    .into_iter()
    .flat_map(|line| machine.observe_line(line))
    .find_map(|event| match event {
        RewardLogEvent::ResponsesComplete { screen_order, .. } => Some(screen_order),
        _ => None,
    })
    .unwrap();

    assert_eq!(
        complete,
        vec![
            "de1e7ed00000000000000006",
            "de1e7ed00000000000000009",
            "de1e7ed00000000000000010",
            "de1e7ed0000000000000000f",
        ]
    );
}

#[test]
fn waiting_responder_is_exposed_before_its_reward_arrives() {
    let mut machine = RewardLogMachine::default();
    machine.observe_line("VoidProjections: OpenVoidProjectionRewardScreenRMI");

    assert_eq!(
        machine.observe_line(
            "VoidProjections: Still waiting on response from de1e7ed00000000000000003"
        ),
        vec![RewardLogEvent::ResponderExpected {
            identity: "de1e7ed00000000000000003".into(),
        }]
    );
    assert!(
        machine
            .observe_line(
                "VoidProjections: Still waiting on response from de1e7ed00000000000000003"
            )
            .is_empty()
    );
}

fn is_render_lifecycle_event(event: &RewardLogEvent) -> bool {
    matches!(
        event,
        RewardLogEvent::BaselineRequested { .. } | RewardLogEvent::ChoicesReady { .. }
    )
}
