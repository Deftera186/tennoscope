use std::{collections::BTreeMap, sync::Mutex, time::Duration};

use warframe_acquisition::{
    AcquisitionError, GameProcess, MemoryReader, ReadableRegion, RegionScanPriority,
    RewardMemoryScanner, RewardNeedle, RewardRepresentation, RewardResolution,
    resolve_reward_choices,
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

fn online_candidates() -> Vec<RewardNeedle> {
    ["Perigale", "Burston", "Trumna", "Forma"]
        .into_iter()
        .map(|name| RewardNeedle::new(name, [name]).unwrap())
        .collect()
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
