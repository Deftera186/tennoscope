use zeroize::Zeroizing;

use crate::{AcquisitionError, GameProcess, InventoryAuthorization, MemoryReader};

const LOGIN_SEARCH_DISTANCE: usize = 2048;
const CANDIDATE_OVERLAP: usize = LOGIN_SEARCH_DISTANCE + 128;
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
            chunk_size: chunk_size.max(1),
        }
    }

    pub fn scan(
        &self,
        memory: &dyn MemoryReader,
        process: &GameProcess,
    ) -> Result<InventoryAuthorization, AcquisitionError> {
        let regions = memory.readable_regions(process)?;
        let mut candidates = Vec::new();
        let mut read_buffer = Zeroizing::new(vec![0_u8; self.chunk_size]);

        for region in regions {
            let mut offset = 0_usize;
            let mut overlap = Zeroizing::new(Vec::with_capacity(CANDIDATE_OVERLAP));

            while offset < region.len() {
                let requested = self.chunk_size.min(region.len() - offset);
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
                window.extend_from_slice(&read_buffer[..read]);
                collect_candidates(&window, false, &mut candidates);

                let keep = CANDIDATE_OVERLAP.min(window.len());
                overlap.clear();
                overlap.extend_from_slice(&window[window.len() - keep..]);
                offset += read;
            }

            collect_candidates(&overlap, true, &mut candidates);
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
}

fn collect_candidates(bytes: &[u8], final_window: bool, output: &mut Vec<Candidate>) {
    collect_url_candidates(bytes, final_window, output);
    collect_login_candidates(bytes, final_window, output);
}

fn collect_url_candidates(bytes: &[u8], final_window: bool, output: &mut Vec<Candidate>) {
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

        let Some(nonce_end) = numeric_value_end(bytes, nonce_marker_end, final_window) else {
            continue;
        };
        push_candidate(
            output,
            CandidateRank::UrlEncoded,
            &bytes[account_start..account_end],
            &bytes[nonce_marker_end..nonce_end],
        );
    }
}

fn collect_login_candidates(bytes: &[u8], final_window: bool, output: &mut Vec<Candidate>) {
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
        let Some(nonce_end) = numeric_value_end(bytes, nonce_start, final_window) else {
            continue;
        };
        push_candidate(
            output,
            CandidateRank::LoginResponse,
            account,
            &bytes[nonce_start..nonce_end],
        );
    }
}

fn push_candidate(output: &mut Vec<Candidate>, rank: CandidateRank, account: &[u8], nonce: &[u8]) {
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
    let duplicate = output.iter().any(|candidate| {
        candidate.rank == rank
            && candidate.authorization.account_id() == authorization.account_id()
            && candidate.authorization.nonce() == authorization.nonce()
    });
    if !duplicate {
        output.push(Candidate {
            rank,
            authorization,
        });
    }
}

fn select_candidate(
    mut candidates: Vec<Candidate>,
) -> Result<InventoryAuthorization, AcquisitionError> {
    let Some(best_rank) = candidates.iter().map(|candidate| candidate.rank).max() else {
        return Err(AcquisitionError::AuthorizationNotFound);
    };
    candidates.retain(|candidate| candidate.rank == best_rank);
    if candidates.len() != 1 {
        return Err(AcquisitionError::AuthorizationAmbiguous);
    }
    Ok(candidates.pop().expect("one candidate").authorization)
}

fn find_all<'a>(bytes: &'a [u8], needle: &'a [u8]) -> impl Iterator<Item = usize> + 'a {
    bytes
        .windows(needle.len())
        .enumerate()
        .filter_map(move |(index, window)| (window == needle).then_some(index))
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

fn numeric_value_end(bytes: &[u8], start: usize, final_window: bool) -> Option<usize> {
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
        None if final_window => Some(end),
        _ => None,
    }
}

fn is_value_terminator(byte: u8) -> bool {
    matches!(byte, b'&' | b'}' | b',' | b'"' | 0) || byte.is_ascii_whitespace()
}

#[cfg(test)]
mod tests {
    use super::{collect_candidates, select_candidate};

    const URL: &[u8] = include_bytes!("../tests/fixtures/authorization-url-encoded.bin");
    const LOGIN: &[u8] = include_bytes!("../tests/fixtures/authorization-login-response.bin");

    #[test]
    fn extracts_the_exact_synthetic_fixture_values() {
        let mut url_candidates = Vec::new();
        collect_candidates(URL, true, &mut url_candidates);
        let url = select_candidate(url_candidates).unwrap();
        assert_eq!(url.account_id(), "00112233445566778899aabb");
        assert_eq!(url.nonce(), "123456789012345678");

        let mut login_candidates = Vec::new();
        collect_candidates(LOGIN, true, &mut login_candidates);
        let login = select_candidate(login_candidates).unwrap();
        assert_eq!(login.account_id(), "aabbccddeeff001122334455");
        assert_eq!(login.nonce(), "987654321012345678");
    }

    #[test]
    fn selection_returns_the_highest_ranked_complete_candidate() {
        let mut candidates = Vec::new();
        collect_candidates(LOGIN, true, &mut candidates);
        collect_candidates(URL, true, &mut candidates);

        let selected = select_candidate(candidates).unwrap();
        assert_eq!(selected.account_id(), "00112233445566778899aabb");
    }
}
