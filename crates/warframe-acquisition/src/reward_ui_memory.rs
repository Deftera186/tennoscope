use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

use crate::{
    AcquisitionError, GameProcess, MemoryReader, ReadableRegion, RegionScanPriority, RewardNeedle,
    RewardResolution,
};

const STRING_BASE_DELTAS: [u64; 7] = [0, 8, 16, 24, 32, 40, 48];
const OBJECT_BASE_SEARCH: u64 = 256;
const MAX_GRAPH_DEPTH: usize = 3;
const MIN_SLOT_STRIDE: u64 = 8;
const MAX_SLOT_STRIDE: u64 = 64;

pub struct PersistentRewardResolver {
    chunk_size: usize,
    byte_budget: u64,
    timeout: Duration,
}

#[derive(Clone)]
struct RegionBytes {
    region: ReadableRegion,
    bytes: Vec<u8>,
}

#[derive(Clone)]
struct PointerHit {
    location: u64,
    region_start: u64,
    target: u64,
}

struct ContainerCandidate {
    fields: Vec<(u64, u64)>,
    choices: Vec<String>,
    region_start: u64,
    stride: u64,
}

impl PersistentRewardResolver {
    pub fn new(chunk_size: usize, byte_budget: u64, timeout: Duration) -> Self {
        Self {
            chunk_size: chunk_size.max(8),
            byte_budget,
            timeout,
        }
    }

    pub fn resolve(
        &self,
        memory: &dyn MemoryReader,
        process: &GameProcess,
        candidates: &[RewardNeedle],
        expected_choices: usize,
    ) -> Result<RewardResolution, AcquisitionError> {
        if expected_choices < 2 || candidates.is_empty() {
            return Ok(RewardResolution::Incomplete);
        }
        let started = Instant::now();
        let regions = self.read_regions(memory, process, started)?;
        let mut targets = seed_targets(&regions, candidates);
        if targets.is_empty() {
            return Ok(RewardResolution::Incomplete);
        }

        for depth in 0..MAX_GRAPH_DEPTH {
            if started.elapsed() >= self.timeout {
                return Ok(RewardResolution::TimedOut);
            }
            let hits = pointer_hits(&regions, &targets);
            if depth > 0 {
                let containers = ordered_containers(&hits, &targets, expected_choices);
                match select_container(containers) {
                    Some(Ok(container)) => {
                        if confirm_container(memory, process, &container, &targets)? {
                            return Ok(RewardResolution::Confirmed {
                                choices: container.choices,
                                region_start: container.region_start,
                            });
                        }
                        return Ok(RewardResolution::Incomplete);
                    }
                    Some(Err(())) => return Ok(RewardResolution::Ambiguous),
                    None => {}
                }
            }
            targets = parent_targets(&hits, &targets);
            if targets.is_empty() {
                break;
            }
        }
        Ok(RewardResolution::Incomplete)
    }

    fn read_regions(
        &self,
        memory: &dyn MemoryReader,
        process: &GameProcess,
        started: Instant,
    ) -> Result<Vec<RegionBytes>, AcquisitionError> {
        if let Some(snapshots) = memory.recently_written_snapshot(process)? {
            let mut bytes_read = 0_u64;
            return Ok(snapshots
                .into_iter()
                .filter(|snapshot| {
                    snapshot.scan_priority() == RegionScanPriority::WritableAnonymous
                })
                .take_while(|snapshot| {
                    let next = bytes_read
                        .saturating_add(u64::try_from(snapshot.bytes().len()).unwrap_or(u64::MAX));
                    let accepted = started.elapsed() < self.timeout && next <= self.byte_budget;
                    if accepted {
                        bytes_read = next;
                    }
                    accepted
                })
                .map(|snapshot| RegionBytes {
                    region: ReadableRegion::classified(
                        snapshot.start(),
                        snapshot.bytes().len(),
                        snapshot.scan_priority(),
                    ),
                    bytes: snapshot.bytes().to_vec(),
                })
                .collect());
        }
        let mut regions = memory
            .recently_written_regions(process)
            .or_else(|_| memory.readable_regions(process))?;
        regions.retain(|region| region.scan_priority() == RegionScanPriority::WritableAnonymous);
        regions.sort_by_key(|region| region.start());
        let mut output = Vec::new();
        let mut bytes_read = 0_u64;
        for region in regions {
            if started.elapsed() >= self.timeout || bytes_read >= self.byte_budget {
                break;
            }
            let remaining = usize::try_from(self.byte_budget - bytes_read).unwrap_or(usize::MAX);
            let len = region.len().min(remaining);
            if len == 0 {
                break;
            }
            let mut bytes = vec![0_u8; len];
            let mut offset = 0_usize;
            while offset < len {
                let request = (len - offset).min(self.chunk_size);
                let read = memory.read_at(
                    process,
                    region.start() + u64::try_from(offset).unwrap_or(u64::MAX),
                    &mut bytes[offset..offset + request],
                )?;
                if read == 0 {
                    break;
                }
                offset += read;
            }
            bytes.truncate(offset);
            bytes_read += u64::try_from(offset).unwrap_or(0);
            if !bytes.is_empty() {
                output.push(RegionBytes { region, bytes });
            }
        }
        Ok(output)
    }
}

