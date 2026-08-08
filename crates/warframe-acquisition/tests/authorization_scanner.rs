use std::sync::Mutex;

use warframe_acquisition::{
    AcquisitionError, AuthorizationScanner, GameProcess, MemoryReader, ReadableRegion,
    RegionScanPriority,
};

const URL_FIXTURE: &[u8] = include_bytes!("fixtures/authorization-url-encoded.bin");
const LOGIN_FIXTURE: &[u8] = include_bytes!("fixtures/authorization-login-response.bin");

struct ByteMemory {
    base: u64,
    bytes: Vec<u8>,
    max_read: usize,
}

impl ByteMemory {
    fn new(bytes: impl Into<Vec<u8>>, max_read: usize) -> Self {
        Self {
            base: 0x1000,
            bytes: bytes.into(),
            max_read,
        }
    }
}

impl MemoryReader for ByteMemory {
    fn readable_regions(
        &self,
        _process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        Ok(vec![ReadableRegion::new(self.base, self.bytes.len())])
    }

    fn read_at(
        &self,
        _process: &GameProcess,
        address: u64,
        buffer: &mut [u8],
    ) -> Result<usize, AcquisitionError> {
        let offset = usize::try_from(address - self.base).unwrap();
        let available = self.bytes.len().saturating_sub(offset);
        let len = available.min(buffer.len()).min(self.max_read);
        buffer[..len].copy_from_slice(&self.bytes[offset..offset + len]);
        Ok(len)
    }
}

fn scan(
    bytes: impl Into<Vec<u8>>,
    chunk_size: usize,
    max_read: usize,
) -> Result<String, AcquisitionError> {
    let memory = ByteMemory::new(bytes, max_read);
    let process = GameProcess::new(7);
    let authorization = AuthorizationScanner::new(chunk_size).scan(&memory, &process)?;
    Ok(format!("{authorization:?}"))
}

#[test]
fn finds_url_authorization_split_across_bounded_partial_reads() {
    let rendered = scan([URL_FIXTURE, URL_FIXTURE, URL_FIXTURE].concat(), 31, 7).unwrap();

    assert_eq!(rendered.matches("[REDACTED]").count(), 2);
    assert!(!rendered.contains("00112233445566778899aabb"));
    assert!(!rendered.contains("123456789012345678"));
}

#[test]
fn finds_login_response_authorization_split_across_chunks() {
    let rendered = scan(LOGIN_FIXTURE, 29, 11).unwrap();

    assert_eq!(rendered.matches("[REDACTED]").count(), 2);
    assert!(!rendered.contains("aabbccddeeff001122334455"));
    assert!(!rendered.contains("987654321012345678"));
}

#[test]
fn prefers_complete_url_candidate_over_lower_ranked_login_candidate() {
    let mut bytes = LOGIN_FIXTURE.to_vec();
    bytes.extend_from_slice(&[URL_FIXTURE, URL_FIXTURE, URL_FIXTURE].concat());

    assert!(scan(bytes, 41, 13).is_ok());
}

#[test]
fn deduplicates_repeated_identical_candidates() {
    let mut bytes = URL_FIXTURE.to_vec();
    bytes.extend_from_slice(URL_FIXTURE);
    bytes.extend_from_slice(URL_FIXTURE);

    assert!(scan(bytes, 37, 17).is_ok());
}

#[test]
fn rejects_distinct_candidates_at_the_same_rank_as_ambiguous() {
    let mut bytes = URL_FIXTURE.to_vec();
    bytes.extend_from_slice(
        b"?accountId=ffeeddccbbaa998877665544&nonce=222222222222222222&ct=synthetic",
    );

    assert_eq!(
        scan(bytes, 43, 19).unwrap_err(),
        AcquisitionError::AuthorizationAmbiguous
    );
}

