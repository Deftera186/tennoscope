use std::io::{self, Read};

use warframe_acquisition::{InventoryJsonDecoder, SnapshotDecoder};

fn main() {
    let mut response = Vec::new();
    io::stdin()
        .read_to_end(&mut response)
        .expect("inventory response should be readable from stdin");
    let snapshot = InventoryJsonDecoder
        .decode(&response)
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