fn seed_targets(
    regions: &[RegionBytes],
    candidates: &[RewardNeedle],
) -> BTreeMap<u64, BTreeSet<String>> {
    let mut patterns = Vec::<(&str, &[u8])>::new();
    for candidate in candidates {
        patterns.push((candidate.choice_name(), candidate.choice_name().as_bytes()));
        for path in candidate.internal_paths() {
            patterns.push((candidate.choice_name(), path));
        }
    }
    let mut targets = BTreeMap::<u64, BTreeSet<String>>::new();
    for region in regions {
        for (choice, pattern) in &patterns {
            if pattern.is_empty() || pattern.len() > region.bytes.len() {
                continue;
            }
            for offset in find_all(&region.bytes, pattern) {
                let address = region.region.start() + u64::try_from(offset).unwrap_or(0);
                for delta in STRING_BASE_DELTAS {
                    if let Some(target) = address.checked_sub(delta) {
                        targets
                            .entry(target)
                            .or_default()
                            .insert((*choice).to_owned());
                    }
                }
            }
        }
    }
    targets
}

fn find_all(haystack: &[u8], needle: &[u8]) -> Vec<usize> {
    haystack
        .windows(needle.len())
        .enumerate()
        .filter_map(|(offset, window)| (window == needle).then_some(offset))
        .collect()
}

fn pointer_hits(
    regions: &[RegionBytes],
    targets: &BTreeMap<u64, BTreeSet<String>>,
) -> Vec<PointerHit> {
    let mut hits = Vec::new();
    for region in regions {
        let alignment = usize::try_from((8 - (region.region.start() % 8)) % 8).unwrap_or(0);
        for offset in (alignment..region.bytes.len().saturating_sub(7)).step_by(8) {
            let target = u64::from_le_bytes(
                region.bytes[offset..offset + 8]
                    .try_into()
                    .unwrap_or([0; 8]),
            );
            if targets.contains_key(&target) {
                hits.push(PointerHit {
                    location: region.region.start() + u64::try_from(offset).unwrap_or(0),
                    region_start: region.region.start(),
                    target,
                });
            }
        }
    }
    hits
}

fn parent_targets(
    hits: &[PointerHit],
    current: &BTreeMap<u64, BTreeSet<String>>,
) -> BTreeMap<u64, BTreeSet<String>> {
    let mut parents = BTreeMap::<u64, BTreeSet<String>>::new();
    for hit in hits {
        let Some(choices) = current.get(&hit.target) else {
            continue;
        };
        for delta in (0..=OBJECT_BASE_SEARCH).step_by(8) {
            let Some(base) = hit.location.checked_sub(delta) else {
                continue;
            };
            parents
                .entry(base)
                .or_default()
                .extend(choices.iter().cloned());
        }
    }
    parents
}

fn ordered_containers(
    hits: &[PointerHit],
    targets: &BTreeMap<u64, BTreeSet<String>>,
    expected: usize,
) -> Vec<ContainerCandidate> {
    let mut ordered = hits.to_vec();
    ordered.sort_by_key(|hit| (hit.region_start, hit.location));
    let mut containers = Vec::new();
    for start in 0..ordered.len() {
        for stride in (MIN_SLOT_STRIDE..=MAX_SLOT_STRIDE).step_by(8) {
            let mut fields = Vec::with_capacity(expected);
            let mut choices = Vec::with_capacity(expected);
            for slot in 0..expected {
                let location =
                    ordered[start].location + (u64::try_from(slot).unwrap_or(0) * stride);
                let Some(hit) = ordered.iter().find(|hit| {
                    hit.region_start == ordered[start].region_start && hit.location == location
                }) else {
                    break;
                };
                let Some(names) = targets.get(&hit.target) else {
                    break;
                };
                if names.len() != 1 {
                    break;
                }
                let name = names.iter().next().expect("one container choice");
                fields.push((hit.location, hit.target));
                choices.push((*name).clone());
            }
            if choices.len() == expected {
                containers.push(ContainerCandidate {
                    fields,
                    choices,
                    region_start: ordered[start].region_start,
                    stride,
                });
            }
        }
    }
    containers
}

fn select_container(
    mut containers: Vec<ContainerCandidate>,
) -> Option<Result<ContainerCandidate, ()>> {
    containers.sort_by_key(|container| (container.stride, container.fields[0].0));
    containers.dedup_by(|left, right| left.fields == right.fields && left.choices == right.choices);
    let distinct_orders = containers
        .iter()
        .map(|container| container.choices.clone())
        .collect::<BTreeSet<_>>();
    if distinct_orders.len() > 1 {
        return Some(Err(()));
    }
    let first = containers.into_iter().next()?;
    Some(Ok(first))
}

fn confirm_container(
    memory: &dyn MemoryReader,
    process: &GameProcess,
    container: &ContainerCandidate,
    targets: &BTreeMap<u64, BTreeSet<String>>,
) -> Result<bool, AcquisitionError> {
    for ((address, expected_target), expected_choice) in
        container.fields.iter().zip(&container.choices)
    {
        let mut pointer = [0_u8; 8];
        if memory.read_at(process, *address, &mut pointer)? != pointer.len() {
            return Ok(false);
        }
        let target = u64::from_le_bytes(pointer);
        if target != *expected_target
            || !targets
                .get(&target)
                .is_some_and(|choices| choices.len() == 1 && choices.contains(expected_choice))
        {
            return Ok(false);
        }
    }
    Ok(true)
}
