use std::fs;
use tempfile::tempdir;

use app_lib::{
    AccessMode, contains_inventory_sync_trigger, read_setup_status, resolve_local_paths,
    save_access_mode,
};

#[test]
fn each_access_mode_has_the_exact_game_observation_boundary() {
    let cases = [
        (
            AccessMode::Companion,
            (false, false, false, false, false, false),
        ),
        (AccessMode::Overlay, (true, true, true, true, false, false)),
        (AccessMode::Full, (true, true, true, true, true, true)),
    ];

    for (mode, expected) in cases {
        let policy = mode.policy();
        assert_eq!(
            (
                policy.observe_process_presence,
                policy.observe_ee_log,
                policy.capture_screen,
                policy.show_overlays,
                policy.read_process_memory,
                policy.acquire_inventory,
            ),
            expected,
            "wrong access boundary for {mode:?}",
        );
    }
}

#[test]
fn fresh_setup_is_incomplete_and_has_no_effective_access() {
    let directory = tempdir().unwrap();
    let status = read_setup_status(&directory.path().join("setup.json")).unwrap();

    assert!(!status.setup_complete);
    assert_eq!(status.access_mode, None);
}

#[test]
fn legacy_accepted_setup_migrates_to_full_access() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("setup.json");
    fs::write(&path, r#"{"risk_accepted":true}"#).unwrap();

    let status = read_setup_status(&path).unwrap();
    assert!(status.setup_complete);
    assert_eq!(status.access_mode, Some(AccessMode::Full));
}

#[test]
fn legacy_unaccepted_setup_remains_incomplete() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("setup.json");
    fs::write(&path, r#"{"risk_accepted":false}"#).unwrap();

    assert!(!read_setup_status(&path).unwrap().setup_complete);
}

#[test]
fn every_access_mode_round_trips_as_the_only_durable_setup_state() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("setup.json");

    for mode in [AccessMode::Companion, AccessMode::Overlay, AccessMode::Full] {
        let status = save_access_mode(&path, mode).unwrap();
        assert_eq!(status.access_mode, Some(mode));
        assert_eq!(read_setup_status(&path).unwrap().access_mode, Some(mode));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            format!(r#"{{"access_mode":"{}"}}"#, mode.as_str()),
        );
    }
}

#[test]
fn changing_mode_replaces_an_existing_setup_file() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("setup.json");
    fs::write(&path, r#"{"access_mode":"companion"}"#).unwrap();

    save_access_mode(&path, AccessMode::Overlay).unwrap();

    assert_eq!(
        read_setup_status(&path).unwrap().access_mode,
        Some(AccessMode::Overlay)
    );
}

#[test]
fn only_a_complete_inventory_sync_log_line_triggers_refresh() {
    assert!(contains_inventory_sync_trigger(
        b"123 Inventory sync done\n"
    ));
    assert!(!contains_inventory_sync_trigger(b"123 Inventory sync do"));
    assert!(!contains_inventory_sync_trigger(
        b"authorization request done\n"
    ));
}

#[test]
fn corrupt_or_unknown_setup_state_fails_closed() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("setup.json");
    for invalid in ["not json", r#"{"access_mode":"other"}"#] {
        fs::write(&path, invalid).unwrap();
        let status = read_setup_status(&path).unwrap();
        assert!(!status.setup_complete);
        assert_eq!(status.access_mode, None);
    }
}

#[test]
fn tennoscope_uses_new_names_for_fresh_local_data() {
    let directory = tempdir().unwrap();

    let paths = resolve_local_paths(directory.path());

    assert_eq!(paths.setup, directory.path().join("tennoscope-setup.json"));
    assert_eq!(paths.database, directory.path().join("tennoscope.sqlite3"));
}
