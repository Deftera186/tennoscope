#![cfg(target_os = "linux")]

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
