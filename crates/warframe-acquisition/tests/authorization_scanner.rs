use warframe_acquisition::{
    AcquisitionError, AuthorizationScanner, GameProcess, MemoryReader, ReadableRegion,
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
    let rendered = scan(URL_FIXTURE, 31, 7).unwrap();

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
    bytes.extend_from_slice(URL_FIXTURE);

    assert!(scan(bytes, 41, 13).is_ok());
}

#[test]
fn deduplicates_repeated_identical_candidates() {
    let mut bytes = URL_FIXTURE.to_vec();
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
    let rendered = scan(URL_FIXTURE, usize::MAX, usize::MAX).unwrap();

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
