use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

use aho_corasick::AhoCorasick;
use zeroize::Zeroize;

use crate::{AcquisitionError, GameProcess, MemoryReader, RegionScanPriority};

const LIVE_UI_ADDRESS_MIN: u64 = 0x1300_0000;
const LIVE_UI_ADDRESS_MAX: u64 = 0x2800_0000;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewardNeedle {
    choice_name: String,
    display_name: Vec<u8>,
    internal_paths: Vec<Vec<u8>>,
}

impl RewardNeedle {
    pub fn new<const N: usize>(
        choice_name: impl Into<String>,
        paths: [&str; N],
    ) -> Result<Self, AcquisitionError> {
        Self::from_paths(choice_name, paths.into_iter().map(str::to_owned).collect())
    }

    pub fn from_paths(
        choice_name: impl Into<String>,
        paths: Vec<String>,
    ) -> Result<Self, AcquisitionError> {
        let choice_name = choice_name.into();
        if choice_name.trim().is_empty() || paths.iter().any(|path| path.trim().is_empty()) {
            return Err(AcquisitionError::SnapshotInvalid);
        }
        Ok(Self {
            display_name: choice_name.as_bytes().to_vec(),
            choice_name,
            internal_paths: paths.into_iter().map(String::into_bytes).collect(),
        })
    }

    pub fn choice_name(&self) -> &str {
        &self.choice_name
    }

