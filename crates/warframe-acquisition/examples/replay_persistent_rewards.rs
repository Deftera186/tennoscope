use std::{env, time::Duration};

use warframe_acquisition::{LinuxProc, PersistentRewardResolver, ProcessDiscovery, RewardNeedle};

fn main() {
    let names = env::args().skip(1).collect::<Vec<_>>();
    assert!(
        names.len() >= 2,
        "provide the visible reward names in order"
    );
    let candidates = names
        .iter()
        .map(|name| RewardNeedle::from_paths(name.clone(), Vec::new()).expect("valid reward name"))
        .collect::<Vec<_>>();
    let memory = LinuxProc::new();
    let process = memory
        .discover()
        .expect("process discovery")
        .expect("Warframe running");
    let resolution =
        PersistentRewardResolver::new(256 * 1024, 512 * 1024 * 1024, Duration::from_millis(2_500))
            .resolve(&memory, &process, &candidates, names.len())
            .expect("persistent reward replay");
    println!("resolution={resolution:?}");
}
