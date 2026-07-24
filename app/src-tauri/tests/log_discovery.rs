use app_lib::inventory_log_path_at;
use std::fs;
use tempfile::tempdir;

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
