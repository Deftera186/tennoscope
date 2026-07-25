use std::time::{Duration, Instant};

use warframe_acquisition::{
    GameProcess, MemoryReader, RewardCatalogEntry, RewardFingerprint, RewardMemoryScanner,
    RewardNeedle, RewardResolution, resolve_reward_choices,
};

const MAXIMUM_REWARD_CLUSTER_SPAN: u64 = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RewardChoiceSource {
    Memory,
    Ocr,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewardChoiceSet {
    pub names: Vec<String>,
    pub source: RewardChoiceSource,
    pub elapsed: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RewardSourceDiagnostic {
    Ready,
    MemoryFallback,
    Agreement,
    Disagreement,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewardSourceResult {
    pub choices: RewardChoiceSet,
    pub diagnostic: RewardSourceDiagnostic,
}

pub trait MemoryRewardSource {
    fn baseline(&mut self, candidates: &[RewardNeedle]);
    fn choices(&mut self, expected: usize) -> RewardResolution;
}

pub trait VisualRewardSource {
    fn choices(&mut self, candidates: &[RewardCatalogEntry]) -> Result<Vec<String>, &'static str>;
}

pub struct RewardSourceCoordinator {
    validation_mode: bool,
}

pub struct LiveMemoryRewardState {
    scanner: RewardMemoryScanner,
    candidates: Vec<RewardNeedle>,
    baseline: Option<RewardFingerprint>,
}

impl LiveMemoryRewardState {
    pub fn new(scanner: RewardMemoryScanner) -> Self {
        Self {
            scanner,
            candidates: Vec::new(),
            baseline: None,
        }
    }

    pub fn candidates(&self) -> &[RewardNeedle] {
        &self.candidates
    }

    pub fn clear(&mut self) {
        self.candidates.clear();
        self.baseline = None;
    }

    pub fn bind<'a>(
        &'a mut self,
        memory: &'a dyn MemoryReader,
        process: GameProcess,
    ) -> BoundMemoryRewardSource<'a> {
        BoundMemoryRewardSource {
            state: self,
            memory,
            process,
        }
    }
}

pub struct BoundMemoryRewardSource<'a> {
    state: &'a mut LiveMemoryRewardState,
    memory: &'a dyn MemoryReader,
    process: GameProcess,
}

impl MemoryRewardSource for BoundMemoryRewardSource<'_> {
    fn baseline(&mut self, candidates: &[RewardNeedle]) {
        self.state.candidates = candidates.to_vec();
        self.state.baseline = self
            .state
            .scanner
            .fingerprint(self.memory, &self.process, candidates)
            .ok();
    }

    fn choices(&mut self, expected: usize) -> RewardResolution {
        let Some(baseline) = self.state.baseline.as_ref() else {
            return RewardResolution::Incomplete;
        };
        let Ok(current) =
            self.state
                .scanner
                .fingerprint(self.memory, &self.process, &self.state.candidates)
        else {
            return RewardResolution::Incomplete;
        };
        let resolution =
            resolve_reward_choices(baseline, &current, expected, MAXIMUM_REWARD_CLUSTER_SPAN);
        let RewardResolution::Confirmed { region_start, .. } = resolution else {
            return resolution;
        };
        let Ok(regions) = self.memory.readable_regions(&self.process) else {
            return RewardResolution::Incomplete;
        };
        let Some(region_len) = regions
            .iter()
            .find(|region| region.start() == region_start)
            .map(|region| region.len())
        else {
            return RewardResolution::Incomplete;
        };
        self.state
            .scanner
            .confirm_region(
                self.memory,
                &self.process,
                &self.state.candidates,
                region_start,
                region_len,
                expected,
                MAXIMUM_REWARD_CLUSTER_SPAN,
            )
            .unwrap_or(RewardResolution::Incomplete)
    }
}

impl RewardSourceCoordinator {
    pub const fn new(validation_mode: bool) -> Self {
        Self { validation_mode }
    }

    pub fn baseline(&mut self, memory: &mut dyn MemoryRewardSource, candidates: &[RewardNeedle]) {
        if !candidates.is_empty() {
            memory.baseline(candidates);
        }
    }

    pub fn choices(
        &self,
        memory: &mut dyn MemoryRewardSource,
        visual: &mut dyn VisualRewardSource,
        expected: usize,
        candidates: &[RewardCatalogEntry],
    ) -> Option<RewardSourceResult> {
        if expected <= 1 {
            return None;
        }
        let started = Instant::now();
        match memory.choices(expected) {
            RewardResolution::Confirmed { choices, .. } => {
                let diagnostic = if self.validation_mode {
                    match visual.choices(candidates) {
                        Ok(visual_choices) if visual_choices == choices => {
                            RewardSourceDiagnostic::Agreement
                        }
                        _ => RewardSourceDiagnostic::Disagreement,
                    }
                } else {
                    RewardSourceDiagnostic::Ready
                };
                Some(RewardSourceResult {
                    choices: RewardChoiceSet {
                        names: choices,
                        source: RewardChoiceSource::Memory,
                        elapsed: started.elapsed(),
                    },
                    diagnostic,
                })
            }
            RewardResolution::Incomplete
            | RewardResolution::Ambiguous
            | RewardResolution::TimedOut => {
                visual
                    .choices(candidates)
                    .ok()
                    .map(|names| RewardSourceResult {
                        choices: RewardChoiceSet {
                            names,
                            source: RewardChoiceSource::Ocr,
                            elapsed: started.elapsed(),
                        },
                        diagnostic: RewardSourceDiagnostic::MemoryFallback,
                    })
            }
        }
    }
}
