#![cfg(target_os = "linux")]

use std::{fs, os::unix::fs::PermissionsExt, path::Path};

use warframe_acquisition::{
    AcquisitionError, GameProcess, LinuxProc, MemoryReader, ProcessDiscovery, ReadableRegion,
    RegionScanPriority,
};

fn write_file(root: &Path, relative: &str, contents: impl AsRef<[u8]>) {
    let path = root.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, contents).unwrap();
}

fn candidate(root: &Path, pid: u32, comm: &str, mapped_executable: &str) {
    candidate_started_at(root, pid, comm, mapped_executable, 777);
}

fn candidate_started_at(
    root: &Path,
    pid: u32,
    comm: &str,
    mapped_executable: &str,
    start_time: u64,
) {
    write_file(root, &format!("{pid}/comm"), format!("{comm}\n"));
    write_stat(root, pid, comm, start_time);
    write_file(
        root,
        &format!("{pid}/maps"),
        format!("140000000-140001000 r--p 00000000 08:01 1 {mapped_executable}\n"),
    );
}

fn write_stat(root: &Path, pid: u32, comm: &str, start_time: u64) {
    let mut fields = vec!["0".to_owned(); 18];
    fields.push(start_time.to_string());
    write_file(
        root,
        &format!("{pid}/stat"),
        format!("{pid} ({comm}) S {}\n", fields.join(" ")),
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
fn a_relaunch_is_discovered_from_the_new_process_rather_than_the_dying_one() {
    let temp = tempfile::tempdir().unwrap();
    let executable = "/games/Warframe/Downloaded/Public/Warframe.x64.exe";
    candidate_started_at(temp.path(), 100, "Warframe.x64.exe", executable, 777);
    candidate_started_at(temp.path(), 200, "Warframe.x64.exe", executable, 778);

    assert_eq!(
        LinuxProc::at(temp.path())
            .discover()
            .unwrap()
            .map(GameProcess::pid),
        Some(200),
        "the replacement process has the newest start time"
    );
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
        LinuxProc::at(temp.path())
            .discover()
            .unwrap()
            .map(GameProcess::pid),
        Some(201)
    );
}

#[test]
fn discovery_propagates_permission_and_non_vanishing_maps_failures() {
    let denied = tempfile::tempdir().unwrap();
    candidate(
        denied.path(),
        210,
        "Warframe.x64.ex",
        "/games/Warframe/Downloaded/Public/Warframe.x64.exe",
    );
    fs::set_permissions(
        denied.path().join("210/maps"),
        fs::Permissions::from_mode(0o000),
    )
    .unwrap();
    assert_eq!(
        LinuxProc::at(denied.path()).discover().unwrap_err(),
        AcquisitionError::MemoryPermissionDenied { pid: 210 }
    );

    let malformed = tempfile::tempdir().unwrap();
    candidate(
        malformed.path(),
        211,
        "Warframe.x64.ex",
        "/games/Warframe/Downloaded/Public/Warframe.x64.exe",
    );
    fs::remove_file(malformed.path().join("211/maps")).unwrap();
    fs::create_dir(malformed.path().join("211/maps")).unwrap();
    assert_eq!(
        LinuxProc::at(malformed.path()).discover().unwrap_err(),
        AcquisitionError::ProcessDiscoveryFailed
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
        LinuxProc::at(temp.path())
            .discover()
            .unwrap()
            .map(GameProcess::pid),
        Some(202)
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
            "3000-3800 rw-p 00000000 00:01 1 /memfd:wine-shared\n",
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
            ReadableRegion::classified(0x1000, 0x800, RegionScanPriority::FileBacked),
            ReadableRegion::classified(0x2000, 0x800, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(
                0x3000,
                0x800,
                RegionScanPriority::WritablePrivateFileBacked
            )
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
fn repeated_reads_reuse_one_open_memory_descriptor() {
    let temp = tempfile::tempdir().unwrap();
    candidate(
        temp.path(),
        402,
        "Warframe.x64.ex",
        "/games/Warframe/Downloaded/Public/Warframe.x64.exe",
    );
    write_file(temp.path(), "402/mem", b"first-second");
    let adapter = LinuxProc::at(temp.path());
    let process = adapter.discover().unwrap().unwrap();
    let mut first = [0_u8; 5];
    assert_eq!(adapter.read_at(&process, 0, &mut first).unwrap(), 5);
    fs::remove_file(temp.path().join("402/mem")).unwrap();
    fs::remove_file(temp.path().join("402/stat")).unwrap();

    let mut second = [0_u8; 6];
    assert_eq!(adapter.read_at(&process, 6, &mut second).unwrap(), 6);
    assert_eq!(&first, b"first");
    assert_eq!(&second, b"second");
}

#[test]
fn changed_start_time_rejects_a_reused_pid_before_maps_or_memory_access() {
    let temp = tempfile::tempdir().unwrap();
    candidate(
        temp.path(),
        403,
        "Warframe.x64.ex",
        "/games/Warframe/Downloaded/Public/Warframe.x64.exe",
    );
    write_file(temp.path(), "403/mem", b"memory");
    let adapter = LinuxProc::at(temp.path());
    let process = adapter.discover().unwrap().unwrap();
    write_stat(temp.path(), 403, "Warframe.x64.ex", 778);

    assert_eq!(
        adapter.readable_regions(&process).unwrap_err(),
        AcquisitionError::ProcessExited { pid: 403 }
    );
    assert_eq!(
        adapter.read_at(&process, 0, &mut [0_u8; 4]).unwrap_err(),
        AcquisitionError::ProcessExited { pid: 403 }
    );
}

#[test]
fn process_identity_parsing_tolerates_parentheses_inside_the_stat_name() {
    let temp = tempfile::tempdir().unwrap();
    candidate(
        temp.path(),
        404,
        "Warframe.x64.ex",
        "/games/Warframe/Downloaded/Public/Warframe.x64.exe",
    );
    write_stat(temp.path(), 404, "War(frame).x64.ex", 777);

    assert_eq!(
        LinuxProc::at(temp.path())
            .discover()
            .unwrap()
            .map(GameProcess::pid),
        Some(404)
    );
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

#[test]
fn resetting_recent_write_tracking_writes_the_soft_dirty_command() {
    let temp = tempfile::tempdir().unwrap();
    write_file(temp.path(), "700/clear_refs", b"");

    LinuxProc::at(temp.path())
        .reset_recent_writes(&GameProcess::new(700))
        .unwrap();

    assert_eq!(fs::read(temp.path().join("700/clear_refs")).unwrap(), b"4");
}

#[test]
fn recently_written_regions_coalesce_only_present_soft_dirty_pages() {
    let temp = tempfile::tempdir().unwrap();
    write_file(
        temp.path(),
        "701/maps",
        concat!(
            "1000-5000 rw-p 00000000 00:00 0\n",
            "9000-a000 r--p 00000000 00:00 0\n",
        ),
    );
    let mut pagemap = vec![0_u8; 10 * 8];
    for page in [1_usize, 2, 4] {
        let entry = (1_u64 << 63) | (1_u64 << 55);
        pagemap[page * 8..page * 8 + 8].copy_from_slice(&entry.to_le_bytes());
    }
    write_file(temp.path(), "701/pagemap", pagemap);

    let regions = LinuxProc::at(temp.path())
        .recently_written_regions(&GameProcess::new(701))
        .unwrap();

    assert_eq!(
        regions,
        vec![
            ReadableRegion::classified(0x1000, 0x2000, RegionScanPriority::WritableAnonymous,),
            ReadableRegion::classified(0x4000, 0x1000, RegionScanPriority::WritableAnonymous,),
        ]
    );
}
