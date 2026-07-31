#![cfg(windows)]

use warframe_acquisition::{MemoryReader, ProcessDiscovery, WindowsProc};

/// Opt-in smoke probe for a running native Warframe session.
///
/// This is the only thing that can answer the two questions CI cannot: does `OpenProcess` with
/// `PROCESS_VM_READ` succeed against Warframe without elevation, and does a full region rescan --
/// which is all this backend has, there being no soft-dirty equivalent -- stay fast enough to poll.
///
/// Output is limited to stage names and aggregate counts. It never prints mapped paths, addresses,
/// memory bytes, or authorization data.
#[test]
#[ignore = "requires a running Warframe session and explicit local opt-in"]
fn live_windows_proc_probe_emits_only_safe_stage_metadata() {
    let adapter = WindowsProc::new();
    let process = adapter
        .discover()
        .expect("game discovery should enumerate processes")
        .expect("Warframe should be running");
    println!("game_discovery=ready");

    let started = std::time::Instant::now();
    let regions = adapter
        .readable_regions(&process)
        .expect("readable regions should be available without elevation");
    println!("readable_regions={}", regions.len());
    println!("enumerate_ms={}", started.elapsed().as_millis());

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

/// The trait defaults are load bearing here: without soft-dirty tracking, `recently_written_regions`
/// must degrade to a full rescan rather than returning nothing, and the snapshot must be absent
/// rather than empty -- an empty snapshot would read as "the game wrote nothing".
#[test]
#[ignore = "requires a running Warframe session and explicit local opt-in"]
fn live_recent_writes_degrade_to_a_full_rescan() {
    let adapter = WindowsProc::new();
    let process = adapter
        .discover()
        .unwrap()
        .expect("Warframe should be running");
    adapter
        .reset_recent_writes(&process)
        .expect("resetting write tracking should be a no-op, not an error");

    let started = std::time::Instant::now();
    let recent = adapter.recently_written_regions(&process).unwrap();
    println!(
        "rescan_regions={} rescan_ms={}",
        recent.len(),
        started.elapsed().as_millis()
    );
    assert_eq!(
        recent.len(),
        adapter.readable_regions(&process).unwrap().len(),
        "every readable region must be rescanned when write tracking is unavailable"
    );
    assert!(
        adapter
            .recently_written_snapshot(&process)
            .unwrap()
            .is_none(),
        "no snapshot is available on Windows; None is what tells the scanner to read regions itself"
    );
}