#[test]
fn rejects_equal_high_confidence_url_candidates_as_ambiguous() {
    let first = [URL_FIXTURE, URL_FIXTURE, URL_FIXTURE].concat();
    let second = [
        b"?accountId=ffeeddccbbaa998877665544&nonce=222222222222222222&ct=synthetic".as_slice(),
        b"?accountId=ffeeddccbbaa998877665544&nonce=222222222222222222&ct=synthetic".as_slice(),
        b"?accountId=ffeeddccbbaa998877665544&nonce=222222222222222222&ct=synthetic".as_slice(),
    ]
    .concat();
    let mut bytes = first;
    bytes.extend_from_slice(&second);

    assert_eq!(
        scan(bytes, 43, 19).unwrap_err(),
        AcquisitionError::AuthorizationAmbiguous
    );
}

#[test]
fn rejects_malformed_or_incomplete_candidates() {
    for malformed in [
        b"?accountId=00112233445566778899aab&nonce=123456".as_slice(),
        b"?accountId=00112233445566778899aabb&nonce=1234&ct=synthetic".as_slice(),
        b"?accountId=00112233445566778899aabb&nonce=12x456".as_slice(),
        b"{\"id\":\"00112233445566778899aabb\",\"Nonce\":\"123456\"}".as_slice(),
        b"{\"id\":\"00112233445566778899aabz\",\"Nonce\":123456}".as_slice(),
    ] {
        assert_eq!(
            scan(malformed, 17, 5).unwrap_err(),
            AcquisitionError::AuthorizationNotFound
        );
    }
}

#[test]
fn rejects_url_nonce_truncated_at_the_region_boundary() {
    let truncated = b"?accountId=00112233445566778899aabb&nonce=123456789012345678";

    assert_eq!(
        scan(truncated, 23, 7).unwrap_err(),
        AcquisitionError::AuthorizationNotFound
    );
}

#[test]
fn rejects_login_nonce_truncated_at_the_region_boundary() {
    let truncated = b"{\"id\":\"00112233445566778899aabb\",\"Nonce\":123456789012345678";

    assert_eq!(
        scan(truncated, 23, 7).unwrap_err(),
        AcquisitionError::AuthorizationNotFound
    );
}

#[test]
fn caps_an_oversized_public_chunk_configuration() {
    let rendered = scan(
        [URL_FIXTURE, URL_FIXTURE, URL_FIXTURE].concat(),
        usize::MAX,
        usize::MAX,
    )
    .unwrap();

    assert_eq!(rendered.matches("[REDACTED]").count(), 2);
}

#[test]
fn marker_heavy_memory_is_rejected_without_changing_the_result() {
    let mut bytes = Vec::new();
    for index in 0..10_000_u64 {
        bytes.extend_from_slice(
            format!(
                "?accountId={index:024x}&nonce={:018}&ct=synthetic;",
                index + 100_000
            )
            .as_bytes(),
        );
    }

    assert_eq!(
        scan(bytes, 4096, 1024).unwrap_err(),
        AcquisitionError::AuthorizationAmbiguous
    );
}

#[test]
fn does_not_join_candidates_across_disjoint_regions() {
    struct SplitRegions;

    impl MemoryReader for SplitRegions {
        fn readable_regions(
            &self,
            _process: &GameProcess,
        ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
            Ok(vec![
                ReadableRegion::new(0x1000, 35),
                ReadableRegion::new(0x2000, 25),
            ])
        }

        fn read_at(
            &self,
            _process: &GameProcess,
            address: u64,
            buffer: &mut [u8],
        ) -> Result<usize, AcquisitionError> {
            let source = if address < 0x2000 {
                b"?accountId=00112233445566778899aabb".as_slice()
            } else {
                b"&nonce=123456789012345678".as_slice()
            };
            let base = if address < 0x2000 { 0x1000 } else { 0x2000 };
            let offset = usize::try_from(address - base).unwrap();
            let len = source.len().saturating_sub(offset).min(buffer.len());
            buffer[..len].copy_from_slice(&source[offset..offset + len]);
            Ok(len)
        }
    }

    let result = AuthorizationScanner::new(16).scan(&SplitRegions, &GameProcess::new(7));
    assert_eq!(result.unwrap_err(), AcquisitionError::AuthorizationNotFound);
}

