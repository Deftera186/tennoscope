use std::{collections::BTreeMap, sync::Mutex, time::Duration};

use warframe_acquisition::{
    AcquisitionError, GameProcess, MemoryReader, MemorySnapshotRegion, ReadableRegion,
    RegionScanPriority, RewardMemoryScanner, RewardNeedle, RewardRepresentation, RewardResolution,
    resolve_current_reward_choices, resolve_reward_choices,
};

struct FixtureMemory {
    regions: Vec<ReadableRegion>,
    bytes: BTreeMap<u64, Vec<u8>>,
    reads: Mutex<Vec<u64>>,
}

struct RecentFixtureMemory {
    readable: Vec<ReadableRegion>,
    recent: Vec<ReadableRegion>,
    bytes: BTreeMap<u64, Vec<u8>>,
    snapshot: Option<Vec<MemorySnapshotRegion>>,
}

impl MemoryReader for RecentFixtureMemory {
    fn readable_regions(
        &self,
        _process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        Ok(self.readable.clone())
    }

    fn recently_written_regions(
        &self,
        _process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        Ok(self.recent.clone())
    }

    fn recently_written_snapshot(
        &self,
        _process: &GameProcess,
    ) -> Result<Option<Vec<MemorySnapshotRegion>>, AcquisitionError> {
        Ok(self.snapshot.clone())
    }

