use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

/// How long to wait between screen reads while the reward cards are still being painted. Capture
/// plus four crops and OCR costs roughly 150ms, so this paces attempts without spinning.
const VISUAL_RETRY_INTERVAL: Duration = Duration::from_millis(200);

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
        trace_player_records(responders.len(), started.elapsed(), &resolution);
        resolution
    }
}

fn trace_fingerprint(
    phase: &str,
    fingerprint: Option<&RewardFingerprint>,
    resolution: Option<&RewardResolution>,
) {
    let Some(fingerprint) = fingerprint else {
        log::debug!("[DEBUG-reward] phase={phase} scan=failed");
        return;
    };
    log::debug!(
        "[DEBUG-reward] phase={phase} bytes={} elapsed_ms={} hits={} resolution={resolution:?}",
        fingerprint.bytes_read(),
        fingerprint.elapsed().as_millis(),
        fingerprint.hits().len(),
    );
    for hit in fingerprint.hits() {
        log::debug!(
            "[DEBUG-reward] hit phase={phase} region={} offset={} priority={:?} representation={:?} name={:?}",
            hit.region_start(),
            hit.address() - hit.region_start(),
            hit.priority(),
            hit.representation(),
            hit.choice_name(),
        );
    }
}

fn trace_player_records(responder_count: usize, elapsed: Duration, resolution: &RewardResolution) {
    log::debug!(
        "[DEBUG-player-record] responders={responder_count} elapsed_ms={} resolution={resolution:?}",
        elapsed.as_millis(),
    );
}

fn trace_persistent_choices(expected: usize, elapsed: Duration, resolution: &RewardResolution) {
    log::debug!(
        "[DEBUG-persistent-ui] expected={expected} elapsed_ms={} resolution={resolution:?}",
        elapsed.as_millis(),
    );
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

    /// Read the cards off the screen.
    ///
    /// Memory holds the four rewards but nothing tying them to a player or a slot, hosting or not,
    /// so this is the only source for three of the four cards. EE.log states the local player's own
    /// reward exactly, which gives a free check: a read that does not contain it is wrong somewhere,
    /// and is dropped rather than shown.
    ///
    /// The log announces the rewards about three milliseconds before Warframe paints the cards, so
    /// a single capture reads an empty screen and finds nothing. Retry until the cards exist or the
    /// deadline passes; the screen itself lives for fifteen seconds.
    ///
    /// `abort` ends the retry early, and it matters more than it looks. This runs synchronously on
    /// the monitor thread, and that same thread is the one that watches for the screen going away
    /// and takes the overlay down. EE.log's flush delay means this can easily be entered *after*
    /// the screen has already closed, and without a way out it then spends the full deadline
    /// retrying against a screen that is gone -- with the monitor blocked behind it, unable to
    /// hide an overlay it has already been told to hide. That is seconds of the overlay sitting
    /// over the game after the rewards are no longer there.
    pub fn visual_choices(
        &self,
        visual: &mut dyn VisualRewardSource,
        candidates: &[RewardCatalogEntry],
        expected: usize,
        local_choice: Option<&str>,
        deadline: Duration,
        abort: &AtomicBool,
    ) -> Option<RewardSourceResult> {
        let started = Instant::now();
        let mut attempts = 0_u32;
        loop {
            if abort.load(Ordering::Acquire) {
                log::debug!(
                    "[DEBUG-visual] aborted after {}ms: the screen is already gone",
                    started.elapsed().as_millis()
                );
                return None;
            }
            attempts += 1;
            let attempt = visual.choices(candidates);
            trace_visual_read(attempts, started.elapsed(), &attempt);
            if let Some(names) =
                attempt
                    .ok()
                    .filter(|names| names.len() == expected)
                    .filter(|names| {
                        local_choice.is_none_or(|local| names.iter().any(|name| name == local))
                    })
            {
                return Some(RewardSourceResult {
                    choices: RewardChoiceSet {
                        names,
                        source: RewardChoiceSource::Ocr,
                        elapsed: started.elapsed(),
                    },
                    diagnostic: RewardSourceDiagnostic::MemoryFallback,
                });
            }
            if started.elapsed() >= deadline {
                return None;
            }
            std::thread::sleep(
                VISUAL_RETRY_INTERVAL.min(deadline.saturating_sub(started.elapsed())),
            );
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

/// The screen read was silent in the log, which is why a blank first capture looked identical to
/// OCR never running at all. Trace every attempt.
fn trace_visual_read(attempt: u32, elapsed: Duration, outcome: &Result<Vec<String>, &'static str>) {
    log::debug!(
        "[DEBUG-visual] attempt={attempt} elapsed_ms={} outcome={outcome:?}",
        elapsed.as_millis(),
    );
}
