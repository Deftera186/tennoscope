use std::fs;
use tempfile::tempdir;

use app_lib::{
    accept_setup_risk, contains_inventory_sync_trigger, read_setup_status, resolve_local_paths,
};

#[test]
fn risk_disclosure_is_asked_once_and_persists_acceptance() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("setup.json");
    assert!(!read_setup_status(&path).unwrap().risk_accepted);
    accept_setup_risk(&path).unwrap();
    assert!(read_setup_status(&path).unwrap().risk_accepted);
    let wire = fs::read_to_string(path).unwrap();
    assert!(!wire.contains("accountId"));
    assert!(!wire.contains("nonce"));
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
fn corrupt_setup_state_fails_closed() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("setup.json");
    fs::write(&path, "not json").unwrap();
    assert!(!read_setup_status(&path).unwrap().risk_accepted);
}

#[test]
fn tennoscope_reuses_existing_legacy_setup_and_database() {
    let directory = tempdir().unwrap();
    let legacy_setup = directory.path().join("setup.json");
    let legacy_database = directory.path().join("warframe-helper.sqlite3");
    fs::write(&legacy_setup, r#"{"risk_accepted":true}"#).unwrap();
    fs::write(&legacy_database, b"existing database").unwrap();

    let paths = resolve_local_paths(directory.path());

    assert_eq!(paths.setup, legacy_setup);
    assert_eq!(paths.database, legacy_database);
}

#[test]
fn tennoscope_uses_new_names_for_fresh_local_data() {
    let directory = tempdir().unwrap();

    let paths = resolve_local_paths(directory.path());

    assert_eq!(paths.setup, directory.path().join("tennoscope-setup.json"));
    assert_eq!(paths.database, directory.path().join("tennoscope.sqlite3"));
}
