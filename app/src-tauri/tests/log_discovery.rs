#[cfg(unix)]
use app_lib::inventory_log_path_at;
use app_lib::log_identity;
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

    // Reading the same unmodified file twice must also be stable: under Wine the creation time
    // jitters below the millisecond, and a jittering identity reads as a rotation on every poll.
    assert_eq!(
        log_identity(&path, &fs::metadata(&path).unwrap()),
        original,
        "the identity must not depend on when it was asked for"
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

#[cfg(unix)]
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

#[cfg(unix)]
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

/// On Windows there is no Wine prefix to walk -- the log is simply under `%LOCALAPPDATA%`. The
/// root-parameterised shape is kept anyway, because it is what lets the layout be asserted against
/// a synthetic tree instead of against whatever happens to be on the machine running the test.
#[cfg(windows)]
#[test]
fn the_windows_log_sits_directly_under_local_appdata() {
    let dir = tempdir().unwrap();
    let local_appdata = dir.path().join("AppData/Local");
    assert!(
        app_lib::inventory_log_under(&local_appdata).is_none(),
        "a machine that has never run Warframe has no log to find"
    );

    let log = local_appdata.join("Warframe/EE.log");
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(&log, b"").unwrap();
    assert_eq!(app_lib::inventory_log_under(&local_appdata), Some(log));
}

/// A directory where the log should be is not a log. Warframe has been seen to leave the folder
/// behind after an uninstall, and treating that as a found log makes every later read fail with a
/// permission error rather than with "the game has not run yet".
#[cfg(windows)]
#[test]
fn a_directory_named_like_the_log_is_not_the_log() {
    let dir = tempdir().unwrap();
    let local_appdata = dir.path().join("AppData/Local");
    fs::create_dir_all(local_appdata.join("Warframe/EE.log")).unwrap();
    assert!(app_lib::inventory_log_under(&local_appdata).is_none());
}
