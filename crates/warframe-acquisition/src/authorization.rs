use std::cmp::Reverse;

use memchr::memmem;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    AcquisitionError, GameProcess, InventoryAuthorization, MemoryReader, RegionScanPriority,
};

const LOGIN_SEARCH_DISTANCE: usize = 2048;
const CANDIDATE_OVERLAP: usize = LOGIN_SEARCH_DISTANCE + 128;
const MAX_CHUNK_SIZE: usize = 1024 * 1024;
// Avoid pathological Wine reservations and cap the maximum successful reads in
// one startup scan. File-backed code/data mappings are not credential sources.
const MAX_REGION_BYTES: usize = 128 * 1024 * 1024;
const MAX_SCAN_BYTES: usize = 512 * 1024 * 1024;
// Existing Linux scanners use repeated identical URL tuples as a confidence
// signal. Distinct physical locations are required so chunk overlap cannot
// manufacture confidence. A conflict observed before this threshold remains
// an explicit ambiguity error.
const CONFIDENT_URL_COPIES: usize = 3;
const URL_ACCOUNT_MARKER: &[u8] = b"accountId=";
const URL_NONCE_MARKER: &[u8] = b"&nonce=";
const LOGIN_ID_MARKER: &[u8] = b"\"id\"";
const LOGIN_NONCE_MARKER: &[u8] = b"\"Nonce\"";

pub struct AuthorizationScanner {
    chunk_size: usize,
}

impl AuthorizationScanner {
    pub fn new(chunk_size: usize) -> Self {
        Self {
            chunk_size: chunk_size.clamp(1, MAX_CHUNK_SIZE),
        }
    }

    pub fn scan(
        &self,
        memory: &dyn MemoryReader,
        process: &GameProcess,
    ) -> Result<InventoryAuthorization, AcquisitionError> {
        let mut regions = memory.readable_regions(process)?;
        regions.sort_by_key(|region| (Reverse(region.scan_priority()), region.start()));
        let mut candidates = CandidateAccumulator::default();
        let mut read_buffer = Zeroizing::new(vec![0_u8; self.chunk_size]);
        let mut scanned_bytes = 0_usize;

        for region in regions {
            if region.scan_priority() == RegionScanPriority::FileBacked
                || region.len() > MAX_REGION_BYTES
                || scanned_bytes >= MAX_SCAN_BYTES
            {
                continue;
            }
            let mut offset = 0_usize;
            let mut overlap = Zeroizing::new(Vec::with_capacity(CANDIDATE_OVERLAP));

            while offset < region.len() && scanned_bytes < MAX_SCAN_BYTES {
                let requested = self
                    .chunk_size
                    .min(region.len() - offset)
                    .min(MAX_SCAN_BYTES - scanned_bytes);
                let address = region
                    .start()
                    .checked_add(offset as u64)
                    .ok_or(AcquisitionError::MemoryReadFailed { pid: process.pid() })?;
                let read = memory.read_at(process, address, &mut read_buffer[..requested])?;
                if read == 0 {
                    break;
                }

                let mut window = Zeroizing::new(Vec::with_capacity(overlap.len() + read));
                window.extend_from_slice(&overlap);
                append_read_and_wipe(&mut window, &mut read_buffer, read);
                let window_start = address
                    .checked_sub(overlap.len() as u64)
                    .ok_or(AcquisitionError::MemoryReadFailed { pid: process.pid() })?;
                collect_candidates_at(&window, window_start, &mut candidates);
                if let Some(authorization) = candidates.take_confident_url() {
                    return Ok(authorization);
                }

                let keep = CANDIDATE_OVERLAP.min(window.len());
                wipe_bytes(&mut overlap);
                overlap.clear();
                overlap.extend_from_slice(&window[window.len() - keep..]);
                offset += read;
                scanned_bytes += read;
            }
        }

        select_candidate(candidates)
    }
}

#[derive(Clone, Copy, Eq, Ord, PartialEq, PartialOrd)]
enum CandidateRank {
    LoginResponse,
    UrlEncoded,
}

struct Candidate {
    rank: CandidateRank,
    authorization: InventoryAuthorization,
    location: u64,
}

#[derive(Default)]
struct CandidateAccumulator {
    login: RankState,
    url: RankState,
}

#[derive(Default)]
enum RankState {
    #[default]
    None,
    One {
        authorization: InventoryAuthorization,
        locations: Vec<u64>,
    },
    Ambiguous,
}

impl CandidateAccumulator {
    fn record(&mut self, candidate: Candidate) {
        let state = match candidate.rank {
            CandidateRank::LoginResponse => &mut self.login,
            CandidateRank::UrlEncoded => &mut self.url,
        };
        match state {
            RankState::None => {
                *state = RankState::One {
                    authorization: candidate.authorization,
                    locations: vec![candidate.location],
                };
            }
            RankState::One {
                authorization,
                locations,
            } if authorization.account_id() == candidate.authorization.account_id()
                && authorization.nonce() == candidate.authorization.nonce() =>
            {
                if locations.len() < CONFIDENT_URL_COPIES
                    && !locations.contains(&candidate.location)
                {
                    locations.push(candidate.location);
                }
            }
            RankState::One { .. } => *state = RankState::Ambiguous,
            RankState::Ambiguous => {}
        }
    }

