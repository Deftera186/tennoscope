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
            let mut offset = 0_usize;
            let mut retained = 0_usize;
            while offset < region.len() {
                if started.elapsed() >= self.timeout || bytes_read >= self.byte_budget {
                    break 'regions;
                }
                let remaining_budget =
                    usize::try_from(self.byte_budget - bytes_read).unwrap_or(usize::MAX);
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
