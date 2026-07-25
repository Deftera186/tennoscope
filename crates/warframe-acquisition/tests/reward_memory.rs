use std::{collections::BTreeMap, sync::Mutex, time::Duration};

use warframe_acquisition::{
    AcquisitionError, GameProcess, MemoryReader, ReadableRegion, RegionScanPriority,
    RewardMemoryScanner, RewardNeedle, RewardRepresentation,
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
