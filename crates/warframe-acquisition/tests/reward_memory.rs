use std::{collections::BTreeMap, sync::Mutex, time::Duration};

use warframe_acquisition::{
    AcquisitionError, GameProcess, MemoryReader, ReadableRegion, RegionScanPriority,
    RewardMemoryScanner, RewardNeedle, RewardRepresentation, RewardResolution,
    resolve_current_reward_choices, resolve_reward_choices, resolve_reward_choices_with_anchor,
};

struct FixtureMemory {
    regions: Vec<ReadableRegion>,
    bytes: BTreeMap<u64, Vec<u8>>,
    reads: Mutex<Vec<u64>>,
}

impl MemoryReader for FixtureMemory {
    fn readable_regions(
        &self,
        _process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        Ok(self.regions.clone())
    }

    fn read_at(
        &self,
        _process: &GameProcess,
        address: u64,
        buffer: &mut [u8],
    ) -> Result<usize, AcquisitionError> {
        self.reads.lock().unwrap().push(address);
        let Some((start, bytes)) = self
            .bytes
            .range(..=address)
            .next_back()
            .filter(|(start, bytes)| address < **start + bytes.len() as u64)
        else {
            return Ok(0);
        };
        let offset = usize::try_from(address - *start).unwrap();
        let len = (bytes.len() - offset).min(buffer.len());
        buffer[..len].copy_from_slice(&bytes[offset..offset + len]);
        Ok(len)
    }
}

fn candidate() -> RewardNeedle {
    RewardNeedle::new(
        "Perigale Prime Receiver",
        ["/Lotus/StoreItems/PerigalePrimeReceiver"],
    )
    .unwrap()
}

#[test]
fn finds_display_names_and_internal_paths_across_chunk_boundaries() {
    let display = b"padding-Perigale Prime Receiver-tail".to_vec();
    let path = b"padding-/Lotus/StoreItems/PerigalePrimeReceiver-tail".to_vec();
    let memory = FixtureMemory {
        regions: vec![
            ReadableRegion::classified(
                0x1000,
                display.len(),
                RegionScanPriority::WritableAnonymous,
            ),
            ReadableRegion::classified(
                0x2000,
                path.len(),
                RegionScanPriority::WritablePrivateFileBacked,
            ),
        ],
        bytes: BTreeMap::from([(0x1000, display), (0x2000, path)]),
        reads: Mutex::new(Vec::new()),
    };
    let scanner = RewardMemoryScanner::new(11, 4096, Duration::from_secs(1));

    let fingerprint = scanner
        .fingerprint(&memory, &GameProcess::new(7), &[candidate()])
        .unwrap();

    assert!(fingerprint.hits().iter().any(|hit| {
        hit.choice_name() == "Perigale Prime Receiver"
            && hit.representation() == RewardRepresentation::DisplayName
    }));
    assert!(fingerprint.hits().iter().any(|hit| {
        hit.choice_name() == "Perigale Prime Receiver"
            && hit.representation() == RewardRepresentation::InternalPath
    }));
}

#[test]
fn scans_writable_anonymous_regions_before_file_backed_regions() {
    let anonymous = b"Perigale Prime Receiver".to_vec();
    let file_backed = b"Perigale Prime Receiver".to_vec();
    let memory = FixtureMemory {
        regions: vec![
            ReadableRegion::classified(0x9000, file_backed.len(), RegionScanPriority::FileBacked),
            ReadableRegion::classified(
                0x3000,
                anonymous.len(),
                RegionScanPriority::WritableAnonymous,
            ),
        ],
        bytes: BTreeMap::from([(0x3000, anonymous), (0x9000, file_backed)]),
        reads: Mutex::new(Vec::new()),
    };
    let scanner = RewardMemoryScanner::new(64, 4096, Duration::from_secs(1));

    scanner
        .fingerprint(&memory, &GameProcess::new(7), &[candidate()])
        .unwrap();

    assert_eq!(memory.reads.lock().unwrap().first().copied(), Some(0x3000));
}

