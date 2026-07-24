use std::{
    env, fs,
    io::{self, Read},
};

use warframe_acquisition::{CatalogIndex, InventoryJsonDecoder, SnapshotDecoder};

fn main() {
    let mut response = Vec::new();
    io::stdin()
        .read_to_end(&mut response)
        .expect("inventory response should be readable from stdin");
    let catalog = env::args_os().nth(1).map(|path| {
        let bytes = fs::read(path).expect("catalog file should be readable");
        CatalogIndex::from_wfcd_json(&bytes).expect("catalog JSON should be valid WFCD All.json")
    });
    let snapshot = match catalog.as_ref() {
        Some(catalog) => InventoryJsonDecoder::with_catalog(catalog).decode(&response),
        None => InventoryJsonDecoder::default().decode(&response),
    }
    .expect("inventory response should be complete and valid");
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
}