    fn take_confident_url(&mut self) -> Option<InventoryAuthorization> {
        if !matches!(
            self.url,
            RankState::One {
                ref locations,
                ..
            } if locations.len() >= CONFIDENT_URL_COPIES
        ) {
            return None;
        }
        match std::mem::take(&mut self.url) {
            RankState::One { authorization, .. } => Some(authorization),
            _ => unreachable!("confidence was checked before taking the candidate"),
        }
    }

    #[cfg(test)]
    fn retained_candidate_count(&self) -> usize {
        [&self.login, &self.url]
            .into_iter()
            .filter(|state| matches!(state, RankState::One { .. }))
            .count()
    }
}

#[cfg(test)]
fn collect_candidates(bytes: &[u8], output: &mut CandidateAccumulator) {
    collect_candidates_at(bytes, 0, output);
}

fn collect_candidates_at(bytes: &[u8], base: u64, output: &mut CandidateAccumulator) {
    collect_url_candidates(bytes, base, output);
    collect_login_candidates(bytes, base, output);
}

fn collect_url_candidates(bytes: &[u8], base: u64, output: &mut CandidateAccumulator) {
    for marker_start in find_all(bytes, URL_ACCOUNT_MARKER) {
        let account_start = marker_start + URL_ACCOUNT_MARKER.len();
        let account_end = account_start + 24;
        let nonce_marker_end = account_end + URL_NONCE_MARKER.len();
        if nonce_marker_end > bytes.len()
            || !is_account_id(&bytes[account_start..account_end])
            || &bytes[account_end..nonce_marker_end] != URL_NONCE_MARKER
        {
            continue;
        }

        let Some(nonce_end) = numeric_value_end(bytes, nonce_marker_end) else {
            continue;
        };
        push_candidate(
            output,
            CandidateRank::UrlEncoded,
            &bytes[account_start..account_end],
            &bytes[nonce_marker_end..nonce_end],
            base.saturating_add(marker_start as u64),
        );
    }
}

fn collect_login_candidates(bytes: &[u8], base: u64, output: &mut CandidateAccumulator) {
    for marker_start in find_all(bytes, LOGIN_ID_MARKER) {
        let Some((account, after_account)) =
            quoted_json_value(bytes, marker_start + LOGIN_ID_MARKER.len())
        else {
            continue;
        };
        if !is_account_id(account) {
            continue;
        }

        let search_end = after_account
            .saturating_add(LOGIN_SEARCH_DISTANCE)
            .min(bytes.len());
        let nonce_area = &bytes[after_account..search_end];
        let Some(relative_nonce_marker) = find_all(nonce_area, LOGIN_NONCE_MARKER).next() else {
            continue;
        };
        let nonce_marker_end = after_account + relative_nonce_marker + LOGIN_NONCE_MARKER.len();
        let Some(nonce_start) = json_number_start(bytes, nonce_marker_end) else {
            continue;
        };
        let Some(nonce_end) = numeric_value_end(bytes, nonce_start) else {
            continue;
        };
        push_candidate(
            output,
            CandidateRank::LoginResponse,
            account,
            &bytes[nonce_start..nonce_end],
            base.saturating_add(marker_start as u64),
        );
    }
}

fn push_candidate(
    output: &mut CandidateAccumulator,
    rank: CandidateRank,
    account: &[u8],
    nonce: &[u8],
    location: u64,
) {
    if !(5..=20).contains(&nonce.len()) {
        return;
    }
    let Ok(nonce_text) = std::str::from_utf8(nonce) else {
        return;
    };
    if nonce_text.parse::<u64>().is_err() {
        return;
    }

    let account_id = Zeroizing::new(String::from_utf8(account.to_vec()).expect("validated ASCII"));
    let nonce = Zeroizing::new(String::from_utf8(nonce.to_vec()).expect("validated ASCII"));
    let authorization = InventoryAuthorization::from_zeroizing(account_id, nonce);
    output.record(Candidate {
        rank,
        authorization,
        location,
    });
}

fn select_candidate(
    candidates: CandidateAccumulator,
) -> Result<InventoryAuthorization, AcquisitionError> {
    for state in [candidates.url, candidates.login] {
        match state {
            RankState::None => {}
            RankState::One { authorization, .. } => return Ok(authorization),
            RankState::Ambiguous => return Err(AcquisitionError::AuthorizationAmbiguous),
        }
    }
    Err(AcquisitionError::AuthorizationNotFound)
}

