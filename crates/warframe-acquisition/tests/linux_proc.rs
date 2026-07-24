#![cfg(target_os = "linux")]

use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use warframe_acquisition::{
    AcquisitionError, GameProcess, LinuxProc, MemoryReader, ProcessDiscovery, ReadableRegion,
};

fn write_file(root: &Path, relative: &str, contents: impl AsRef<[u8]>) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn candidate(root: &Path, pid: u32, comm: &str, mapped_executable: &str) {
    write_file(root, &format!("{pid}/comm"), format!("{comm}\n"));
    write_file(
        root,
        &format!("{pid}/maps"),
        format!("140000000-140001000 r--p 00000000 08:01 1 {mapped_executable}\n"),
    );
}

#[test]
fn discovers_full_and_wine_truncated_names_only_when_the_game_executable_is_mapped() {
    let temp = tempfile::tempdir().unwrap();
    candidate(
        temp.path(),
        101,
        "Warframe.x64.ex",
        "/games/Warframe/Downloaded/Public/Warframe.x64.exe",
    );
    candidate(
        temp.path(),
        102,
        "Warframe.x64.exe",
        "/games/Warframe/Downloaded/Public/Warframe.x64.exe",
    );
    candidate(
        temp.path(),
        103,
        "Warframe.x64.ex",
        "/games/Warframe/Tools/Launcher.exe",
    );
    candidate(
        temp.path(),
        104,
        "WarframeLauncher",
        "/games/Warframe/Downloaded/Public/Warframe.x64.exe",
    );

    let process = LinuxProc::at(temp.path()).discover().unwrap().unwrap();

    assert_eq!(process.pid(), 102);
}

#[test]
fn discovery_ignores_vanished_and_malformed_process_entries() {
    let temp = tempfile::tempdir().unwrap();
    write_file(temp.path(), "not-a-pid/comm", "Warframe.x64.exe\n");
    write_file(temp.path(), "200/comm", "Warframe.x64.exe\n");
    candidate(
        temp.path(),
        201,
        "Warframe.x64.ex",
        "/games/Warframe/Downloaded/Public/Warframe.x64.exe",
    );

    assert_eq!(
        LinuxProc::at(temp.path()).discover().unwrap(),
        Some(GameProcess::new(201))
    );
}

#[test]
fn discovery_accepts_a_game_executable_mapping_beneath_a_path_with_spaces() {
    let temp = tempfile::tempdir().unwrap();
    candidate(
        temp.path(),
        202,
        "Warframe.x64.ex",
        "/games/My Warframe Install/Downloaded/Public/Warframe.x64.exe",
    );

    assert_eq!(
        LinuxProc::at(temp.path()).discover().unwrap(),
        Some(GameProcess::new(202))
    );
}

#[test]
fn parses_only_nonempty_readable_ranges_and_skips_special_kernel_mappings() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        temp.path(),
        "300/maps",
        concat!(
            "1000-1800 r--p 00000000 00:00 0 /game/Warframe.x64.exe\n",
            "1800-2000 ---p 00000000 00:00 0\n",
            "2000-2800 rw-p 00000000 00:00 0 [heap]\n",
            "2800-3000 r-xp 00000000 00:00 0 [vdso]\n",
            "bad maps line\n",
            "4000-4000 r--p 00000000 00:00 0\n",
        ),
    );

    let regions = LinuxProc::at(temp.path())
        .readable_regions(&GameProcess::new(300))
        .unwrap();

    assert_eq!(
        regions,
        vec![
            ReadableRegion::new(0x1000, 0x800),
            ReadableRegion::new(0x2000, 0x800)
        ]
    );
}

#[test]
fn bounded_reads_return_partial_data_without_touching_the_rest_of_the_buffer() {
    let temp = tempfile::tempdir().unwrap();
    let mut memory = vec![0_u8; 32];
    memory[8..15].copy_from_slice(b"payload");
    write_file(temp.path(), "400/mem", memory);
    let mut buffer = [0xaa; 12];

    let read = LinuxProc::at(temp.path())
        .read_at(&GameProcess::new(400), 8, &mut buffer)
        .unwrap();

    assert_eq!(read, 12);
    assert_eq!(&buffer[..7], b"payload");
    assert_eq!(&buffer[7..], &[0, 0, 0, 0, 0]);
}

#[test]
fn a_read_past_the_available_memory_is_a_clean_zero_length_partial_read() {
    let temp = tempfile::tempdir().unwrap();
    write_file(temp.path(), "401/mem", b"short");
    let mut buffer = [0xaa; 8];

    let read = LinuxProc::at(temp.path())
        .read_at(&GameProcess::new(401), 100, &mut buffer)
        .unwrap();

    assert_eq!(read, 0);
    assert_eq!(buffer, [0xaa; 8]);
}

#[test]
fn missing_process_files_report_that_the_process_exited() {
    let temp = tempfile::tempdir().unwrap();
    let adapter = LinuxProc::at(temp.path());

    assert_eq!(
        adapter
            .readable_regions(&GameProcess::new(500))
            .unwrap_err(),
        AcquisitionError::ProcessExited { pid: 500 }
    );
    assert_eq!(
        adapter
            .read_at(&GameProcess::new(500), 0, &mut [0_u8; 4])
            .unwrap_err(),
        AcquisitionError::ProcessExited { pid: 500 }
    );
}

#[test]
fn denied_memory_reports_actionable_permission_guidance_without_paths() {
    let temp = tempfile::tempdir().unwrap();
    write_file(temp.path(), "600/mem", b"secret");
    let path = temp.path().join("600/mem");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).unwrap();

    let error = LinuxProc::at(temp.path())
        .read_at(&GameProcess::new(600), 0, &mut [0_u8; 4])
        .unwrap_err();
    let rendered = error.to_string();

    assert_eq!(error, AcquisitionError::MemoryPermissionDenied { pid: 600 });
    assert!(rendered.contains("same user"));
    assert!(rendered.contains("Yama"));
    assert!(rendered.contains("sandbox"));
    assert!(!rendered.contains(temp.path().to_str().unwrap()));
}
