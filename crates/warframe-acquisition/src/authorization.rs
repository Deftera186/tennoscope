use std::{cmp::Reverse, collections::VecDeque};

use memchr::memmem;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    AcquisitionError, GameProcess, InventoryAuthorization, MemoryReader, RegionScanPriority,
};

const LOGIN_SEARCH_DISTANCE: usize = 2048;
const CANDIDATE_OVERLAP: usize = LOGIN_SEARCH_DISTANCE + 128;
const MAX_CHUNK_SIZE: usize = 1024 * 1024;
const PREFERRED_SCAN_BYTES: usize = 152 * 1024 * 1024;
const FALLBACK_SCAN_BYTES: usize = 64 * 1024 * 1024;
const MAX_PREFERRED_REGION_BYTES: usize = 128 * 1024 * 1024;
const FALLBACK_SAMPLE_BYTES: usize = 4 * 1024 * 1024;
const CONFIDENT_URL_COPIES: usize = 3;
const MAX_DISTINCT_CANDIDATES_PER_RANK: usize = 8;
const MAX_LOCATIONS_PER_CANDIDATE: usize = 16;
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
        self.scan_with_policy(memory, process, ScanPolicy::default())
    }

    fn scan_with_policy(
        &self,
        memory: &dyn MemoryReader,
        process: &GameProcess,
        policy: ScanPolicy,
    ) -> Result<InventoryAuthorization, AcquisitionError> {
        let mut regions = memory.readable_regions(process)?;
        regions.sort_by_key(|region| (Reverse(region.scan_priority()), region.start()));
        let region_count = regions.len();
        let mut candidates = CandidateAccumulator::default();
        let mut read_buffer = Zeroizing::new(vec![0_u8; self.chunk_size]);
        let mut fallback = Vec::with_capacity(regions.len());
        let mut preferred_remaining = policy.preferred_bytes;

        for region in regions {
            let preferred = region.scan_priority() == RegionScanPriority::WritableAnonymous
                && region.len() <= policy.max_preferred_region_bytes;
            if !preferred || preferred_remaining == 0 {
                fallback.push(ScanRange::whole(region));
                continue;
            }
            let scan_len = region.len().min(preferred_remaining);
            scan_range(
                memory,
                process,
                ScanRange::new(region, 0, scan_len),
                &mut read_buffer,
                &mut candidates,
                self.chunk_size,
            )?;
            preferred_remaining -= scan_len;
            if scan_len < region.len() {
                fallback.push(ScanRange::new(region, scan_len, region.len() - scan_len));
            }
        }

        fallback.sort_by_key(|range| {
            (
                Reverse(range.region.scan_priority()),
                range.region.start(),
                range.offset,
            )
        });
        let mut fallback = fallback
            .into_iter()
            .map(FallbackCursor::new)
            .collect::<VecDeque<_>>();
        let mut sampled = self.drain_fallback(
            memory,
            process,
            &mut fallback,
            &mut read_buffer,
            &mut candidates,
            policy,
            policy.fallback_bytes,
        )?;

        // A budget is a guess about where the credential sits, and on a small-memory machine the
        // guess misses: the same session read fine once and then reported "not found" on the retry
        // because the sampler simply never looked at the page holding it. Nothing found at all is
        // not evidence of absence, so spend the rest of the address space rather than report a
        // conclusion we did not earn. A found-but-ambiguous result is a real answer and stops here.
        let exhaustive = candidates.is_empty();
        if exhaustive {
            sampled += self.drain_fallback(
                memory,
                process,
                &mut fallback,
                &mut read_buffer,
                &mut candidates,
                policy,
                usize::MAX,
            )?;
        }

        trace_scan(&format!(
            "[authorization] regions={region_count} sampled_bytes={sampled} exhaustive={exhaustive} url={} login={}",
            candidates.url.candidates.len(),
            candidates.login.candidates.len(),
        ));
        select_candidate(candidates)
    }

    /// Sample fallback ranges round-robin until `budget` bytes are read or every cursor is spent.
    ///
    /// Returns the bytes actually read, which is less than the budget once the cursors run dry.
    #[allow(clippy::too_many_arguments)]
    fn drain_fallback(
        &self,
        memory: &dyn MemoryReader,
        process: &GameProcess,
        fallback: &mut VecDeque<FallbackCursor>,
        read_buffer: &mut Zeroizing<Vec<u8>>,
        candidates: &mut CandidateAccumulator,
        policy: ScanPolicy,
        budget: usize,
    ) -> Result<usize, AcquisitionError> {
        let mut remaining = budget;
        while remaining > 0 {
            let Some(mut cursor) = fallback.pop_front() else {
                break;
            };
            let Some(sample_offset) = cursor.next_sample_offset(policy.fallback_sample_bytes)
            else {
                continue;
            };
            let sample_len = policy
                .fallback_sample_bytes
                .min(cursor.range.len - sample_offset)
                .min(remaining);
            scan_range(
                memory,
                process,
                ScanRange::new(
                    cursor.range.region,
                    cursor.range.offset + sample_offset,
                    sample_len,
                ),
                read_buffer,
                candidates,
                self.chunk_size,
            )?;
            remaining -= sample_len;
            fallback.push_back(cursor);
        }
        Ok(budget - remaining)
    }
}