struct OrderedMemory {
    regions: Vec<(ReadableRegion, Vec<u8>)>,
    reads: Mutex<Vec<u64>>,
}

impl MemoryReader for OrderedMemory {
    fn readable_regions(
        &self,
        _process: &GameProcess,
    ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
        Ok(self.regions.iter().map(|(region, _)| *region).collect())
    }

    fn read_at(
        &self,
        _process: &GameProcess,
        address: u64,
        buffer: &mut [u8],
    ) -> Result<usize, AcquisitionError> {
        self.reads.lock().unwrap().push(address);
        let (region, bytes) = self
            .regions
            .iter()
            .find(|(region, _)| {
                address >= region.start()
                    && address < region.start() + u64::try_from(region.len()).unwrap()
            })
            .unwrap();
        let offset = usize::try_from(address - region.start()).unwrap();
        buffer.fill(0);
        if offset < bytes.len() {
            let len = (bytes.len() - offset).min(buffer.len());
            buffer[..len].copy_from_slice(&bytes[offset..offset + len]);
        }
        Ok(buffer.len())
    }
}

#[test]
fn scans_writable_anonymous_regions_before_file_backed_regions() {
    let preferred = [URL_FIXTURE, URL_FIXTURE, URL_FIXTURE].concat();
    let memory = OrderedMemory {
        regions: vec![
            (
                ReadableRegion::classified(
                    0x1000,
                    URL_FIXTURE.len(),
                    RegionScanPriority::FileBacked,
                ),
                URL_FIXTURE.to_vec(),
            ),
            (
                ReadableRegion::classified(
                    0x2000,
                    preferred.len(),
                    RegionScanPriority::WritableAnonymous,
                ),
                preferred,
            ),
        ],
        reads: Mutex::new(Vec::new()),
    };

    AuthorizationScanner::new(4096)
        .scan(&memory, &GameProcess::new(7))
        .unwrap();

    assert_eq!(memory.reads.into_inner().unwrap(), vec![0x2000, 0x1000]);
}

#[test]
fn three_current_copies_outvote_one_later_stale_conflict() {
    let confident = [URL_FIXTURE, URL_FIXTURE, URL_FIXTURE].concat();
    let stale = b"?accountId=ffeeddccbbaa998877665544&nonce=222222222222222222&ct=synthetic";
    let memory = OrderedMemory {
        regions: vec![
            (
                ReadableRegion::classified(
                    0x3000,
                    confident.len(),
                    RegionScanPriority::WritableAnonymous,
                ),
                confident,
            ),
            (
                ReadableRegion::classified(
                    0x4000,
                    stale.len(),
                    RegionScanPriority::WritableAnonymous,
                ),
                stale.to_vec(),
            ),
        ],
        reads: Mutex::new(Vec::new()),
    };

    AuthorizationScanner::new(4096)
        .scan(&memory, &GameProcess::new(7))
        .unwrap();

    assert_eq!(memory.reads.into_inner().unwrap(), vec![0x3000, 0x4000]);
}

#[test]
fn three_current_copies_outvote_one_earlier_stale_conflict() {
    let confident = [URL_FIXTURE, URL_FIXTURE, URL_FIXTURE].concat();
    let stale = b"?accountId=ffeeddccbbaa998877665544&nonce=222222222222222222&ct=synthetic";
    let memory = OrderedMemory {
        regions: vec![
            (
                ReadableRegion::classified(
                    0x3000,
                    stale.len(),
                    RegionScanPriority::WritableAnonymous,
                ),
                stale.to_vec(),
            ),
            (
                ReadableRegion::classified(
                    0x4000,
                    confident.len(),
                    RegionScanPriority::WritableAnonymous,
                ),
                confident,
            ),
        ],
        reads: Mutex::new(Vec::new()),
    };

    AuthorizationScanner::new(4096)
        .scan(&memory, &GameProcess::new(7))
        .unwrap();

    assert_eq!(memory.reads.into_inner().unwrap(), vec![0x3000, 0x4000]);
}