fn find_all<'a>(bytes: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    memmem::find_iter(bytes, needle)
}

fn is_account_id(value: &[u8]) -> bool {
    value.len() == 24 && value.iter().all(u8::is_ascii_hexdigit)
}

fn quoted_json_value(bytes: &[u8], mut cursor: usize) -> Option<(&[u8], usize)> {
    cursor = skip_ascii_whitespace(bytes, cursor);
    if bytes.get(cursor) != Some(&b':') {
        return None;
    }
    cursor = skip_ascii_whitespace(bytes, cursor + 1);
    if bytes.get(cursor) != Some(&b'"') {
        return None;
    }
    let value_start = cursor + 1;
    let quote = bytes[value_start..].iter().position(|byte| *byte == b'"')?;
    let value_end = value_start + quote;
    Some((&bytes[value_start..value_end], value_end + 1))
}

fn json_number_start(bytes: &[u8], mut cursor: usize) -> Option<usize> {
    cursor = skip_ascii_whitespace(bytes, cursor);
    if bytes.get(cursor) != Some(&b':') {
        return None;
    }
    Some(skip_ascii_whitespace(bytes, cursor + 1))
}

fn skip_ascii_whitespace(bytes: &[u8], mut cursor: usize) -> usize {
    while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
        cursor += 1;
    }
    cursor
}

fn numeric_value_end(bytes: &[u8], start: usize) -> Option<usize> {
    let digits = bytes[start..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    if digits == 0 {
        return None;
    }
    let end = start + digits;
    match bytes.get(end) {
        Some(byte) if is_value_terminator(*byte) => Some(end),
        _ => None,
    }
}

fn is_value_terminator(byte: u8) -> bool {
    matches!(byte, b'&' | b'}' | b',' | b'"' | 0) || byte.is_ascii_whitespace()
}

fn wipe_bytes(bytes: &mut [u8]) {
    bytes.zeroize();
}

fn append_read_and_wipe(destination: &mut Vec<u8>, source: &mut [u8], read: usize) {
    debug_assert!(read <= source.len());
    destination.extend_from_slice(&source[..read]);
    wipe_bytes(source);
}

#[cfg(test)]
mod tests {
    use zeroize::Zeroizing;

    use super::{
        Candidate, CandidateAccumulator, CandidateRank, append_read_and_wipe, collect_candidates,
        select_candidate, wipe_bytes,
    };
    use crate::InventoryAuthorization;

    const URL: &[u8] = include_bytes!("../tests/fixtures/authorization-url-encoded.bin");
    const LOGIN: &[u8] = include_bytes!("../tests/fixtures/authorization-login-response.bin");

    #[test]
    fn extracts_the_exact_synthetic_fixture_values() {
        let mut url_candidates = CandidateAccumulator::default();
        collect_candidates(URL, &mut url_candidates);
        let url = select_candidate(url_candidates).unwrap();
        assert_eq!(url.account_id(), "00112233445566778899aabb");
        assert_eq!(url.nonce(), "123456789012345678");

        let mut login_candidates = CandidateAccumulator::default();
        collect_candidates(LOGIN, &mut login_candidates);
        let login = select_candidate(login_candidates).unwrap();
        assert_eq!(login.account_id(), "aabbccddeeff001122334455");
        assert_eq!(login.nonce(), "987654321012345678");
    }

    #[test]
    fn selection_returns_the_highest_ranked_complete_candidate() {
        let mut candidates = CandidateAccumulator::default();
        collect_candidates(LOGIN, &mut candidates);
        collect_candidates(URL, &mut candidates);

        let selected = select_candidate(candidates).unwrap();
        assert_eq!(selected.account_id(), "00112233445566778899aabb");
    }

    #[test]
    fn overlap_wipe_overwrites_contents_before_the_buffer_is_reused() {
        let mut overlap = b"synthetic-sensitive-overlap".to_vec();

        wipe_bytes(&mut overlap);

        assert!(overlap.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn read_buffer_copy_wipes_the_copied_prefix_and_stale_suffix() {
        let mut read_buffer = b"new-secretstale-secret".to_vec();
        let mut window = Vec::new();

        append_read_and_wipe(&mut window, &mut read_buffer, 10);

        assert_eq!(window, b"new-secret");
        assert!(read_buffer.iter().all(|byte| *byte == 0));
    }

    #[test]
    fn adversarial_candidates_leave_bounded_per_rank_state() {
        let mut candidates = CandidateAccumulator::default();
        for index in 0..10_000_u64 {
            candidates.record(Candidate {
                rank: CandidateRank::UrlEncoded,
                authorization: InventoryAuthorization::from_zeroizing(
                    Zeroizing::new(format!("{index:024x}")),
                    Zeroizing::new(format!("{}", index + 100_000)),
                ),
                location: index,
            });
        }

        assert_eq!(candidates.retained_candidate_count(), 0);
    }
}
