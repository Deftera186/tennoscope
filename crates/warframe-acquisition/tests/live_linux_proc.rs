#![cfg(target_os = "linux")]

use std::{
    thread,
    time::{Duration, Instant},
};

use warframe_acquisition::{LinuxProc, MemoryReader, ProcessDiscovery};

/// Opt-in smoke probe for a running native or Proton Warframe session.
///
/// Output is intentionally limited to stage names and aggregate counts. It
/// never prints mapped paths, addresses, memory bytes, or authorization data.
#[test]
#[ignore = "requires a running Warframe session and explicit local opt-in"]
fn live_linux_proc_probe_emits_only_safe_stage_metadata() {
    let adapter = LinuxProc::new();
    let process = adapter
        .discover()
        .expect("game discovery should access procfs")
        .expect("Warframe should be running");
    println!("game_discovery=ready");

    let regions = adapter
        .readable_regions(&process)
        .expect("readable maps should be available");
    println!("readable_regions={}", regions.len());

    let mut readable = 0_usize;
    let mut unavailable = 0_usize;
    let mut buffer = [0_u8; 64];
    for region in &regions {
        match adapter.read_at(&process, region.start(), &mut buffer) {
            Ok(0) => unavailable += 1,
            Ok(_) => readable += 1,
            Err(_) => unavailable += 1,
        }
    }
    buffer.fill(0);

    assert!(
        readable > 0,
        "at least one mapped region should be readable"
    );
    println!("memory_read=ready");
    println!("readable_samples={readable}");
    println!("unavailable_samples={unavailable}");
}

#[test]
#[ignore = "requires a running Warframe session and explicit local opt-in"]
fn live_recent_memory_snapshot_reports_safe_timing() {
    let adapter = LinuxProc::new();
    let process = adapter
        .discover()
        .unwrap()
        .expect("Warframe should be running");
    adapter.reset_recent_writes(&process).unwrap();
    thread::sleep(Duration::from_millis(100));

    let regions_started = Instant::now();
    let regions = adapter.recently_written_regions(&process).unwrap();
    let regions_elapsed = regions_started.elapsed();
    let dirty_bytes = regions.iter().map(|region| region.len()).sum::<usize>();

    let snapshot_started = Instant::now();
    let snapshot = adapter
        .recently_written_snapshot(&process)
        .unwrap()
        .expect("Linux should provide a snapshot");
    let snapshot_elapsed = snapshot_started.elapsed();
    println!(
        "dirty_ranges={} dirty_mib={} enumerate_ms={} snapshot_ranges={} snapshot_ms={}",
        regions.len(),
        dirty_bytes / (1024 * 1024),
        regions_elapsed.as_millis(),
        snapshot.len(),
        snapshot_elapsed.as_millis(),
    );
}
