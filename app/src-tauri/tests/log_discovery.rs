use app_lib::{inventory_log_path_at, log_identity};
use std::fs;
use tempfile::tempdir;

/// The identity is what tells "the same log, grown" apart from "a new log at the same path", which
/// is the difference between resuming at the old offset and re-reading from zero. It used to be
/// `dev:ino`, which does not exist on Windows.
#[test]
fn log_identity_survives_appending_and_changes_when_the_file_is_replaced() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("EE.log");
    fs::write(&path, b"first").unwrap();

    let original = log_identity(&path, &fs::metadata(&path).unwrap());

    // Appending is the normal case: same file, more bytes, and the reader must keep its offset.
    fs::write(&path, b"first and more").unwrap();
    assert_eq!(
        log_identity(&path, &fs::metadata(&path).unwrap()),
        original,
        "appending to the log must not change its identity"
    );

    // A different path is a different log even with identical contents.
    let other = dir.path().join("other-EE.log");
    fs::write(&other, b"first").unwrap();
    assert_ne!(
        log_identity(&other, &fs::metadata(&other).unwrap()),
        original,
        "a log at another path must not share an identity"
    );
}

#[test]
fn discovers_any_wine_user_from_environment_and_retries_creation() {
    let dir = tempdir().unwrap();
    let proc_root = dir.path().join("proc");
    let prefix = dir.path().join("prefix");
    fs::create_dir_all(proc_root.join("7")).unwrap();
    fs::write(
        proc_root.join("7/environ"),
        format!("A=1\0WINEPREFIX={}\0", prefix.display()),
    )
    .unwrap();
    assert!(inventory_log_path_at(&proc_root, 7).is_none());
    let log = prefix.join("drive_c/users/alice/AppData/Local/Warframe/EE.log");
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(&log, b"").unwrap();
    assert_eq!(inventory_log_path_at(&proc_root, 7), Some(log));
}

#[test]
fn derives_prefix_from_mapped_drive_c_path_without_environment() {
    let dir = tempdir().unwrap();
    let proc_root = dir.path().join("proc");
    let prefix = dir.path().join("prefix");
    fs::create_dir_all(proc_root.join("7")).unwrap();
    fs::write(proc_root.join("7/environ"), b"").unwrap();
    fs::write(
        proc_root.join("7/maps"),
        format!(
            "1000-2000 r--p 0 0:0 0 {}/drive_c/windows/system32/a.dll\n",
            prefix.display()
        ),
    )
    .unwrap();
    let log = prefix.join("drive_c/users/bob/AppData/Local/Warframe/EE.log");
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(&log, b"").unwrap();
    assert_eq!(inventory_log_path_at(&proc_root, 7), Some(log));
}