#[derive(Clone, Copy)]
struct ScanPolicy {
    preferred_bytes: usize,
    fallback_bytes: usize,
    max_preferred_region_bytes: usize,
    fallback_sample_bytes: usize,
}

impl Default for ScanPolicy {
    fn default() -> Self {
        Self {
            preferred_bytes: PREFERRED_SCAN_BYTES,
            fallback_bytes: FALLBACK_SCAN_BYTES,
            max_preferred_region_bytes: MAX_PREFERRED_REGION_BYTES,
            fallback_sample_bytes: FALLBACK_SAMPLE_BYTES,
        }
    }
}

#[derive(Clone, Copy)]
struct ScanRange {
    region: crate::ReadableRegion,
    offset: usize,
    len: usize,
}

impl ScanRange {
    fn new(region: crate::ReadableRegion, offset: usize, len: usize) -> Self {
        Self {
            region,
            offset,
            len,
        }
    }

    fn whole(region: crate::ReadableRegion) -> Self {
        Self::new(region, 0, region.len())
    }
}

struct FallbackCursor {
    range: ScanRange,
    round: usize,
    visited_offsets: Vec<usize>,
}

impl FallbackCursor {
    fn new(range: ScanRange) -> Self {
        Self {
            range,
            round: 0,
            visited_offsets: Vec::new(),
        }
    }

    fn next_sample_offset(&mut self, slice_bytes: usize) -> Option<usize> {
        if self.range.len == 0 || slice_bytes == 0 {
            return None;
        }
        let max_offset = self
            .range
            .len
            .saturating_sub(slice_bytes.min(self.range.len));
        let sequential_rounds = self.range.len.div_ceil(slice_bytes);
        while self.round < sequential_rounds.saturating_add(3) {
            let round = self.round;
            self.round += 1;
            let offset = match round {
                0 => 0,
                1 => max_offset / 2,
                2 => max_offset,
                _ => (round - 2).saturating_mul(slice_bytes).min(max_offset),
            };
            if !self.visited_offsets.contains(&offset) {
                self.visited_offsets.push(offset);
                return Some(offset);
            }
        }
        None
    }
}

