use std::time::{Duration, Instant};

#[cfg(debug_assertions)]
use warframe_acquisition::append_debug_line;

use warframe_acquisition::{
    GameProcess, MemoryReader, PersistentRewardResolver, RewardCatalogEntry, RewardFingerprint,
    RewardMemoryScanner, RewardNeedle, RewardResolution,
};

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
    fn player_records(
        &mut self,
        responders: &[&str],
        local_identity: Option<&str>,
        local_choice: Option<&str>,
    ) -> RewardResolution;
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

    pub fn prepare_candidates(&mut self, candidates: &[RewardNeedle]) {
        self.candidates = candidates.to_vec();
        self.baseline = None;
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
        #[cfg(debug_assertions)]
        trace_fingerprint("baseline", self.state.baseline.as_ref(), None);
    }

    fn choices(&mut self, expected: usize) -> RewardResolution {
        let started = Instant::now();
        let resolution = PersistentRewardResolver::new(
            256 * 1024,
            512 * 1024 * 1024,
            Duration::from_millis(2_500),
        )
        .resolve(self.memory, &self.process, &self.state.candidates, expected)
        .unwrap_or(RewardResolution::Incomplete);
        #[cfg(debug_assertions)]
        trace_persistent_choices(expected, started.elapsed(), &resolution);
        resolution
    }

    fn player_records(
        &mut self,
        responders: &[&str],
        local_identity: Option<&str>,
        local_choice: Option<&str>,
    ) -> RewardResolution {
        let started = Instant::now();
        let resolution = self
            .state
            .scanner
            .resolve_strict_player_records(
                self.memory,
                &self.process,
                &self.state.candidates,
                responders,
                local_identity,
                local_choice,
            )
            .unwrap_or(RewardResolution::Incomplete);
        #[cfg(debug_assertions)]
        trace_player_records(responders.len(), started.elapsed(), &resolution);
        resolution
    }
}

#[cfg(debug_assertions)]
fn trace_fingerprint(
    phase: &str,
    fingerprint: Option<&RewardFingerprint>,
    resolution: Option<&RewardResolution>,
) {
    let Some(fingerprint) = fingerprint else {
        append_debug_line(&format!("[DEBUG-reward] phase={phase} scan=failed"));
        return;
    };
    append_debug_line(&format!(
        "[DEBUG-reward] phase={phase} bytes={} elapsed_ms={} hits={} resolution={resolution:?}",
        fingerprint.bytes_read(),
        fingerprint.elapsed().as_millis(),
        fingerprint.hits().len(),
    ));
    for hit in fingerprint.hits() {
        append_debug_line(&format!(
            "[DEBUG-reward] hit phase={phase} region={} offset={} priority={:?} representation={:?} name={:?}",
            hit.region_start(),
            hit.address() - hit.region_start(),
            hit.priority(),
            hit.representation(),
            hit.choice_name(),
        ));
    }
}

#[cfg(debug_assertions)]
fn trace_player_records(responder_count: usize, elapsed: Duration, resolution: &RewardResolution) {
    append_debug_line(&format!(
        "[DEBUG-player-record] responders={responder_count} elapsed_ms={} resolution={resolution:?}",
        elapsed.as_millis(),
    ));
}

#[cfg(debug_assertions)]
fn trace_persistent_choices(expected: usize, elapsed: Duration, resolution: &RewardResolution) {
    append_debug_line(&format!(
        "[DEBUG-persistent-ui] expected={expected} elapsed_ms={} resolution={resolution:?}",
        elapsed.as_millis(),
    ));
}

impl RewardSourceCoordinator {
    pub const fn new(validation_mode: bool) -> Self {
        Self { validation_mode }
    }

    pub fn baseline(&mut self, memory: &mut dyn MemoryRewardSource, candidates: &[RewardNeedle]) {
        memory.baseline(candidates);
    }

    pub fn player_record_choices(
        &self,
        memory: &mut dyn MemoryRewardSource,
        responders: &[&str],
        local_identity: Option<&str>,
        local_choice: Option<&str>,
    ) -> Option<RewardSourceResult> {
        let started = Instant::now();
        let RewardResolution::Confirmed { choices, .. } =
            memory.player_records(responders, local_identity, local_choice)
        else {
            return None;
        };
        Some(RewardSourceResult {
            choices: RewardChoiceSet {
                names: choices,
                source: RewardChoiceSource::Memory,
                elapsed: started.elapsed(),
            },
            diagnostic: RewardSourceDiagnostic::Ready,
        })
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
            | RewardResolution::TimedOut => visual
                .choices(candidates)
                .ok()
                .filter(|names| names.len() == expected)
                .map(|names| RewardSourceResult {
                    choices: RewardChoiceSet {
                        names,
                        source: RewardChoiceSource::Ocr,
                        elapsed: started.elapsed(),
                    },
                    diagnostic: RewardSourceDiagnostic::MemoryFallback,
                }),
        }
    }
}