#[test]
fn fallback_finds_a_candidate_only_in_a_file_backed_region() {
    let confident = [URL_FIXTURE, URL_FIXTURE, URL_FIXTURE].concat();
    let memory = OrderedMemory {
        regions: vec![(
            ReadableRegion::classified(0x5000, confident.len(), RegionScanPriority::FileBacked),
            confident,
        )],
        reads: Mutex::new(Vec::new()),
    };

    AuthorizationScanner::new(4096)
        .scan(&memory, &GameProcess::new(7))
        .unwrap();

    assert_eq!(memory.reads.into_inner().unwrap(), vec![0x5000]);
}

/// A credential that only the tail of a large process holds must still be found.
///
/// The budget is a fixed number of bytes over an address space far larger than it, and where the
/// account/nonce pair happens to sit varies per launch. A Steam Deck report saw a read succeed and
/// then fail with `AuthorizationNotFound` on the retry, same session -- the sampler simply missed.
/// Giving up after one pass turned "we did not look there" into "your game has no credential".
#[test]
fn a_credential_the_first_pass_misses_is_still_found() {
    let confident = [URL_FIXTURE, URL_FIXTURE, URL_FIXTURE].concat();
    let region_len = 512 * 1024 * 1024;
    let candidate_offset = 200 * 1024 * 1024;

    struct SparseMemory {
        len: usize,
        candidate_offset: usize,
        candidate: Vec<u8>,
        reads: Mutex<usize>,
    }

    impl MemoryReader for SparseMemory {
        fn readable_regions(
            &self,
            _process: &GameProcess,
        ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
            Ok(vec![ReadableRegion::classified(
                0x10_0000,
                self.len,
                RegionScanPriority::WritableAnonymous,
            )])
        }

        fn read_at(
            &self,
            _process: &GameProcess,
            address: u64,
            buffer: &mut [u8],
        ) -> Result<usize, AcquisitionError> {
            *self.reads.lock().unwrap() += 1;
            let read_start = usize::try_from(address - 0x10_0000).unwrap();
            let read_end = read_start + buffer.len();
            let candidate_end = self.candidate_offset + self.candidate.len();
            buffer.fill(0);
            let overlap_start = read_start.max(self.candidate_offset);
            let overlap_end = read_end.min(candidate_end);
            if overlap_start < overlap_end {
                let source = overlap_start - self.candidate_offset;
                let destination = overlap_start - read_start;
                let len = overlap_end - overlap_start;
                buffer[destination..destination + len]
                    .copy_from_slice(&self.candidate[source..source + len]);
            }
            Ok(buffer.len())
        }
    }

    let memory = SparseMemory {
        len: region_len,
        candidate_offset,
        candidate: confident,
        reads: Mutex::new(0),
    };

    let rendered = AuthorizationScanner::new(1024 * 1024)
        .scan(&memory, &GameProcess::new(7))
        .map(|authorization| format!("{authorization:?}"))
        .expect("a credential inside a readable region must be found");

    assert_eq!(rendered.matches("[REDACTED]").count(), 2);
}

#[test]
fn fallback_samples_a_giant_anonymous_region() {
    let confident = [URL_FIXTURE, URL_FIXTURE, URL_FIXTURE].concat();
    let memory = OrderedMemory {
        regions: vec![(
            ReadableRegion::classified(
                0x6000,
                129 * 1024 * 1024,
                RegionScanPriority::WritableAnonymous,
            ),
            confident,
        )],
        reads: Mutex::new(Vec::new()),
    };

    AuthorizationScanner::new(4096)
        .scan(&memory, &GameProcess::new(7))
        .unwrap();

    assert!(memory.reads.into_inner().unwrap().contains(&0x6000));
}