fn scan_range(
    memory: &dyn MemoryReader,
    process: &GameProcess,
    range: ScanRange,
    read_buffer: &mut Zeroizing<Vec<u8>>,
    candidates: &mut CandidateAccumulator,
    chunk_size: usize,
) -> Result<(), AcquisitionError> {
    let mut consumed = 0_usize;
    let mut overlap = Zeroizing::new(Vec::with_capacity(CANDIDATE_OVERLAP));
    while consumed < range.len {
        let requested = chunk_size.min(range.len - consumed);
        let absolute_offset = range
            .offset
            .checked_add(consumed)
            .ok_or(AcquisitionError::MemoryReadFailed { pid: process.pid() })?;
        let address = range
            .region
            .start()
            .checked_add(absolute_offset as u64)
            .ok_or(AcquisitionError::MemoryReadFailed { pid: process.pid() })?;
        let read = memory.read_at(process, address, &mut read_buffer[..requested])?;
        if read == 0 {
            break;
        }

        let mut window = Zeroizing::new(Vec::with_capacity(overlap.len() + read));
        window.extend_from_slice(&overlap);
        append_read_and_wipe(&mut window, read_buffer, read);
        let window_start = address
            .checked_sub(overlap.len() as u64)
            .ok_or(AcquisitionError::MemoryReadFailed { pid: process.pid() })?;
        collect_candidates_at(&window, window_start, candidates);

        let keep = CANDIDATE_OVERLAP.min(window.len());
        wipe_bytes(&mut overlap);
        overlap.clear();
        overlap.extend_from_slice(&window[window.len() - keep..]);
        consumed += read;
    }
    Ok(())
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
    login: CandidateSet,
    url: CandidateSet,
}

#[derive(Default)]
struct CandidateSet {
    candidates: Vec<CountedCandidate>,
    overflowed: bool,
}

struct CountedCandidate {
    authorization: InventoryAuthorization,
    locations: Vec<u64>,
}

impl CandidateSet {
    /// Drop every candidate but the newest one per account.
    ///
    /// A mistyped password, a re-login, or a session the game refreshed leaves each nonce it ever
    /// held resident in the process until the game itself restarts, and the scan finds all of
    /// them -- which read as "multiple authorizations" and stayed unrecoverable for the rest of
    /// the play session. The nonce is a monotonically increasing counter for a given account, so
    /// the largest one is the live session; the smaller ones are dead credentials that would be
    /// rejected by the endpoint anyway. Ambiguity across *different* accounts is left alone: that
    /// one has no right answer and should still refuse.
    fn collapse_stale_nonces(&mut self) {
        let mut newest: Vec<usize> = Vec::new();
        for index in 0..self.candidates.len() {
            match newest.iter_mut().find(|kept| {
                self.candidates[**kept].authorization.account_id()
                    == self.candidates[index].authorization.account_id()
            }) {
                Some(kept) => {
                    if nonce_value(&self.candidates[index]) > nonce_value(&self.candidates[*kept]) {
                        *kept = index;
                    }
                }
                None => newest.push(index),
            }
        }
        if newest.len() == self.candidates.len() {
            return;
        }
        newest.sort_unstable();
        let mut index = 0;
        self.candidates.retain(|_| {
            let keep = newest.binary_search(&index).is_ok();
            index += 1;
            keep
        });
        // The dropped copies were part of what made the survivor look unconvincing. Overflow only
        // ever recorded "there were more distinct pairs than we keep", so once a single credential
        // for a single account is left it no longer stands in the way of trusting it -- but with
        // two accounts still in hand it is exactly the signal that should keep refusing.
        if self.candidates.len() == 1 {
            self.overflowed = false;
        }
    }
}

fn nonce_value(candidate: &CountedCandidate) -> u64 {
    candidate
        .authorization
        .nonce()
        .parse::<u64>()
        .unwrap_or_default()
}

impl CandidateAccumulator {
    fn record(&mut self, candidate: Candidate) {
        let set = match candidate.rank {
            CandidateRank::LoginResponse => &mut self.login,
            CandidateRank::UrlEncoded => &mut self.url,
        };
        if let Some(existing) = set.candidates.iter_mut().find(|existing| {
            existing.authorization.account_id() == candidate.authorization.account_id()
                && existing.authorization.nonce() == candidate.authorization.nonce()
        }) {
            if existing.locations.len() < MAX_LOCATIONS_PER_CANDIDATE
                && !existing.locations.contains(&candidate.location)
            {
                existing.locations.push(candidate.location);
            }
        } else if set.candidates.len() < MAX_DISTINCT_CANDIDATES_PER_RANK {
            set.candidates.push(CountedCandidate {
                authorization: candidate.authorization,
                locations: vec![candidate.location],
            });
        } else {
            set.overflowed = true;
        }
    }

