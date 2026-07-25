use std::{env, time::Duration};

use warframe_acquisition::{
    LinuxProc, ProcessDiscovery, RewardMemoryScanner, RewardNeedle, RewardRepresentation,
    RewardResolution, resolve_current_reward_choices,
};

fn main() {
    let names = env::args().skip(1).collect::<Vec<_>>();
    assert!(
        !names.is_empty(),
        "provide one or more reward display names"
    );
    let candidates = names
        .into_iter()
        .map(|name| RewardNeedle::from_paths(name, Vec::new()).expect("valid reward name"))
        .collect::<Vec<_>>();
    let procfs = LinuxProc::new();
    let process = procfs
        .discover()
        .expect("process discovery")
        .expect("Warframe running");
    let scanner =
        RewardMemoryScanner::new(256 * 1024, 384 * 1024 * 1024, Duration::from_millis(450));
    let fingerprint = scanner
        .fingerprint(&procfs, &process, &candidates)
        .expect("bounded reward scan");

    for candidate in &candidates {
        let display_hits = fingerprint
            .hits()
            .iter()
            .filter(|hit| {
                hit.choice_name() == candidate.choice_name()
                    && hit.representation() == RewardRepresentation::DisplayName
            })
            .count();
        println!(
            "candidate={:?} display_hits={display_hits}",
            candidate.choice_name()
        );
    }
    println!(
        "summary bytes_read={} elapsed_ms={} total_hits={}",
        fingerprint.bytes_read(),
        fingerprint.elapsed().as_millis(),
        fingerprint.hits().len()
    );
    if let Some(expected) = env::var("TENN_OSCOPE_EXPECTED_CHOICES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
    {
        let (status, count) =
            match resolve_current_reward_choices(&fingerprint, expected, 2 * 1024 * 1024) {
                RewardResolution::Confirmed { choices, .. } => ("confirmed", choices.len()),
                RewardResolution::Incomplete => ("incomplete", 0),
                RewardResolution::Ambiguous => ("ambiguous", 0),
                RewardResolution::TimedOut => ("timed_out", 0),
            };
        println!("resolution={status} choice_count={count}");
    }
}