    pub fn internal_paths(&self) -> &[Vec<u8>] {
        &self.internal_paths
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RewardRepresentation {
    DisplayName,
    InternalPath,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewardHit {
    choice_name: String,
    address: u64,
    region_start: u64,
    priority: RegionScanPriority,
    representation: RewardRepresentation,
}

impl RewardHit {
    pub fn choice_name(&self) -> &str {
        &self.choice_name
    }
    pub const fn address(&self) -> u64 {
        self.address
    }
    pub const fn region_start(&self) -> u64 {
        self.region_start
    }
    pub const fn priority(&self) -> RegionScanPriority {
        self.priority
    }
    pub const fn representation(&self) -> RewardRepresentation {
        self.representation
    }
}

#[derive(Clone, Debug)]
pub struct RewardFingerprint {
    hits: Vec<RewardHit>,
    bytes_read: u64,
    elapsed: Duration,
}

impl RewardFingerprint {
    pub fn hits(&self) -> &[RewardHit] {
        &self.hits
    }
    pub const fn bytes_read(&self) -> u64 {
        self.bytes_read
    }
    pub const fn elapsed(&self) -> Duration {
        self.elapsed
    }
}

pub struct RewardMemoryScanner {
    chunk_size: usize,
    byte_budget: u64,
    timeout: Duration,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RewardResolution {
    Confirmed {
        choices: Vec<String>,
        region_start: u64,
    },
    Incomplete,
    Ambiguous,
    TimedOut,
}

pub fn resolve_reward_choices(
    baseline: &RewardFingerprint,
    current: &RewardFingerprint,
    expected_choices: usize,
    maximum_span: u64,
) -> RewardResolution {
    if expected_choices == 0 {
        return RewardResolution::Incomplete;
    }
    let old = baseline
        .hits
        .iter()
        .map(hit_identity)
        .collect::<BTreeSet<_>>();
    let mut regions = BTreeMap::<u64, Vec<&RewardHit>>::new();
    for hit in &current.hits {
        if !old.contains(&hit_identity(hit)) {
            regions.entry(hit.region_start).or_default().push(hit);
        }
    }
    let mut complete = Vec::new();
    for (region_start, hits) in regions {
        let mut ordered_hits = hits;
        ordered_hits.sort_by_key(|hit| hit.address);
        let mut region_clusters = BTreeMap::<Vec<String>, u64>::new();
        for left in 0..ordered_hits.len() {
            let mut earliest = BTreeMap::<&str, &RewardHit>::new();
            for hit in &ordered_hits[left..] {
                if hit.address - ordered_hits[left].address > maximum_span {
                    break;
                }
                earliest.entry(hit.choice_name()).or_insert(hit);
                if earliest.len() > expected_choices {
                    break;
                }
                if earliest.len() == expected_choices {
                    let mut choices = earliest.values().copied().collect::<Vec<_>>();
                    choices.sort_by_key(|choice| choice.address);
                    let span = choices.last().expect("non-empty choice cluster").address
                        - choices.first().expect("non-empty choice cluster").address;
                    let names = choices
                        .into_iter()
                        .map(|choice| choice.choice_name.clone())
                        .collect::<Vec<_>>();
                    region_clusters
                        .entry(names)
                        .and_modify(|existing| *existing = (*existing).min(span))
                        .or_insert(span);
                }
            }
        }
        for (choices, span) in region_clusters {
            complete.push((span, region_start, choices));
        }
    }
    complete.sort_by_key(|(span, _, _)| *span);
    let Some((best_span, _, _)) = complete.first() else {
        return RewardResolution::Incomplete;
    };
    if complete
        .get(1)
        .is_some_and(|(span, _, _)| *span <= best_span.saturating_mul(2))
    {
        return RewardResolution::Ambiguous;
    }
    match complete.first() {
        Some((_, region_start, choices)) => RewardResolution::Confirmed {
            choices: choices.clone(),
            region_start: *region_start,
        },
        None => RewardResolution::Incomplete,
    }
}

pub fn resolve_current_reward_choices(
    current: &RewardFingerprint,
    expected_choices: usize,
    maximum_span: u64,
) -> RewardResolution {
    resolve_reward_choices(
        &RewardFingerprint {
            hits: Vec::new(),
            bytes_read: 0,
            elapsed: Duration::ZERO,
        },
        current,
        expected_choices,
        maximum_span,
    )
}

fn hit_identity(hit: &RewardHit) -> (&str, RewardRepresentation, RegionScanPriority, u64) {
    (
        hit.choice_name(),
        hit.representation,
        hit.priority,
        hit.address - hit.region_start,
    )
}

impl RewardMemoryScanner {
    pub fn new(chunk_size: usize, byte_budget: u64, timeout: Duration) -> Self {
        Self {
            chunk_size: chunk_size.max(1),
            byte_budget,
            timeout,
        }
    }

    pub fn fingerprint(
        &self,
        memory: &dyn MemoryReader,
        process: &GameProcess,
        candidates: &[RewardNeedle],
    ) -> Result<RewardFingerprint, AcquisitionError> {
        self.fingerprint_regions(memory, process, candidates, None)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn resolve_player_records(
        &self,
        memory: &dyn MemoryReader,
        process: &GameProcess,
        candidates: &[RewardNeedle],
        responders: &[&str],
        local_identity: Option<&str>,
        local_choice: Option<&str>,
    ) -> Result<RewardResolution, AcquisitionError> {
        if responders.is_empty() || responders.iter().any(|identity| identity.len() != 24) {
            return Ok(RewardResolution::Incomplete);
        }
        let started = Instant::now();
        let mut regions = memory.readable_regions(process)?;
        regions.retain(|region| region.scan_priority() == RegionScanPriority::WritableAnonymous);
        regions.sort_by_key(|region| {
            (
                region.start() >= 0x8000_0000,
                std::cmp::Reverse(region.start()),
            )
        });
        let player_byte_budget = self.byte_budget.saturating_mul(3);

        let mut reward_patterns = BTreeSet::<(&str, Vec<u8>)>::new();
        for candidate in candidates {
            reward_patterns.insert((
                candidate.choice_name(),
                candidate
                    .choice_name()
                    .chars()
                    .filter(|character| character.is_ascii_alphanumeric())
                    .collect::<String>()
                    .into_bytes(),
            ));
            for path in candidate.internal_paths() {
                if let Some(basename) = path.rsplit(|byte| *byte == b'/').next()
                    && !basename.is_empty()
                {
                    reward_patterns.insert((candidate.choice_name(), basename.to_vec()));
                }
            }
        }
        let reward_patterns = reward_patterns.into_iter().collect::<Vec<_>>();
        let player_matcher = AhoCorasick::new(
            responders
                .iter()
                .map(|identity| identity.as_bytes())
                .collect::<Vec<_>>(),
        )
        .map_err(|_| AcquisitionError::SnapshotInvalid)?;
        let player_overlap = responders
            .iter()
            .map(|identity| identity.len())
            .max()
            .unwrap_or(1)
            .saturating_sub(1);
        let mut buffer = vec![0_u8; self.chunk_size + player_overlap];
        let mut player_hits = Vec::<(&str, u64, u64)>::new();
        let mut structured_rewards = BTreeMap::<&str, BTreeSet<String>>::new();
        let mut bytes_read = 0_u64;

        'regions: for region in &regions {
            let mut offset = 0_usize;
            let mut retained = 0_usize;
            while offset < region.len() {
                if started.elapsed() >= self.timeout || bytes_read >= player_byte_budget {
                    break 'regions;
                }
                let remaining_budget =
                    usize::try_from(player_byte_budget - bytes_read).unwrap_or(usize::MAX);
                let request = (region.len() - offset)
                    .min(self.chunk_size)
                    .min(remaining_budget);
                if request == 0 {
                    break 'regions;
                }
                let address = region.start() + u64::try_from(offset).unwrap_or(u64::MAX);
                let read =
                    memory.read_at(process, address, &mut buffer[retained..retained + request])?;
                if read == 0 {
                    break;
                }
                bytes_read += u64::try_from(read).unwrap_or(0);
                let available = retained + read;
                let base = address.saturating_sub(u64::try_from(retained).unwrap_or(0));
                for found in player_matcher.find_overlapping_iter(&buffer[..available]) {
                    let hit_address = base + u64::try_from(found.start()).unwrap_or(0);
                    let identity = responders[found.pattern().as_usize()];
                    let record_start = hit_address.saturating_sub(1).max(region.start());
                    let region_end = region.start() + u64::try_from(region.len()).unwrap_or(0);
                    let mut record = [0_u8; 768];
                    let request = usize::try_from(region_end.saturating_sub(record_start))
                        .unwrap_or(0)
                        .min(record.len());
                    let record_read = if request == 0 {
                        0
                    } else {
                        memory.read_at(process, record_start, &mut record[..request])?
                    };
                    if let Some(choice) =
                        structured_response_reward(&record[..record_read], identity, candidates)
                    {
                        structured_rewards
                            .entry(identity)
                            .or_default()
                            .insert(choice.to_owned());
                        if let Some(choices) = confirmed_structured_choices(
                            responders,
                            local_identity,
                            local_choice,
                            &structured_rewards,
                        ) {
                            record.zeroize();
                            buffer.zeroize();
                            return Ok(RewardResolution::Confirmed {
                                choices,
                                region_start: 0,
                            });
                        }
                    }
                    record.zeroize();
                    player_hits.push((identity, hit_address, region.start()));
                }
                retained = player_overlap.min(available);
                buffer.copy_within(available - retained..available, 0);
                offset += read;
            }
        }
        buffer.zeroize();

        let reward_matcher = AhoCorasick::new(
            reward_patterns
                .iter()
                .map(|(_, pattern)| pattern.as_slice())
                .collect::<Vec<_>>(),
        )
        .map_err(|_| AcquisitionError::SnapshotInvalid)?;
        let reward_overlap = reward_patterns
            .iter()
            .map(|(_, pattern)| pattern.len())
            .max()
            .unwrap_or(1)
            .saturating_sub(1);
        let mut windows = player_hits
            .iter()
            .filter_map(|(_, address, region_start)| {
                let region = regions
                    .iter()
                    .find(|region| region.start() == *region_start)?;
                let region_end = region.start() + u64::try_from(region.len()).ok()?;
                Some((
                    address.saturating_sub(32 * 1024).max(region.start()),
                    address.saturating_add(32 * 1024).min(region_end),
                    *region_start,
                ))
            })
            .collect::<Vec<_>>();
        windows.sort_unstable();
        let mut merged_windows = Vec::<(u64, u64, u64)>::new();
        for (start, end, region_start) in windows {
            if let Some((_, previous_end, previous_region)) = merged_windows.last_mut()
                && *previous_region == region_start
                && start <= *previous_end
            {
                *previous_end = (*previous_end).max(end);
            } else {
                merged_windows.push((start, end, region_start));
            }
        }

        let mut reward_hits = Vec::<(&str, u64, u64)>::new();
        let mut buffer = vec![0_u8; self.chunk_size + reward_overlap];
        'windows: for (window_start, window_end, region_start) in merged_windows {
            let mut address = window_start;
            let mut retained = 0_usize;
            while address < window_end {
                if started.elapsed() >= self.timeout {
                    break 'windows;
                }
                let request = usize::try_from(window_end - address)
                    .unwrap_or(usize::MAX)
                    .min(self.chunk_size);
                let read =
                    memory.read_at(process, address, &mut buffer[retained..retained + request])?;
                if read == 0 {
                    break;
                }
                let available = retained + read;
                let base = address.saturating_sub(u64::try_from(retained).unwrap_or(0));
                for found in reward_matcher.find_overlapping_iter(&buffer[..available]) {
                    reward_hits.push((
                        reward_patterns[found.pattern().as_usize()].0,
                        base + u64::try_from(found.start()).unwrap_or(0),
                        region_start,
                    ));
                }
                retained = reward_overlap.min(available);
                buffer.copy_within(available - retained..available, 0);
                address += u64::try_from(read).unwrap_or(0);
            }
        }
        buffer.zeroize();

        let reward_for = |identity: &str| {
            if let Some(names) = structured_rewards.get(identity)
                && names.len() == 1
            {
                return names.iter().next().cloned();
            }
            let mut inline_names = BTreeSet::new();
            let mut retained_names = BTreeSet::new();
            for (_, player_address, region_start) in player_hits
                .iter()
                .filter(|(found, _, _)| *found == identity)
            {
                for (name, reward_address, reward_region) in &reward_hits {
                    let distance = reward_address.abs_diff(*player_address);
                    if reward_region != region_start {
                        continue;
                    }
                    if (64..=256).contains(&distance) {
                        inline_names.insert((*name).to_owned());
                    } else if (257..=32 * 1024).contains(&distance) {
                        retained_names.insert((*name).to_owned());
                    }
                }
            }
            let names = if inline_names.is_empty() {
                retained_names
            } else {
                inline_names
            };
            (names.len() == 1)
                .then(|| names.into_iter().next())
                .flatten()
        };

        if responders.len() == 1 && local_choice.is_none() {
            return Ok(match reward_for(responders[0]) {
                Some(choice) => RewardResolution::Confirmed {
                    choices: vec![choice],
                    region_start: 0,
                },
                None => RewardResolution::Incomplete,
            });
        }

        let inferred_local = local_identity.or_else(|| {
            let local_choice = local_choice?;
            responders
                .iter()
                .copied()
                .find(|identity| reward_for(identity).as_deref() == Some(local_choice))
        });
        let (Some(local_identity), Some(local_choice)) = (inferred_local, local_choice) else {
            return Ok(RewardResolution::Incomplete);
        };
        let mut choices = vec![local_choice.to_owned()];
        for identity in responders
            .iter()
            .copied()
            .filter(|identity| *identity != local_identity)
        {
            let Some(choice) = reward_for(identity) else {
                return Ok(RewardResolution::Incomplete);
            };
            choices.push(choice);
        }
        if choices.len() != responders.len() {
            return Ok(RewardResolution::Incomplete);
        }
        Ok(RewardResolution::Confirmed {
            choices,
            region_start: 0,
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub fn confirm_region(
        &self,
        memory: &dyn MemoryReader,
        process: &GameProcess,
        candidates: &[RewardNeedle],
        region_start: u64,
        region_len: usize,
        expected_choices: usize,
        maximum_span: u64,
    ) -> Result<RewardResolution, AcquisitionError> {
        let current = self.fingerprint_regions(
            memory,
            process,
            candidates,
            Some((region_start, region_len)),
        )?;
        let baseline = RewardFingerprint {
            hits: Vec::new(),
            bytes_read: 0,
            elapsed: Duration::ZERO,
        };
        Ok(resolve_reward_choices(
            &baseline,
            &current,
            expected_choices,
            maximum_span,
        ))
    }

    fn fingerprint_regions(
        &self,
        memory: &dyn MemoryReader,
        process: &GameProcess,
        candidates: &[RewardNeedle],
        selected_region: Option<(u64, usize)>,
    ) -> Result<RewardFingerprint, AcquisitionError> {
        let started = Instant::now();
        let mut regions = memory.readable_regions(process)?;
        if let Some((start, len)) = selected_region {
            regions.retain(|region| region.start() == start);
            for region in &mut regions {
                *region = crate::ReadableRegion::classified(
                    region.start(),
                    region.len().min(len),
                    region.scan_priority(),
                );
            }
        }
        regions.sort_by_key(|region| {
            (
                priority_rank(region.scan_priority()),
                !is_live_ui_region(region.start()),
                std::cmp::Reverse(region.start()),
            )
        });
        let longest = candidates
            .iter()
            .flat_map(|candidate| {
                std::iter::once(candidate.display_name.len())
                    .chain(candidate.internal_paths.iter().map(Vec::len))
            })
            .max()
            .unwrap_or(1);
        let overlap = longest.saturating_sub(1);
        let mut patterns = Vec::<&[u8]>::new();
        let mut pattern_metadata = Vec::<(&str, RewardRepresentation)>::new();
        for candidate in candidates {
            patterns.push(&candidate.display_name);
            pattern_metadata.push((candidate.choice_name(), RewardRepresentation::DisplayName));
            for path in &candidate.internal_paths {
                patterns.push(path);
                pattern_metadata
                    .push((candidate.choice_name(), RewardRepresentation::InternalPath));
            }
        }
        let matcher = AhoCorasick::new(patterns).map_err(|_| AcquisitionError::SnapshotInvalid)?;
        let mut buffer = vec![0_u8; self.chunk_size + overlap];
        let mut hits = Vec::new();
        let mut seen = BTreeSet::new();
        let mut bytes_read = 0_u64;

        'regions: for region in regions {
            let region_len = if is_live_ui_region(region.start()) {
                region.len().min(
                    usize::try_from(LIVE_UI_ADDRESS_MAX - region.start()).unwrap_or(usize::MAX),
                )
            } else {
                region.len()
            };
            let mut offset = 0_usize;
            let mut retained = 0_usize;
            while offset < region_len {
                if started.elapsed() >= self.timeout || bytes_read >= self.byte_budget {
                    break 'regions;
                }
                let remaining_budget =
                    usize::try_from(self.byte_budget - bytes_read).unwrap_or(usize::MAX);
                let request = (region_len - offset)
                    .min(self.chunk_size)
                    .min(remaining_budget);
                if request == 0 {
                    break 'regions;
                }
                let address = region.start() + u64::try_from(offset).unwrap_or(u64::MAX);
                let read =
                    memory.read_at(process, address, &mut buffer[retained..retained + request])?;
                if read == 0 {
                    break;
                }
                bytes_read += u64::try_from(read).unwrap_or(0);
                let available = retained + read;
                let base = address.saturating_sub(u64::try_from(retained).unwrap_or(0));
                for found in matcher.find_overlapping_iter(&buffer[..available]) {
                    let (choice_name, representation) =
                        pattern_metadata[found.pattern().as_usize()];
                    let hit_address = base + u64::try_from(found.start()).unwrap_or(0);
                    if seen.insert((hit_address, representation)) {
                        hits.push(RewardHit {
                            choice_name: choice_name.to_owned(),
                            address: hit_address,
                            region_start: region.start(),
                            priority: region.scan_priority(),
                            representation,
                        });
                    }
                }
                retained = overlap.min(available);
                buffer.copy_within(available - retained..available, 0);
                offset += read;
            }
        }
        buffer.zeroize();
        Ok(RewardFingerprint {
            hits,
            bytes_read,
            elapsed: started.elapsed(),
        })
    }
}

fn confirmed_structured_choices(
    responders: &[&str],
    local_identity: Option<&str>,
    local_choice: Option<&str>,
    rewards: &BTreeMap<&str, BTreeSet<String>>,
) -> Option<Vec<String>> {
    let unique_reward = |identity: &str| {
        let names = rewards.get(identity)?;
        (names.len() == 1)
            .then(|| names.iter().next().cloned())
            .flatten()
    };
    if responders.len() == 1 && local_choice.is_none() {
        return Some(vec![unique_reward(responders[0])?]);
    }

    let local_identity = local_identity?;
    let mut choices = vec![local_choice?.to_owned()];
    for identity in responders
        .iter()
        .copied()
        .filter(|identity| *identity != local_identity)
    {
        choices.push(unique_reward(identity)?);
    }
    (choices.len() == responders.len()).then_some(choices)
}

fn structured_response_reward<'a>(
    bytes: &[u8],
    identity: &str,
    candidates: &'a [RewardNeedle],
) -> Option<&'a str> {
    if bytes.first().copied() != Some(identity.len() as u8)
        || bytes.get(1..1 + identity.len()) != Some(identity.as_bytes())
    {
        return None;
    }
    let search = bytes.get(1 + identity.len()..)?;
    let path_start = search
        .windows(b"/Lotus/StoreItems/".len())
        .position(|window| window == b"/Lotus/StoreItems/")?;
    let encoded_length = path_start
        .checked_sub(2)
        .and_then(|offset| search.get(offset))
        .copied()
        .map(usize::from)?;
    if encoded_length < b"/Lotus/StoreItems/".len() {
        return None;
    }
    let path = search.get(path_start..path_start.checked_add(encoded_length)?)?;
    let response_basename = path.rsplit(|byte| *byte == b'/').next()?;
    let mut matches = BTreeSet::new();
    for candidate in candidates {
        let compact_choice = candidate
            .choice_name()
            .bytes()
            .filter(|byte| byte.is_ascii_alphanumeric())
            .collect::<Vec<_>>();
        if response_basename == compact_choice {
            matches.insert(candidate.choice_name());
            continue;
        }
        for internal_path in candidate.internal_paths() {
            let basename = internal_path.rsplit(|byte| *byte == b'/').next()?;
            if !basename.is_empty() && response_basename == basename {
                matches.insert(candidate.choice_name());
            }
        }
    }
    (matches.len() == 1)
        .then(|| matches.into_iter().next())
        .flatten()
}

fn priority_rank(priority: RegionScanPriority) -> u8 {
    match priority {
        RegionScanPriority::WritableAnonymous => 0,
        RegionScanPriority::WritablePrivateFileBacked => 1,
        RegionScanPriority::Anonymous => 2,
        RegionScanPriority::FileBacked => 3,
    }
}

const fn is_live_ui_region(start: u64) -> bool {
    start >= LIVE_UI_ADDRESS_MIN && start < LIVE_UI_ADDRESS_MAX
}