    fn take_confident_url(&mut self) -> Option<InventoryAuthorization> {
        if self.url.overflowed {
            return None;
        }
        let highest = self
            .url
            .candidates
            .iter()
            .map(|candidate| candidate.locations.len())
            .max()?;
        if highest < CONFIDENT_URL_COPIES
            || self
                .url
                .candidates
                .iter()
                .filter(|candidate| candidate.locations.len() == highest)
                .count()
                != 1
        {
            return None;
        }
        let index = self
            .url
            .candidates
            .iter()
            .position(|candidate| candidate.locations.len() == highest)
            .expect("unique maximum exists");
        Some(self.url.candidates.swap_remove(index).authorization)
    }

    fn is_empty(&self) -> bool {
        self.login.candidates.is_empty()
            && self.url.candidates.is_empty()
            && !self.login.overflowed
            && !self.url.overflowed
    }

    #[cfg(test)]
    fn retained_candidate_count(&self) -> usize {
        self.login.candidates.len() + self.url.candidates.len()
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
    mut candidates: CandidateAccumulator,
) -> Result<InventoryAuthorization, AcquisitionError> {
    candidates.url.collapse_stale_nonces();
    candidates.login.collapse_stale_nonces();
    if let Some(authorization) = candidates.take_confident_url() {
        return Ok(authorization);
    }
    if candidates.url.overflowed || !candidates.url.candidates.is_empty() {
        return Err(AcquisitionError::AuthorizationAmbiguous);
    }
    if candidates.login.overflowed || candidates.login.candidates.len() > 1 {
        return Err(AcquisitionError::AuthorizationAmbiguous);
    }
    if let Some(candidate) = candidates.login.candidates.pop() {
        return Ok(candidate.authorization);
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

/// Counts only: how much was looked at and how many distinct pairs were seen, never their bytes.
fn trace_scan(line: &str) {
    #[cfg(debug_assertions)]
    crate::append_debug_line(line);
    #[cfg(not(debug_assertions))]
    let _ = line;
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
        Candidate, CandidateAccumulator, CandidateRank, ScanPolicy, append_read_and_wipe,
        collect_candidates, collect_candidates_at, select_candidate, wipe_bytes,
    };
    use crate::{
        AcquisitionError, GameProcess, InventoryAuthorization, MemoryReader, ReadableRegion,
        RegionScanPriority,
    };

    const URL: &[u8] = include_bytes!("../tests/fixtures/authorization-url-encoded.bin");
    const LOGIN: &[u8] = include_bytes!("../tests/fixtures/authorization-login-response.bin");
    const CURRENT_ACCOUNT: &str = "00112233445566778899aabb";
    const STALE_URL: &[u8] =
        b"?accountId=ffeeddccbbaa998877665544&nonce=222222222222222222&ct=synthetic";

    #[test]
    fn extracts_the_exact_synthetic_fixture_values() {
        let mut url_candidates = CandidateAccumulator::default();
        for offset in 0..3_u64 {
            collect_candidates_at(URL, offset, &mut url_candidates);
        }
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
        for offset in 0..3_u64 {
            collect_candidates_at(URL, offset, &mut candidates);
        }

        let selected = select_candidate(candidates).unwrap();
        assert_eq!(selected.account_id(), "00112233445566778899aabb");
    }

    #[test]
    fn a_re_login_keeps_the_newest_nonce_rather_than_refusing() {
        let mut candidates = CandidateAccumulator::default();
        for (nonce, location) in [
            // The dead session left behind by the mistyped password, still resident in the game's
            // memory, and copied more times than the live one purely by chance.
            ("123456789012345670", 0),
            ("123456789012345670", 1),
            ("123456789012345670", 2),
            ("123456789012345670", 3),
            ("123456789012345678", 4),
            ("123456789012345678", 5),
            ("123456789012345678", 6),
        ] {
            candidates.record(Candidate {
                rank: CandidateRank::UrlEncoded,
                authorization: InventoryAuthorization::from_zeroizing(
                    Zeroizing::new(CURRENT_ACCOUNT.to_owned()),
                    Zeroizing::new(nonce.to_owned()),
                ),
                location,
            });
        }

        let selected = select_candidate(candidates).unwrap();

        assert_eq!(selected.account_id(), CURRENT_ACCOUNT);
        assert_eq!(selected.nonce(), "123456789012345678");
    }

    #[test]
    fn two_different_accounts_still_refuse() {
        let mut candidates = CandidateAccumulator::default();
        for (account, location) in [(CURRENT_ACCOUNT, 0), ("ffeeddccbbaa998877665544", 1)] {
            for copy in 0..4 {
                candidates.record(Candidate {
                    rank: CandidateRank::UrlEncoded,
                    authorization: InventoryAuthorization::from_zeroizing(
                        Zeroizing::new(account.to_owned()),
                        Zeroizing::new("123456789012345678".to_owned()),
                    ),
                    location: location * 10 + copy,
                });
            }
        }

        assert!(matches!(
            select_candidate(candidates),
            Err(AcquisitionError::AuthorizationAmbiguous)
        ));
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

        assert_eq!(candidates.retained_candidate_count(), 8);
        assert!(candidates.url.overflowed);
    }

    struct BudgetMemory {
        confident: Vec<u8>,
    }

    impl MemoryReader for BudgetMemory {
        fn readable_regions(
            &self,
            _process: &GameProcess,
        ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
            Ok(vec![
                ReadableRegion::classified(0x1000, 64, RegionScanPriority::WritableAnonymous),
                ReadableRegion::classified(
                    0x2000,
                    self.confident.len(),
                    RegionScanPriority::WritableAnonymous,
                ),
            ])
        }

        fn read_at(
            &self,
            _process: &GameProcess,
            address: u64,
            buffer: &mut [u8],
        ) -> Result<usize, AcquisitionError> {
            buffer.fill(0);
            if address >= 0x2000 {
                let offset = usize::try_from(address - 0x2000).unwrap();
                let len = self
                    .confident
                    .len()
                    .saturating_sub(offset)
                    .min(buffer.len());
                buffer[..len].copy_from_slice(&self.confident[offset..offset + len]);
            }
            Ok(buffer.len())
        }
    }

    #[test]
    fn fallback_reaches_regions_beyond_the_preferred_budget() {
        let memory = BudgetMemory {
            confident: [URL, URL, URL].concat(),
        };
        let scanner = super::AuthorizationScanner::new(64);

        let result = scanner.scan_with_policy(
            &memory,
            &GameProcess::new(7),
            ScanPolicy {
                preferred_bytes: 64,
                fallback_bytes: 512,
                max_preferred_region_bytes: 128,
                fallback_sample_bytes: 512,
            },
        );

        assert!(result.is_ok());
    }

    struct TierMemory {
        regions: Vec<(ReadableRegion, Vec<u8>)>,
    }

    impl MemoryReader for TierMemory {
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
            let len = bytes.len().saturating_sub(offset).min(buffer.len());
            buffer[..len].copy_from_slice(&bytes[offset..offset + len]);
            Ok(buffer.len())
        }
    }

    fn tier_policy() -> ScanPolicy {
        ScanPolicy {
            preferred_bytes: 4096,
            fallback_bytes: 4096,
            max_preferred_region_bytes: 4096,
            fallback_sample_bytes: 4096,
        }
    }

    #[test]
    fn four_fallback_copies_outvote_three_preferred_stale_copies() {
        let memory = TierMemory {
            regions: vec![
                (
                    ReadableRegion::classified(
                        0x1000,
                        STALE_URL.len() * 3,
                        RegionScanPriority::WritableAnonymous,
                    ),
                    [STALE_URL, STALE_URL, STALE_URL].concat(),
                ),
                (
                    ReadableRegion::classified(
                        0x3000,
                        URL.len() * 4,
                        RegionScanPriority::FileBacked,
                    ),
                    [URL, URL, URL, URL].concat(),
                ),
            ],
        };

        let authorization = super::AuthorizationScanner::new(4096)
            .scan_with_policy(&memory, &GameProcess::new(7), tier_policy())
            .unwrap();

        assert_eq!(authorization.account_id(), CURRENT_ACCOUNT);
    }

    #[test]
    fn four_fallback_stale_copies_outvote_three_preferred_current_copies() {
        let memory = TierMemory {
            regions: vec![
                (
                    ReadableRegion::classified(
                        0x1000,
                        URL.len() * 3,
                        RegionScanPriority::WritableAnonymous,
                    ),
                    [URL, URL, URL].concat(),
                ),
                (
                    ReadableRegion::classified(
                        0x3000,
                        STALE_URL.len() * 4,
                        RegionScanPriority::FileBacked,
                    ),
                    [STALE_URL, STALE_URL, STALE_URL, STALE_URL].concat(),
                ),
            ],
        };

        let authorization = super::AuthorizationScanner::new(4096)
            .scan_with_policy(&memory, &GameProcess::new(7), tier_policy())
            .unwrap();

        assert_eq!(authorization.account_id(), "ffeeddccbbaa998877665544");
    }

    struct SparseRangeMemory {
        len: usize,
        candidate_offset: usize,
        candidate: Vec<u8>,
    }

    impl MemoryReader for SparseRangeMemory {
        fn readable_regions(
            &self,
            _process: &GameProcess,
        ) -> Result<Vec<ReadableRegion>, AcquisitionError> {
            Ok(vec![ReadableRegion::classified(
                0x10_0000,
                self.len,
                RegionScanPriority::FileBacked,
            )])
        }

        fn read_at(
            &self,
            _process: &GameProcess,
            address: u64,
            buffer: &mut [u8],
        ) -> Result<usize, AcquisitionError> {
            let read_start = usize::try_from(address - 0x10_0000).unwrap();
            let read_end = read_start + buffer.len();
            let candidate_end = self.candidate_offset + self.candidate.len();
            buffer.fill(0);
            let overlap_start = read_start.max(self.candidate_offset);
            let overlap_end = read_end.min(candidate_end);
            if overlap_start < overlap_end {
                let source_start = overlap_start - self.candidate_offset;
                let destination_start = overlap_start - read_start;
                let len = overlap_end - overlap_start;
                buffer[destination_start..destination_start + len]
                    .copy_from_slice(&self.candidate[source_start..source_start + len]);
            }
            Ok(buffer.len())
        }
    }

    fn sparse_policy(fallback_bytes: usize) -> ScanPolicy {
        ScanPolicy {
            preferred_bytes: 0,
            fallback_bytes,
            max_preferred_region_bytes: 0,
            fallback_sample_bytes: 4 * 1024 * 1024,
        }
    }

    #[test]
    fn fallback_samples_beyond_the_first_four_mebibytes() {
        let candidate = [URL, URL, URL].concat();
        let memory = SparseRangeMemory {
            len: 20 * 1024 * 1024,
            candidate_offset: 5 * 1024 * 1024,
            candidate,
        };

        assert!(
            super::AuthorizationScanner::new(1024 * 1024)
                .scan_with_policy(
                    &memory,
                    &GameProcess::new(7),
                    sparse_policy(16 * 1024 * 1024),
                )
                .is_ok()
        );
    }

    #[test]
    fn fallback_samples_near_the_tail_of_a_large_region() {
        let candidate = [URL, URL, URL].concat();
        let len = 20 * 1024 * 1024;
        let memory = SparseRangeMemory {
            len,
            candidate_offset: len - candidate.len() - 128,
            candidate,
        };

        assert!(
            super::AuthorizationScanner::new(1024 * 1024)
                .scan_with_policy(
                    &memory,
                    &GameProcess::new(7),
                    sparse_policy(12 * 1024 * 1024),
                )
                .is_ok()
        );
    }
}
