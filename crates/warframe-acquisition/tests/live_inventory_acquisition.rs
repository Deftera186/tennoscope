#![cfg(target_os = "linux")]

use std::{env, fs, time::Instant};

use warframe_acquisition::{
    AuthorizationScanner, CatalogIndex, InventoryHttpTransport, InventoryJsonDecoder,
    InventoryTransport, LinuxProc, ProcessDiscovery, SnapshotDecoder,
};

/// Opt-in live transaction. Output contains stage names and aggregate counts
/// only: no PID, address, path, credentials, response bytes, or item names.
#[test]
#[ignore = "requires a running logged-in Warframe session and network access"]
fn live_inventory_acquisition_emits_only_safe_metadata() {
    let process_access = LinuxProc::new();
    let process = process_access
        .discover()
        .expect("game discovery should access procfs")
        .expect("Warframe should be running");
    println!("game_discovery=ready");

    let scan_started = Instant::now();
    let authorization = AuthorizationScanner::new(1024 * 1024)
        .scan(&process_access, &process)
        .expect("ephemeral authorization should be discoverable");
    println!("authorization_discovery=ready");
    println!(
        "authorization_scan_milliseconds={}",
        scan_started.elapsed().as_millis()
    );

    let response = InventoryHttpTransport::new()
        .expect("HTTPS client policy should initialize")
        .fetch(&authorization)
        .expect("inventory endpoint should return a bounded response");
    println!("endpoint_fetch=ready");
    println!("response_bytes={}", response.len());

    let catalog = env::var_os("WFCD_CATALOG_PATH").map(|path| {
        let bytes = fs::read(path).expect("catalog file should be readable");
        CatalogIndex::from_wfcd_json(&bytes).expect("catalog should be valid WFCD All.json")
    });
    let snapshot = match catalog.as_ref() {
        Some(catalog) => InventoryJsonDecoder::with_catalog(catalog).decode(&response),
        None => InventoryJsonDecoder::default().decode(&response),
    }
    .expect("complete response should pass schema validation");
    println!("schema_validation=ready");
    println!("snapshot_entries={}", snapshot.entries().len());
    println!(
        "owned_entries={}",
        snapshot
            .entries()
            .iter()
            .filter(|entry| entry.quantity > 0)
            .count()
    );
    println!(
        "mastered_entries={}",
        snapshot
            .entries()
            .iter()
            .filter(|entry| entry.mastered)
            .count()
    );
}