#[test]
fn scans_newer_high_address_writable_regions_first() {
    let memory = FixtureMemory {
        regions: vec![
            ReadableRegion::classified(0x3000, 32, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(0x9000, 32, RegionScanPriority::WritableAnonymous),
        ],
        bytes: BTreeMap::from([(0x3000, vec![0; 32]), (0x9000, vec![0; 32])]),
        reads: Mutex::new(Vec::new()),
    };
    RewardMemoryScanner::new(64, 4096, Duration::from_secs(1))
        .fingerprint(&memory, &GameProcess::new(7), &[candidate()])
        .unwrap();

    assert_eq!(memory.reads.lock().unwrap().first().copied(), Some(0x9000));
}

#[test]
fn scans_live_ui_heap_before_unrelated_higher_mappings() {
    let memory = FixtureMemory {
        regions: vec![
            ReadableRegion::classified(0x6000_0000, 32, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(0x1900_0000, 32, RegionScanPriority::WritableAnonymous),
        ],
        bytes: BTreeMap::from([(0x6000_0000, vec![0; 32]), (0x1900_0000, vec![0; 32])]),
        reads: Mutex::new(Vec::new()),
    };
    RewardMemoryScanner::new(64, 32, Duration::from_secs(1))
        .fingerprint(&memory, &GameProcess::new(7), &[candidate()])
        .unwrap();

    assert_eq!(
        memory.reads.lock().unwrap().first().copied(),
        Some(0x1900_0000)
    );
}

#[test]
fn scans_reward_card_heap_before_the_general_live_ui_heap() {
    let memory = FixtureMemory {
        regions: vec![
            ReadableRegion::classified(0x1900_0000, 32, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(0x4e34_0000, 32, RegionScanPriority::WritableAnonymous),
        ],
        bytes: BTreeMap::from([(0x1900_0000, vec![0; 32]), (0x4e34_0000, vec![0; 32])]),
        reads: Mutex::new(Vec::new()),
    };
    RewardMemoryScanner::new(64, 32, Duration::from_secs(1))
        .fingerprint(&memory, &GameProcess::new(7), &[candidate()])
        .unwrap();

    assert_eq!(
        memory.reads.lock().unwrap().first().copied(),
        Some(0x4e34_0000)
    );
}

#[test]
fn clips_a_live_ui_mapping_at_the_priority_band_boundary() {
    let oversized_start = 0x2700_0000;
    let oversized_len = 32 * 1024 * 1024;
    let lower_start = 0x1900_0000;
    let memory = FixtureMemory {
        regions: vec![
            ReadableRegion::classified(
                oversized_start,
                oversized_len,
                RegionScanPriority::WritableAnonymous,
            ),
            ReadableRegion::classified(lower_start, 32, RegionScanPriority::WritableAnonymous),
        ],
        bytes: BTreeMap::from([
            (oversized_start, vec![0; oversized_len]),
            (lower_start, vec![0; 32]),
        ]),
        reads: Mutex::new(Vec::new()),
    };
    let scanner = RewardMemoryScanner::new(1024 * 1024, 17 * 1024 * 1024, Duration::from_secs(1));
    scanner
        .fingerprint(&memory, &GameProcess::new(7), &[candidate()])
        .unwrap();

    assert!(memory.reads.lock().unwrap().contains(&lower_start));
}

fn online_candidates() -> Vec<RewardNeedle> {
    ["Perigale", "Burston", "Trumna", "Forma", "Lex"]
        .into_iter()
        .map(|name| RewardNeedle::new(name, [name]).unwrap())
        .collect()
}

fn card_candidates() -> Vec<RewardNeedle> {
    [
        ("Local", "/Lotus/LocalReward"),
        ("Second", "/Lotus/SecondReward"),
        ("Third", "/Lotus/ThirdReward"),
        ("Fourth", "/Lotus/FourthReward"),
    ]
    .into_iter()
    .map(|(name, path)| RewardNeedle::new(name, [path]).unwrap())
    .collect()
}

fn card_fingerprint(entries: &[(usize, &str)]) -> warframe_acquisition::RewardFingerprint {
    let mut bytes = vec![b'.'; 2048];
    for (sequence, (slot, path)) in entries.iter().enumerate() {
        let start = 128 + sequence * 384;
        let tag = format!("RewardList.Item{slot}.TagContainer.Tag1.IconText");
        bytes[start..start + tag.len()].copy_from_slice(tag.as_bytes());
        let path_start = start + 80;
        bytes[path_start..path_start + path.len()].copy_from_slice(path.as_bytes());
    }
    let memory = FixtureMemory {
        regions: vec![ReadableRegion::classified(
            0x1000,
            bytes.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        bytes: BTreeMap::from([(0x1000, bytes)]),
        reads: Mutex::new(Vec::new()),
    };
    RewardMemoryScanner::new(64, 4096, Duration::from_secs(1))
        .fingerprint(&memory, &GameProcess::new(9), &card_candidates())
        .unwrap()
}

#[test]
fn anchored_card_slots_resolve_four_rewards_in_screen_order() {
    let baseline = card_fingerprint(&[]);
    let current = card_fingerprint(&[
        (2, "/Lotus/SecondReward"),
        (3, "/Lotus/ThirdReward"),
        (4, "/Lotus/FourthReward"),
    ]);

    assert_eq!(
        resolve_reward_choices_with_anchor(&baseline, &current, 4, 256, Some("Local")),
        RewardResolution::Confirmed {
            choices: vec![
                "Local".into(),
                "Second".into(),
                "Third".into(),
                "Fourth".into(),
            ],
            region_start: 0,
        }
    );
}

#[test]
fn anchored_card_slots_reject_a_missing_remote_slot() {
    let baseline = card_fingerprint(&[]);
    let current = card_fingerprint(&[(2, "/Lotus/SecondReward"), (4, "/Lotus/FourthReward")]);

    assert_eq!(
        resolve_reward_choices_with_anchor(&baseline, &current, 4, 256, Some("Local")),
        RewardResolution::Incomplete
    );
}

#[test]
fn anchored_card_slots_reject_conflicting_values_for_one_slot() {
    let baseline = card_fingerprint(&[]);
    let current = card_fingerprint(&[
        (2, "/Lotus/SecondReward"),
        (2, "/Lotus/ThirdReward"),
        (3, "/Lotus/ThirdReward"),
        (4, "/Lotus/FourthReward"),
    ]);

    assert_eq!(
        resolve_reward_choices_with_anchor(&baseline, &current, 4, 256, Some("Local")),
        RewardResolution::Ambiguous
    );
}

#[test]
fn anchored_card_slots_ignore_non_binding_item_fields() {
    let mut bytes = vec![b'.'; 2048];
    let false_tag = b"RewardList.Item2.ShadowContainer.ImageShadow";
    bytes[128..128 + false_tag.len()].copy_from_slice(false_tag);
    bytes[208..208 + b"/Lotus/SecondReward".len()].copy_from_slice(b"/Lotus/SecondReward");
    for (sequence, (slot, path)) in [
        (3, "/Lotus/Language/ThirdReward"),
        (4, "/Lotus/Language/FourthReward"),
    ]
    .into_iter()
    .enumerate()
    {
        let start = 640 + sequence * 384;
        let tag = format!("RewardList.Item{slot}.TagContainer.Tag1.IconText");
        bytes[start..start + tag.len()].copy_from_slice(tag.as_bytes());
        bytes[start + 80..start + 80 + path.len()].copy_from_slice(path.as_bytes());
    }
    let memory = FixtureMemory {
        regions: vec![ReadableRegion::classified(
            0x1000,
            bytes.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        bytes: BTreeMap::from([(0x1000, bytes)]),
        reads: Mutex::new(Vec::new()),
    };
    let current = RewardMemoryScanner::new(64, 4096, Duration::from_secs(1))
        .fingerprint(&memory, &GameProcess::new(9), &card_candidates())
        .unwrap();

    assert_eq!(
        resolve_reward_choices_with_anchor(&card_fingerprint(&[]), &current, 4, 256, Some("Local"),),
        RewardResolution::Incomplete
    );
}

#[test]
fn anchored_card_slots_resolve_three_rendered_rewards() {
    let baseline = card_fingerprint(&[]);
    let current = card_fingerprint(&[(2, "/Lotus/SecondReward"), (3, "/Lotus/ThirdReward")]);

    assert_eq!(
        resolve_reward_choices_with_anchor(&baseline, &current, 3, 256, Some("Local")),
        RewardResolution::Confirmed {
            choices: vec!["Local".into(), "Second".into(), "Third".into()],
            region_start: 0,
        }
    );
}

#[test]
fn anchored_card_slots_ignore_unchanged_stale_slot_bindings() {
    let baseline = card_fingerprint(&[(2, "/Lotus/FourthReward")]);
    let current = card_fingerprint(&[
        (2, "/Lotus/FourthReward"),
        (2, "/Lotus/SecondReward"),
        (3, "/Lotus/ThirdReward"),
        (4, "/Lotus/FourthReward"),
    ]);

    assert_eq!(
        resolve_reward_choices_with_anchor(&baseline, &current, 4, 256, Some("Local")),
        RewardResolution::Confirmed {
            choices: vec![
                "Local".into(),
                "Second".into(),
                "Third".into(),
                "Fourth".into(),
            ],
            region_start: 0,
        }
    );
}

#[test]
fn current_resolution_selects_a_tight_choice_cluster_among_stale_region_strings() {
    let mut current_bytes = vec![b'.'; 2048];
    current_bytes[40..43].copy_from_slice(b"Lex");
    current_bytes[1200..1207].copy_from_slice(b"Burston");
    current_bytes[1240..1245].copy_from_slice(b"Forma");
    current_bytes[1280..1288].copy_from_slice(b"Perigale");
    current_bytes[1320..1326].copy_from_slice(b"Trumna");
    let current = fingerprint(
        current_bytes,
        vec![ReadableRegion::classified(
            0x1000,
            2048,
            RegionScanPriority::WritableAnonymous,
        )],
    );

    assert_eq!(
        resolve_current_reward_choices(&current, 4, 256),
        RewardResolution::Confirmed {
            choices: vec![
                "Burston".into(),
                "Forma".into(),
                "Perigale".into(),
                "Trumna".into(),
            ],
            region_start: 0x1000,
        }
    );
}

#[test]
fn current_resolution_rejects_near_equal_four_of_five_interpretations() {
    let mut current_bytes = vec![b'.'; 2048];
    current_bytes[1000..1003].copy_from_slice(b"Lex");
    current_bytes[1040..1047].copy_from_slice(b"Burston");
    current_bytes[1080..1085].copy_from_slice(b"Forma");
    current_bytes[1120..1128].copy_from_slice(b"Perigale");
    current_bytes[1170..1176].copy_from_slice(b"Trumna");
    let current = fingerprint(
        current_bytes,
        vec![ReadableRegion::classified(
            0x1000,
            2048,
            RegionScanPriority::WritableAnonymous,
        )],
    );

    assert_eq!(
        resolve_current_reward_choices(&current, 4, 256),
        RewardResolution::Ambiguous
    );
}

fn fingerprint(
    bytes: Vec<u8>,
    regions: Vec<ReadableRegion>,
) -> warframe_acquisition::RewardFingerprint {
    let memory = FixtureMemory {
        bytes: regions
            .iter()
            .scan(0_usize, |offset, region| {
                let start = *offset;
                let end = start + region.len();
                *offset = end;
                Some((region.start(), bytes[start..end].to_vec()))
            })
            .collect(),
        regions,
        reads: Mutex::new(Vec::new()),
    };
    RewardMemoryScanner::new(64, 4096, Duration::from_secs(1))
        .fingerprint(&memory, &GameProcess::new(9), &online_candidates())
        .unwrap()
}

#[test]
fn temporal_resolution_ignores_catalog_strings_and_orders_the_new_cluster() {
    let mut baseline_bytes = vec![b'.'; 1024];
    baseline_bytes[20..28].copy_from_slice(b"Perigale");
    baseline_bytes[80..87].copy_from_slice(b"Burston");
    let mut current_bytes = baseline_bytes.clone();
    current_bytes[500..507].copy_from_slice(b"Burston");
    current_bytes[540..545].copy_from_slice(b"Forma");
    current_bytes[580..588].copy_from_slice(b"Perigale");
    current_bytes[620..626].copy_from_slice(b"Trumna");
    let regions = vec![ReadableRegion::classified(
        0x1000,
        1024,
        RegionScanPriority::WritableAnonymous,
    )];
    let baseline = fingerprint(baseline_bytes, regions.clone());
    let current = fingerprint(current_bytes, regions);

    assert_eq!(
        resolve_reward_choices(&baseline, &current, 4, 256),
        RewardResolution::Confirmed {
            choices: vec![
                "Burston".into(),
                "Forma".into(),
                "Perigale".into(),
                "Trumna".into()
            ],
            region_start: 0x1000,
        }
    );
}

#[test]
fn temporal_resolution_confirms_the_rendered_three_choice_count() {
    let baseline_bytes = vec![b'.'; 1024];
    let mut current_bytes = baseline_bytes.clone();
    current_bytes[500..507].copy_from_slice(b"Burston");
    current_bytes[540..545].copy_from_slice(b"Forma");
    current_bytes[580..588].copy_from_slice(b"Perigale");
    let regions = vec![ReadableRegion::classified(
        0x1000,
        1024,
        RegionScanPriority::WritableAnonymous,
    )];

    assert_eq!(
        resolve_reward_choices(
            &fingerprint(baseline_bytes, regions.clone()),
            &fingerprint(current_bytes, regions),
            3,
            256,
        ),
        RewardResolution::Confirmed {
            choices: vec!["Burston".into(), "Forma".into(), "Perigale".into()],
            region_start: 0x1000,
        }
    );
}

#[test]
fn temporal_resolution_rejects_equally_complete_competing_clusters() {
    let baseline = fingerprint(
        vec![b'.'; 2048],
        vec![
            ReadableRegion::classified(0x1000, 1024, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(0x3000, 1024, RegionScanPriority::WritableAnonymous),
        ],
    );
    let mut current = vec![b'.'; 2048];
    for base in [0_usize, 1024] {
        current[base + 100..base + 108].copy_from_slice(b"Perigale");
        current[base + 140..base + 147].copy_from_slice(b"Burston");
        current[base + 180..base + 186].copy_from_slice(b"Trumna");
        current[base + 220..base + 225].copy_from_slice(b"Forma");
    }
    let current = fingerprint(
        current,
        vec![
            ReadableRegion::classified(0x1000, 1024, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(0x3000, 1024, RegionScanPriority::WritableAnonymous),
        ],
    );

    assert_eq!(
        resolve_reward_choices(&baseline, &current, 4, 256),
        RewardResolution::Ambiguous
    );
}

#[test]
fn temporal_resolution_rejects_an_exact_choice_count_scattered_across_regions() {
    let baseline = fingerprint(
        vec![b'.'; 1024],
        vec![
            ReadableRegion::classified(0x1000, 256, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(0x3000, 256, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(0x5000, 512, RegionScanPriority::WritableAnonymous),
        ],
    );
    let mut current = vec![b'.'; 1024];
    current[40..47].copy_from_slice(b"Burston");
    current[300..305].copy_from_slice(b"Forma");
    current[600..608].copy_from_slice(b"Perigale");
    current[700..706].copy_from_slice(b"Trumna");
    current[760..768].copy_from_slice(b"Perigale");
    let current = fingerprint(
        current,
        vec![
            ReadableRegion::classified(0x1000, 256, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(0x3000, 256, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(0x5000, 512, RegionScanPriority::WritableAnonymous),
        ],
    );

    assert_eq!(
        resolve_reward_choices(&baseline, &current, 4, 128),
        RewardResolution::Incomplete
    );
}

#[test]
fn current_cluster_can_recover_when_the_baseline_was_contaminated() {
    let mut current_bytes = vec![b'.'; 1024];
    current_bytes[500..507].copy_from_slice(b"Burston");
    current_bytes[540..545].copy_from_slice(b"Forma");
    current_bytes[580..588].copy_from_slice(b"Perigale");
    current_bytes[620..626].copy_from_slice(b"Trumna");
    let current = fingerprint(
        current_bytes,
        vec![ReadableRegion::classified(
            0x1000,
            1024,
            RegionScanPriority::WritableAnonymous,
        )],
    );

    assert_eq!(
        resolve_current_reward_choices(&current, 4, 256),
        RewardResolution::Confirmed {
            choices: vec![
                "Burston".into(),
                "Forma".into(),
                "Perigale".into(),
                "Trumna".into(),
            ],
            region_start: 0x1000,
        }
    );
}

#[test]
fn confirmation_rereads_only_the_selected_region_and_preserves_order() {
    let mut selected = vec![b'.'; 512];
    selected[100..108].copy_from_slice(b"Perigale");
    selected[140..147].copy_from_slice(b"Burston");
    selected[180..186].copy_from_slice(b"Trumna");
    selected[220..225].copy_from_slice(b"Forma");
    let memory = FixtureMemory {
        regions: vec![
            ReadableRegion::classified(0x1000, 512, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(0x3000, 512, RegionScanPriority::WritableAnonymous),
        ],
        bytes: BTreeMap::from([(0x1000, vec![b'x'; 512]), (0x3000, selected)]),
        reads: Mutex::new(Vec::new()),
    };
    let scanner = RewardMemoryScanner::new(128, 4096, Duration::from_secs(1));

    let resolution = scanner
        .confirm_region(
            &memory,
            &GameProcess::new(9),
            &online_candidates(),
            0x3000,
            512,
            4,
            256,
        )
        .unwrap();

    assert_eq!(
        resolution,
        RewardResolution::Confirmed {
            choices: vec![
                "Perigale".into(),
                "Burston".into(),
                "Trumna".into(),
                "Forma".into()
            ],
            region_start: 0x3000,
        }
    );
    assert!(
        memory
            .reads
            .lock()
            .unwrap()
            .iter()
            .all(|address| *address >= 0x3000)
    );
}
