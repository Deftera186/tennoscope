use std::{collections::BTreeMap, env, time::Duration};

use warframe_acquisition::{
    ProcessDiscovery, RewardMemoryScanner, RewardNeedle, RewardRepresentation, RewardResolution,
    resolve_current_reward_choices,
};

#[cfg(unix)]
use warframe_acquisition::LinuxProc as GameMemory;
#[cfg(windows)]
use warframe_acquisition::WindowsProc as GameMemory;

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
    let procfs = GameMemory::new();
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
    let mut clusters = BTreeMap::<u64, Vec<&str>>::new();
    for hit in fingerprint.hits() {
        let names = clusters.entry(hit.region_start()).or_default();
        if !names.contains(&hit.choice_name()) {
            names.push(hit.choice_name());
        }
    }
    for (ordinal, names) in clusters
        .values()
        .filter(|names| names.len() >= 2)
        .enumerate()
    {
        println!(
            "cluster={ordinal} distinct_candidates={} names={names:?}",
            names.len()
        );
    }
    let clustered_regions = clusters
        .iter()
        .filter(|(_, names)| names.len() >= 4)
        .map(|(start, _)| *start)
        .collect::<Vec<_>>();
    for hit in fingerprint
        .hits()
        .iter()
        .filter(|hit| clustered_regions.contains(&hit.region_start()))
    {
        println!(
            "hit region={} offset={} priority={:?} representation={:?} name={:?}",
            clustered_regions
                .iter()
                .position(|start| *start == hit.region_start())
                .unwrap_or(usize::MAX),
            hit.address() - hit.region_start(),
            hit.priority(),
            hit.representation(),
            hit.choice_name()
        );
    }
    if let Some(expected) = env::var("TENN_OSCOPE_EXPECTED_CHOICES")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
    {
        let (status, choices) =
            match resolve_current_reward_choices(&fingerprint, expected, 2 * 1024 * 1024) {
                RewardResolution::Confirmed { choices, .. } => ("confirmed", choices),
                RewardResolution::Incomplete => ("incomplete", Vec::new()),
                RewardResolution::Ambiguous => ("ambiguous", Vec::new()),
                RewardResolution::TimedOut => ("timed_out", Vec::new()),
            };
        println!(
            "resolution={status} choice_count={} choices={choices:?}",
            choices.len()
        );
    }
}