    fn read_at(
        &self,
        _process: &GameProcess,
        address: u64,
        buffer: &mut [u8],
    ) -> Result<usize, AcquisitionError> {
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
            ReadableRegion::classified(0x5000_0000, 32, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(0x1900_0000, 32, RegionScanPriority::WritableAnonymous),
        ],
        bytes: BTreeMap::from([(0x5000_0000, vec![0; 32]), (0x1900_0000, vec![0; 32])]),
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
fn player_record_scan_reaches_a_high_response_heap_before_a_large_ui_heap() {
    let identity = "de1e7ed00000000000000006";
    let mut response_heap = vec![0_u8; 512];
    response_heap[128..152].copy_from_slice(identity.as_bytes());
    response_heap[255..271].copy_from_slice(b"BratonPrimeStock");
    let memory = FixtureMemory {
        regions: vec![
            ReadableRegion::classified(0x1900_0000, 4096, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(
                0x3eda_0000,
                response_heap.len(),
                RegionScanPriority::WritableAnonymous,
            ),
        ],
        bytes: BTreeMap::from([(0x1900_0000, vec![0; 4096]), (0x3eda_0000, response_heap)]),
        reads: Mutex::new(Vec::new()),
    };
    let candidate = RewardNeedle::from_paths(
        "Braton Prime Stock",
        vec!["/Lotus/Types/Recipes/Weapons/WeaponParts/BratonPrimeStock".into()],
    )
    .unwrap();

    assert_eq!(
        RewardMemoryScanner::new(256, 512, Duration::from_secs(1))
            .resolve_player_records(
                &memory,
                &GameProcess::new(9),
                &[candidate],
                &[identity],
                None,
                None,
            )
            .unwrap(),
        RewardResolution::Confirmed {
            choices: vec!["Braton Prime Stock".into()],
            region_start: 0,
        }
    );
    assert_eq!(
        memory.reads.lock().unwrap().first().copied(),
        Some(0x3eda_0000)
    );
}

#[test]
fn player_record_scan_samples_each_response_heap_before_exhausting_one_heap() {
    let identity = "de1e7ed00000000000000006";
    let mut response_heap = vec![0_u8; 512];
    response_heap[0..24].copy_from_slice(identity.as_bytes());
    response_heap[127..143].copy_from_slice(b"BratonPrimeStock");
    let memory = FixtureMemory {
        regions: vec![
            ReadableRegion::classified(
                0x5bf1_0000,
                16 * 1024,
                RegionScanPriority::WritableAnonymous,
            ),
            ReadableRegion::classified(
                0x3eda_0000,
                response_heap.len(),
                RegionScanPriority::WritableAnonymous,
            ),
        ],
        bytes: BTreeMap::from([
            (0x5bf1_0000, vec![0; 16 * 1024]),
            (0x3eda_0000, response_heap),
        ]),
        reads: Mutex::new(Vec::new()),
    };
    let candidate = RewardNeedle::from_paths(
        "Braton Prime Stock",
        vec!["/Lotus/Types/Recipes/Weapons/WeaponParts/BratonPrimeStock".into()],
    )
    .unwrap();

    assert_eq!(
        RewardMemoryScanner::new(128, 6 * 1024, Duration::from_secs(1))
            .resolve_player_records(
                &memory,
                &GameProcess::new(9),
                &[candidate],
                &[identity],
                None,
                None,
            )
            .unwrap(),
        RewardResolution::Confirmed {
            choices: vec!["Braton Prime Stock".into()],
            region_start: 0,
        }
    );
}

#[test]
fn player_record_scan_uses_recently_written_pages_when_available() {
    let identity = "de1e7ed00000000000000006";
    let mut response = vec![0_u8; 512];
    response[0..24].copy_from_slice(identity.as_bytes());
    response[127..143].copy_from_slice(b"BratonPrimeStock");
    let memory = RecentFixtureMemory {
        readable: vec![
            ReadableRegion::classified(0x5bf1_0000, 512, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(
                0x3eda_0000,
                response.len(),
                RegionScanPriority::WritableAnonymous,
            ),
        ],
        recent: vec![ReadableRegion::classified(
            0x3eda_0000,
            response.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        bytes: BTreeMap::from([(0x5bf1_0000, vec![0; 512]), (0x3eda_0000, response)]),
        snapshot: None,
    };
    let candidate = RewardNeedle::from_paths(
        "Braton Prime Stock",
        vec!["/Lotus/Types/Recipes/Weapons/WeaponParts/BratonPrimeStock".into()],
    )
    .unwrap();

    assert_eq!(
        RewardMemoryScanner::new(128, 512, Duration::from_secs(1))
            .resolve_player_records(
                &memory,
                &GameProcess::new(9),
                &[candidate],
                &[identity],
                None,
                None,
            )
            .unwrap(),
        RewardResolution::Confirmed {
            choices: vec!["Braton Prime Stock".into()],
            region_start: 0,
        }
    );
}

#[test]
fn snapshot_player_hit_can_resolve_a_reward_from_the_adjacent_live_page() {
    let identity = "de1e7ed00000000000000006";
    let base = 0x3eda_0000_u64;
    let mut live = vec![0_u8; 16 * 1024];
    live[128..152].copy_from_slice(identity.as_bytes());
    live[5_128..5_144].copy_from_slice(b"BratonPrimeStock");
    let mut dirty_page = vec![0_u8; 4096];
    dirty_page.copy_from_slice(&live[..4096]);
    let memory = RecentFixtureMemory {
        readable: vec![ReadableRegion::classified(
            base,
            live.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        recent: vec![ReadableRegion::classified(
            base,
            dirty_page.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        bytes: BTreeMap::from([(base, live)]),
        snapshot: Some(vec![MemorySnapshotRegion::new(
            base,
            dirty_page,
            RegionScanPriority::WritableAnonymous,
        )]),
    };
    let candidate = RewardNeedle::from_paths(
        "Braton Prime Stock",
        vec!["/Lotus/Types/Recipes/Weapons/WeaponParts/BratonPrimeStock".into()],
    )
    .unwrap();

    assert_eq!(
        RewardMemoryScanner::new(4096, 64 * 1024, Duration::from_secs(1))
            .resolve_player_records(
                &memory,
                &GameProcess::new(9),
                &[candidate],
                &[identity],
                None,
                None,
            )
            .unwrap(),
        RewardResolution::Confirmed {
            choices: vec!["Braton Prime Stock".into()],
            region_start: 0,
        }
    );
}

#[test]
fn low_heap_player_record_scan_reaches_captured_response_heaps_before_high_mappings() {
    let identity = "de1e7ed00000000000000006";
    let mut response_heap = vec![0_u8; 512];
    response_heap[128..152].copy_from_slice(identity.as_bytes());
    response_heap[255..275].copy_from_slice(b"BratonPrimeBlueprint");
    let memory = FixtureMemory {
        regions: vec![
            ReadableRegion::classified(0x5bf1_0000, 4096, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(
                0x1d72_0000,
                response_heap.len(),
                RegionScanPriority::WritableAnonymous,
            ),
        ],
        bytes: BTreeMap::from([(0x5bf1_0000, vec![0; 4096]), (0x1d72_0000, response_heap)]),
        reads: Mutex::new(Vec::new()),
    };
    let candidate = RewardNeedle::from_paths(
        "Braton Prime Blueprint",
        vec!["/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint".into()],
    )
    .unwrap();

    assert!(matches!(
        RewardMemoryScanner::new(256, 512, Duration::from_secs(1))
            .resolve_player_records_from_low_heaps(
                &memory,
                &GameProcess::new(9),
                &[candidate],
                &[identity],
                None,
                None,
            )
            .unwrap(),
        RewardResolution::Confirmed { .. }
    ));
    assert_eq!(
        memory.reads.lock().unwrap().first().copied(),
        Some(0x1d72_0000)
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

#[test]
fn player_records_ignore_a_tighter_stale_reward_cluster_and_preserve_screen_order() {
    let responders = [
        "de1e7ed00000000000000005",
        "de1e7ed0000000000000000a",
        "de1e7ed00000000000000004",
        "de1e7ed00000000000000006",
    ];
    let candidates = [
        "Daikyu Prime Upper Limb",
        "Akbronco Prime Link",
        "Forma Blueprint",
        "Trumna Prime Stock",
        "Vadarya Prime Receiver",
        "Alternox Prime Receiver",
    ]
    .into_iter()
    .map(|name| RewardNeedle::from_paths(name, vec![name.replace(' ', "")]).unwrap())
    .collect::<Vec<_>>();
    let mut bytes = vec![0_u8; 8192];

    // This reproduces the false-positive shape from the live run: four stale
    // reward identities form a tighter block but have no player record.
    for (offset, name) in [
        (256, "DaikyuPrimeUpperLimb"),
        (384, "VadaryaPrimeReceiver"),
        (512, "AlternoxPrimeReceiver"),
        (640, "AkbroncoPrimeLink"),
    ] {
        bytes[offset..offset + name.len()].copy_from_slice(name.as_bytes());
    }

    for (offset, identity, reward) in [
        (2048, responders[0], "AkbroncoPrimeLink"),
        (3072, responders[1], "FormaBlueprint"),
        (4096, responders[2], "TrumnaPrimeStock"),
        (5120, responders[3], "DaikyuPrimeUpperLimb"),
    ] {
        bytes[offset..offset + identity.len()].copy_from_slice(identity.as_bytes());
        let reward_offset = offset + 123;
        bytes[reward_offset..reward_offset + reward.len()].copy_from_slice(reward.as_bytes());
    }
    let memory = FixtureMemory {
        regions: vec![ReadableRegion::classified(
            0x1900_0000,
            bytes.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        bytes: BTreeMap::from([(0x1900_0000, bytes)]),
        reads: Mutex::new(Vec::new()),
    };

    let resolution = RewardMemoryScanner::new(256, 16 * 1024, Duration::from_secs(1))
        .resolve_player_records(
            &memory,
            &GameProcess::new(9),
            &candidates,
            &responders,
            Some(responders[3]),
            Some("Daikyu Prime Upper Limb"),
        )
        .unwrap();

    assert_eq!(
        resolution,
        RewardResolution::Confirmed {
            choices: vec![
                "Daikyu Prime Upper Limb".into(),
                "Akbronco Prime Link".into(),
                "Forma Blueprint".into(),
                "Trumna Prime Stock".into(),
            ],
            region_start: 0,
        }
    );
}

#[test]
fn archived_player_record_layouts_resolve_as_each_response_arrives() {
    for (identity, name, internal_name, distance) in [
        (
            "de1e7ed00000000000000006",
            "Lavos Prime Chassis Blueprint",
            "LavosPrimeChassisBlueprint",
            123,
        ),
        (
            "de1e7ed00000000000000009",
            "Perigale Prime Stock",
            "PerigalePrimeStock",
            126,
        ),
        (
            "de1e7ed00000000000000006",
            "Afentis Prime Handle",
            "AfentisPrimeHandle",
            127,
        ),
        (
            "de1e7ed00000000000000008",
            "Bronco Prime Barrel",
            "BroncoPrimeBarrel",
            131,
        ),
    ] {
        let candidate = RewardNeedle::from_paths(
            name,
            vec![format!(
                "/Lotus/Types/Recipes/Weapons/WeaponParts/{internal_name}"
            )],
        )
        .unwrap();
        let mut bytes = vec![0_u8; 1024];
        bytes[256..280].copy_from_slice(identity.as_bytes());
        let reward_offset = 256 + distance;
        bytes[reward_offset..reward_offset + internal_name.len()]
            .copy_from_slice(internal_name.as_bytes());
        let memory = FixtureMemory {
            regions: vec![ReadableRegion::classified(
                0x3bb9_0000,
                bytes.len(),
                RegionScanPriority::WritableAnonymous,
            )],
            bytes: BTreeMap::from([(0x3bb9_0000, bytes)]),
            reads: Mutex::new(Vec::new()),
        };

        assert_eq!(
            RewardMemoryScanner::new(128, 4096, Duration::from_secs(1))
                .resolve_player_records(
                    &memory,
                    &GameProcess::new(9),
                    &[candidate],
                    &[identity],
                    None,
                    None,
                )
                .unwrap(),
            RewardResolution::Confirmed {
                choices: vec![name.into()],
                region_start: 0,
            }
        );
    }
}

#[test]
fn retained_remote_record_resolves_a_reward_before_the_player_identity() {
    let identity = "de1e7ed00000000000000002";
    let candidate = RewardNeedle::from_paths(
        "Forma Blueprint",
        vec!["/Lotus/Types/Recipes/Components/FormaBlueprint".into()],
    )
    .unwrap();
    let mut bytes = vec![0_u8; 64 * 1024];
    let identity_offset = 40 * 1024;
    bytes[identity_offset..identity_offset + identity.len()].copy_from_slice(identity.as_bytes());
    let reward_offset = identity_offset - 24_156;
    bytes[reward_offset..reward_offset + "FormaBlueprint".len()].copy_from_slice(b"FormaBlueprint");
    let memory = FixtureMemory {
        regions: vec![ReadableRegion::classified(
            0x2d0e_0000,
            bytes.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        bytes: BTreeMap::from([(0x2d0e_0000, bytes)]),
        reads: Mutex::new(Vec::new()),
    };

    assert_eq!(
        RewardMemoryScanner::new(4096, 128 * 1024, Duration::from_secs(1))
            .resolve_player_records(
                &memory,
                &GameProcess::new(9),
                &[candidate],
                &[identity],
                None,
                None,
            )
            .unwrap(),
        RewardResolution::Confirmed {
            choices: vec!["Forma Blueprint".into()],
            region_start: 0,
        }
    );
}

#[test]
fn retained_remote_record_rejects_ambiguous_nearby_rewards() {
    let identity = "de1e7ed00000000000000002";
    let candidates = [
        ("Forma Blueprint", "FormaBlueprint"),
        ("Fang Prime Blade", "FangPrimeBlade"),
    ]
    .into_iter()
    .map(|(name, internal_name)| {
        RewardNeedle::from_paths(name, vec![internal_name.into()]).unwrap()
    })
    .collect::<Vec<_>>();
    let mut bytes = vec![0_u8; 64 * 1024];
    let identity_offset = 40 * 1024;
    bytes[identity_offset..identity_offset + identity.len()].copy_from_slice(identity.as_bytes());
    for (offset, value) in [
        (identity_offset - 24_156, "FormaBlueprint"),
        (identity_offset - 12_000, "FangPrimeBlade"),
    ] {
        bytes[offset..offset + value.len()].copy_from_slice(value.as_bytes());
    }
    let memory = FixtureMemory {
        regions: vec![ReadableRegion::classified(
            0x2d0e_0000,
            bytes.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        bytes: BTreeMap::from([(0x2d0e_0000, bytes)]),
        reads: Mutex::new(Vec::new()),
    };

    assert_eq!(
        RewardMemoryScanner::new(4096, 128 * 1024, Duration::from_secs(1))
            .resolve_player_records(
                &memory,
                &GameProcess::new(9),
                &candidates,
                &[identity],
                None,
                None,
            )
            .unwrap(),
        RewardResolution::Incomplete
    );
}

#[test]
fn structured_response_record_wins_over_stale_nearby_reward_strings() {
    let identity = "de1e7ed00000000000000006";
    let candidates = [
        (
            "Braton Prime Stock",
            "/Lotus/StoreItems/Types/Recipes/Weapons/WeaponParts/BratonPrimeStock",
        ),
        (
            "Forma Blueprint",
            "/Lotus/StoreItems/Types/Recipes/Components/FormaBlueprint",
        ),
    ]
    .into_iter()
    .map(|(name, path)| RewardNeedle::from_paths(name, vec![path.into()]).unwrap())
    .collect::<Vec<_>>();
    let mut bytes = vec![0_u8; 4096];
    let identity_offset = 1024;
    bytes[identity_offset - 1] = identity.len() as u8;
    bytes[identity_offset..identity_offset + identity.len()].copy_from_slice(identity.as_bytes());
    let player_name = b"player001";
    let player_name_length = identity_offset + identity.len();
    bytes[player_name_length] = player_name.len() as u8;
    bytes[player_name_length + 1..player_name_length + 1 + player_name.len()]
        .copy_from_slice(player_name);
    let session_marker = player_name_length + 1 + player_name.len();
    bytes[session_marker..session_marker + 4].copy_from_slice(&[0xee, 0x80, 0x80, 0x00]);
    let session_length = session_marker + 4;
    bytes[session_length] = 32;
    let session = b"5e551000000000000000000000000002";
    bytes[session_length + 1..session_length + 1 + session.len()].copy_from_slice(session);
    let reward_marker = session_length + 1 + session.len();
    bytes[reward_marker..reward_marker + 4].copy_from_slice(&[0x76, 0x81, 0x44, 0x00]);
    let reward_path = b"/Lotus/StoreItems/Types/Recipes/Weapons/WeaponParts/BratonPrimeStock";
    let reward_offset = reward_marker + 4;
    bytes[reward_offset..reward_offset + reward_path.len()].copy_from_slice(reward_path);

    let stale = b"FormaBlueprint";
    bytes[identity_offset + 220..identity_offset + 220 + stale.len()].copy_from_slice(stale);
    let memory = FixtureMemory {
        regions: vec![
            ReadableRegion::classified(
                0x3eda_0000,
                bytes.len(),
                RegionScanPriority::WritableAnonymous,
            ),
            ReadableRegion::classified(
                0x2d0e_0000,
                16 * 1024,
                RegionScanPriority::WritableAnonymous,
            ),
        ],
        bytes: BTreeMap::from([(0x2d0e_0000, vec![0; 16 * 1024]), (0x3eda_0000, bytes)]),
        reads: Mutex::new(Vec::new()),
    };

    assert_eq!(
        RewardMemoryScanner::new(256, 8192, Duration::from_secs(1))
            .resolve_player_records(
                &memory,
                &GameProcess::new(9),
                &candidates,
                &[identity],
                None,
                None,
            )
            .unwrap(),
        RewardResolution::Confirmed {
            choices: vec!["Braton Prime Stock".into()],
            region_start: 0,
        }
    );
    assert!(
        memory
            .reads
            .lock()
            .unwrap()
            .iter()
            .all(|address| *address >= 0x3eda_0000)
    );
}

#[test]
fn structured_response_matches_the_visible_blueprint_when_catalog_uses_component() {
    let identity = "de1e7ed0000000000000000c";
    let candidate = RewardNeedle::from_paths(
        "Sevagoth Prime Systems Blueprint",
        vec!["/Lotus/Types/Recipes/WarframeRecipes/SevagothPrimeSystemsComponent".into()],
    )
    .unwrap();
    let mut bytes = vec![0_u8; 1024];
    let identity_offset = 128;
    bytes[identity_offset - 1] = identity.len() as u8;
    bytes[identity_offset..identity_offset + identity.len()].copy_from_slice(identity.as_bytes());
    let name_offset = identity_offset + identity.len();
    let player_name = b"MI-NUA-BUA";
    bytes[name_offset] = player_name.len() as u8;
    bytes[name_offset + 1..name_offset + 1 + player_name.len()].copy_from_slice(player_name);
    let session_marker = name_offset + 1 + player_name.len();
    bytes[session_marker..session_marker + 4].copy_from_slice(&[0xee, 0x80, 0x82, 0x00]);
    bytes[session_marker + 4] = 32;
    bytes[session_marker + 5..session_marker + 37]
        .copy_from_slice(b"5e551000000000000000000000000003");
    let path = b"/Lotus/StoreItems/Types/Recipes/WarframeRecipes/SevagothPrimeSystemsBlueprint";
    let reward_marker = session_marker + 37;
    bytes[reward_marker..reward_marker + 4].copy_from_slice(&[0x96, 0x83, path.len() as u8, 0x00]);
    bytes[reward_marker + 4..reward_marker + 4 + path.len()].copy_from_slice(path);
    let memory = FixtureMemory {
        regions: vec![
            ReadableRegion::classified(
                0x3eda_0000,
                bytes.len(),
                RegionScanPriority::WritableAnonymous,
            ),
            ReadableRegion::classified(
                0x2d0e_0000,
                16 * 1024,
                RegionScanPriority::WritableAnonymous,
            ),
        ],
        bytes: BTreeMap::from([(0x2d0e_0000, vec![0; 16 * 1024]), (0x3eda_0000, bytes)]),
        reads: Mutex::new(Vec::new()),
    };

    assert_eq!(
        RewardMemoryScanner::new(256, 4096, Duration::from_secs(1))
            .resolve_player_records(
                &memory,
                &GameProcess::new(9),
                &[candidate],
                &[identity],
                None,
                None,
            )
            .unwrap(),
        RewardResolution::Confirmed {
            choices: vec!["Sevagoth Prime Systems Blueprint".into()],
            region_start: 0,
        }
    );
}

#[test]
fn player_record_scan_prioritizes_low_proton_heaps_over_high_unrelated_mappings() {
    let identity = "de1e7ed0000000000000000c";
    let candidate = RewardNeedle::from_paths(
        "Sevagoth Prime Systems Blueprint",
        vec!["/Lotus/Types/Recipes/WarframeRecipes/SevagothPrimeSystemsBlueprint".into()],
    )
    .unwrap();
    let mut response = vec![0_u8; 2048];
    let identity_offset = 128;
    response[identity_offset - 1] = identity.len() as u8;
    response[identity_offset..identity_offset + identity.len()]
        .copy_from_slice(identity.as_bytes());
    let path = b"/Lotus/StoreItems/Types/Recipes/WarframeRecipes/SevagothPrimeSystemsBlueprint";
    let path_offset = identity_offset + 76;
    response[path_offset - 2] = path.len() as u8;
    response[path_offset..path_offset + path.len()].copy_from_slice(path);
    let low = 0x3eda_0000;
    let high = 0x7fff_0000_0000;
    let memory = FixtureMemory {
        regions: vec![
            ReadableRegion::classified(high, 8192, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(low, response.len(), RegionScanPriority::WritableAnonymous),
        ],
        bytes: BTreeMap::from([(low, response), (high, vec![0; 8192])]),
        reads: Mutex::new(Vec::new()),
    };

    assert!(matches!(
        RewardMemoryScanner::new(256, 4096, Duration::from_secs(1))
            .resolve_player_records(
                &memory,
                &GameProcess::new(9),
                &[candidate],
                &[identity],
                None,
                None,
            )
            .unwrap(),
        RewardResolution::Confirmed { .. }
    ));
    assert_eq!(memory.reads.lock().unwrap().first().copied(), Some(low));
}

#[test]
fn structured_single_response_stops_before_scanning_unrelated_lower_heaps() {
    let identity = "de1e7ed00000000000000006";
    let candidate = RewardNeedle::from_paths(
        "Braton Prime Stock",
        vec!["/Lotus/StoreItems/Types/Recipes/Weapons/WeaponParts/BratonPrimeStock".into()],
    )
    .unwrap();
    let mut response = vec![0_u8; 2048];
    let identity_offset = 128;
    response[identity_offset - 1] = identity.len() as u8;
    response[identity_offset..identity_offset + identity.len()]
        .copy_from_slice(identity.as_bytes());
    let name_offset = identity_offset + identity.len();
    response[name_offset] = 5;
    response[name_offset + 1..name_offset + 6].copy_from_slice(b"Tenno");
    let session_marker = name_offset + 6;
    response[session_marker..session_marker + 4].copy_from_slice(&[0xee, 0x80, 0x80, 0x00]);
    let session_length = session_marker + 4;
    response[session_length] = 32;
    response[session_length + 1..session_length + 33]
        .copy_from_slice(b"5e551000000000000000000000000002");
    let reward_marker = session_length + 33;
    response[reward_marker..reward_marker + 4].copy_from_slice(&[0x76, 0x81, 0x44, 0x00]);
    let path = b"/Lotus/StoreItems/Types/Recipes/Weapons/WeaponParts/BratonPrimeStock";
    response[reward_marker + 4..reward_marker + 4 + path.len()].copy_from_slice(path);

    let high = 0x3eda_0000;
    let low = 0x1900_0000;
    let memory = FixtureMemory {
        regions: vec![
            ReadableRegion::classified(low, 4096, RegionScanPriority::WritableAnonymous),
            ReadableRegion::classified(high, response.len(), RegionScanPriority::WritableAnonymous),
        ],
        bytes: BTreeMap::from([(low, vec![0; 4096]), (high, response)]),
        reads: Mutex::new(Vec::new()),
    };

    assert!(matches!(
        RewardMemoryScanner::new(256, 8192, Duration::from_secs(1))
            .resolve_player_records(
                &memory,
                &GameProcess::new(9),
                &[candidate],
                &[identity],
                None,
                None,
            )
            .unwrap(),
        RewardResolution::Confirmed { .. }
    ));
    assert!(
        memory
            .reads
            .lock()
            .unwrap()
            .iter()
            .all(|address| *address >= high)
    );
}

#[test]
fn structured_squad_records_preserve_the_supplied_screen_order() {
    let responders = [
        "de1e7ed00000000000000006",
        "de1e7ed0000000000000000e",
        "de1e7ed0000000000000000d",
        "de1e7ed0000000000000000b",
    ];
    let rewards = [
        (
            "Nagantaka Prime Stock",
            "/Lotus/StoreItems/Types/Recipes/Weapons/WeaponParts/NagantakaPrimeStock",
            "/Lotus/Types/Recipes/Weapons/WeaponParts/NagantakaPrimeStock",
        ),
        (
            "Forma Blueprint",
            "/Lotus/StoreItems/Types/Recipes/Components/FormaBlueprint",
            "/Lotus/Types/Recipes/Components/FormaBlueprint",
        ),
        (
            "Wukong Prime Neuroptics Blueprint",
            "/Lotus/StoreItems/Types/Recipes/WarframeRecipes/WukongPrimeHelmetComponent",
            "/Lotus/Types/Recipes/WarframeRecipes/WukongPrimeHelmetComponent",
        ),
        (
            "Kompressa Prime Blueprint",
            "/Lotus/StoreItems/Types/Recipes/Weapons/KompressaPrimeBlueprint",
            "/Lotus/Types/Recipes/Weapons/KompressaPrimeBlueprint",
        ),
    ];
    let candidates = rewards
        .iter()
        .map(|(name, _, catalog_path)| {
            RewardNeedle::from_paths(*name, vec![(*catalog_path).into()]).unwrap()
        })
        .collect::<Vec<_>>();
    let mut bytes = vec![0_u8; 16 * 1024];

    // Allocation order is deliberately unrelated to the screen order.
    for (offset, responder_index) in [(1024, 2), (4096, 0), (7168, 3), (10_240, 1)] {
        let identity = responders[responder_index];
        let path = rewards[responder_index].1.as_bytes();
        bytes[offset - 1] = identity.len() as u8;
        bytes[offset..offset + identity.len()].copy_from_slice(identity.as_bytes());
        let name_offset = offset + identity.len();
        bytes[name_offset] = 5;
        bytes[name_offset + 1..name_offset + 6].copy_from_slice(b"Tenno");
        let session_marker = name_offset + 6;
        bytes[session_marker..session_marker + 4].copy_from_slice(&[0xee, 0x80, 0x80, 0x00]);
        bytes[session_marker + 4] = 32;
        bytes[session_marker + 5..session_marker + 37]
            .copy_from_slice(b"5e551000000000000000000000000002");
        let reward_marker = session_marker + 37;
        bytes[reward_marker..reward_marker + 4].copy_from_slice(&[
            0x76,
            0x81,
            path.len() as u8,
            0x00,
        ]);
        bytes[reward_marker + 4..reward_marker + 4 + path.len()].copy_from_slice(path);
    }

    let memory = FixtureMemory {
        regions: vec![
            ReadableRegion::classified(
                0x3eda_0000,
                bytes.len(),
                RegionScanPriority::WritableAnonymous,
            ),
            ReadableRegion::classified(
                0x2d0e_0000,
                16 * 1024,
                RegionScanPriority::WritableAnonymous,
            ),
        ],
        bytes: BTreeMap::from([(0x2d0e_0000, vec![0; 16 * 1024]), (0x3eda_0000, bytes)]),
        reads: Mutex::new(Vec::new()),
    };

    assert_eq!(
        RewardMemoryScanner::new(256, 32 * 1024, Duration::from_secs(1))
            .resolve_player_records(
                &memory,
                &GameProcess::new(9),
                &candidates,
                &responders,
                Some(responders[0]),
                Some(rewards[0].0),
            )
            .unwrap(),
        RewardResolution::Confirmed {
            choices: rewards.iter().map(|(name, _, _)| (*name).into()).collect(),
            region_start: 0,
        }
    );
    assert!(
        memory
            .reads
            .lock()
            .unwrap()
            .iter()
            .all(|address| *address >= 0x3eda_0000)
    );
}

#[test]
fn captured_caliban_athodai_vadarya_sevagoth_screen_replays_in_ltr_order() {
    let responders = [
        "de1e7ed00000000000000006",
        "de1e7ed00000000000000007",
        "de1e7ed00000000000000001",
        "de1e7ed0000000000000000c",
    ];
    let remote_rewards = [
        (
            responders[1],
            "Athodai Prime Blueprint",
            "/Lotus/StoreItems/Types/Recipes/Weapons/AthodaiPrimeBlueprint",
            "/Lotus/Types/Recipes/Weapons/AthodaiPrimeBlueprint",
        ),
        (
            responders[2],
            "Vadarya Prime Receiver",
            "/Lotus/StoreItems/Types/Recipes/Weapons/WeaponParts/PrimeLightningGunReceiver",
            "/Lotus/Types/Recipes/Weapons/WeaponParts/PrimeLightningGunReceiver",
        ),
        (
            responders[3],
            "Sevagoth Prime Systems Blueprint",
            "/Lotus/StoreItems/Types/Recipes/WarframeRecipes/SevagothPrimeSystemsBlueprint",
            "/Lotus/Types/Recipes/WarframeRecipes/SevagothPrimeSystemsComponent",
        ),
    ];
    let mut candidates = remote_rewards
        .iter()
        .map(|(_, name, _, catalog_path)| {
            RewardNeedle::from_paths(*name, vec![(*catalog_path).into()]).unwrap()
        })
        .collect::<Vec<_>>();
    candidates.push(
        RewardNeedle::from_paths(
            "Caliban Prime Chassis Blueprint",
            vec!["/Lotus/Types/Recipes/WarframeRecipes/CalibanPrimeChassisComponent".into()],
        )
        .unwrap(),
    );
    let mut bytes = vec![0_u8; 8192];
    for ((identity, _, response_path, _), offset) in remote_rewards.iter().zip([512, 3072, 5632]) {
        let identity = identity.as_bytes();
        let response_path = response_path.as_bytes();
        bytes[offset - 1] = identity.len() as u8;
        bytes[offset..offset + identity.len()].copy_from_slice(identity);
        let path_offset = offset + 76;
        bytes[path_offset - 2] = response_path.len() as u8;
        bytes[path_offset..path_offset + response_path.len()].copy_from_slice(response_path);
    }
    let memory = FixtureMemory {
        regions: vec![ReadableRegion::classified(
            0x3eda_0000,
            bytes.len(),
            RegionScanPriority::WritableAnonymous,
        )],
        bytes: BTreeMap::from([(0x3eda_0000, bytes)]),
        reads: Mutex::new(Vec::new()),
    };

    assert_eq!(
        RewardMemoryScanner::new(256, 32 * 1024, Duration::from_secs(1))
            .resolve_player_records(
                &memory,
                &GameProcess::new(9),
                &candidates,
                &responders,
                Some(responders[0]),
                Some("Caliban Prime Chassis Blueprint"),
            )
            .unwrap(),
        RewardResolution::Confirmed {
            choices: vec![
                "Caliban Prime Chassis Blueprint".into(),
                "Athodai Prime Blueprint".into(),
                "Vadarya Prime Receiver".into(),
                "Sevagoth Prime Systems Blueprint".into(),
            ],
            region_start: 0,
        }
    );
}
