# Memory-Gated Relic Reward Recognition Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build an event-gated, candidate-limited experimental Warframe memory recognizer that can identify online relic reward choices quickly and fall back to OCR when validation fails.

**Architecture:** `warframe-acquisition` gains a pure temporal scanner that compares candidate occurrences before and after reward initialization using the existing `MemoryReader` seam. The Tauri monitor parses reward lifecycle events and loaded relic paths, coordinates scans only inside the reward window, and preserves the existing OCR adapter as fallback. Live captures remain experimental until three online runs validate the extraction rule.

**Tech Stack:** Rust, existing Linux `/proc` adapter, Tauri monitor thread, WFCD catalog data, deterministic synthetic memory fixtures, existing Tesseract fallback.

---

### Task 1: Candidate-Limited Memory Search Core

**Files:**
- Create: `crates/warframe-acquisition/src/reward_memory.rs`
- Modify: `crates/warframe-acquisition/src/lib.rs`
- Test: `crates/warframe-acquisition/tests/reward_memory.rs`

- [ ] **Step 1: Write failing tests for candidate encoding and region priority**

Define tests that construct a fake `MemoryReader` with writable-anonymous and file-backed regions. Assert that `RewardMemoryScanner::fingerprint` finds UTF-8 display names and internal paths, scans writable-anonymous regions first, and does not read file-backed regions after every candidate has enough occurrences.

- [ ] **Step 2: Run the focused test and verify failure**

Run: `cargo test -p warframe-acquisition --test reward_memory -- --nocapture`

Expected: compilation fails because `RewardMemoryScanner`, `RewardNeedle`, and `RewardFingerprint` do not exist.

- [ ] **Step 3: Implement bounded candidate search**

Add these public contracts:

```rust
pub struct RewardNeedle {
    pub choice_name: String,
    pub display_name: Vec<u8>,
    pub internal_paths: Vec<Vec<u8>>,
}

pub struct RewardHit {
    pub choice_name: String,
    pub address: u64,
    pub region_start: u64,
    pub priority: RegionScanPriority,
    pub representation: RewardRepresentation,
}

pub struct RewardFingerprint {
    hits: Vec<RewardHit>,
    bytes_read: u64,
    elapsed: Duration,
}

pub struct RewardMemoryScanner {
    chunk_size: usize,
    byte_budget: u64,
    timeout: Duration,
}
```

Sort regions by `RegionScanPriority`, reuse one buffer, preserve only the longest-needle overlap, check deadline and byte budget before each read, and wipe the buffer before returning.

- [ ] **Step 4: Run focused tests**

Run: `cargo test -p warframe-acquisition --test reward_memory -- --nocapture`

Expected: all candidate search tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/warframe-acquisition/src/reward_memory.rs crates/warframe-acquisition/src/lib.rs crates/warframe-acquisition/tests/reward_memory.rs
git commit -m "feat: add bounded reward memory scanner"
```

### Task 2: Temporal Difference and Cluster Validation

**Files:**
- Modify: `crates/warframe-acquisition/src/reward_memory.rs`
- Modify: `crates/warframe-acquisition/tests/reward_memory.rs`

- [ ] **Step 1: Write failing temporal validation tests**

Create fixtures containing general catalog strings in both baseline and reward scans, plus four new candidate occurrences in one post-event region. Assert that `resolve_reward_choices` removes unchanged occurrences, selects the four-choice cluster, preserves address order, and rejects equal competing clusters.

- [ ] **Step 2: Run and verify failure**

Run: `cargo test -p warframe-acquisition --test reward_memory temporal -- --nocapture`

Expected: compilation fails because `resolve_reward_choices` and `RewardResolution` do not exist.

- [ ] **Step 3: Implement temporal subtraction and ranking**

Add:

```rust
pub enum RewardResolution {
    Confirmed { choices: Vec<String>, region_start: u64 },
    Incomplete,
    Ambiguous,
    TimedOut,
}

pub fn resolve_reward_choices(
    baseline: &RewardFingerprint,
    current: &RewardFingerprint,
    expected_choices: usize,
    maximum_span: u64,
) -> RewardResolution;
```

Subtract hits by candidate, representation, region-relative offset, and region priority. Group remaining hits by region, deduplicate candidate names, reject clusters wider than `maximum_span`, require the expected distinct count, and reject tied clusters.

- [ ] **Step 4: Add confirmation-read tests**

Test that `confirm_region` rereads only the selected region range, returns the same order for a stable cluster, and rejects changed or missing candidates.

- [ ] **Step 5: Run focused tests and commit**

Run: `cargo test -p warframe-acquisition --test reward_memory -- --nocapture`

```bash
git add crates/warframe-acquisition/src/reward_memory.rs crates/warframe-acquisition/tests/reward_memory.rs
git commit -m "feat: validate temporal reward memory clusters"
```

### Task 3: Reward Lifecycle Parser

**Files:**
- Create: `app/src-tauri/src/reward_log.rs`
- Modify: `app/src-tauri/src/lib.rs`
- Test: `app/src-tauri/tests/reward_log.rs`

- [ ] **Step 1: Write failing log-sequence tests**

Feed realistic line sequences containing projection resource loads, `OpenVoidProjectionRewardScreenRMI`, waiting-client lines, `Got rewards`, selection completion, and shutdown. Assert emitted events and online choice counts. Add a solo sequence and assert that it never requests multi-choice scanning.

- [ ] **Step 2: Run and verify failure**

Run: `cargo test -p warframe-helper --test reward_log -- --nocapture`

Expected: compilation fails because `RewardLogMachine` and `RewardLogEvent` do not exist.

- [ ] **Step 3: Implement the state machine**

Expose:

```rust
pub enum RewardLogEvent {
    BaselineRequested { relic_paths: Vec<String> },
    ChoicesReady { expected_choices: usize },
    Closed,
}

pub struct RewardLogMachine {
    loaded_relics: Vec<String>,
    waiting_clients: usize,
    reward_window_open: bool,
}
```

Deduplicate loaded relic paths, clear state after shutdown/mission reset, emit baseline once per window, derive expected choices from reward responses/waiting clients, and suppress scanning when the result is one.

- [ ] **Step 4: Run tests and commit**

Run: `cargo test -p warframe-helper --test reward_log -- --nocapture`

```bash
git add app/src-tauri/src/reward_log.rs app/src-tauri/src/lib.rs app/src-tauri/tests/reward_log.rs
git commit -m "feat: parse relic reward lifecycle events"
```

### Task 4: Relic Candidate Index

**Files:**
- Create: `crates/warframe-acquisition/src/relic_catalog.rs`
- Modify: `crates/warframe-acquisition/src/lib.rs`
- Modify: `crates/warframe-acquisition/src/catalog_cache.rs`
- Test: `crates/warframe-acquisition/tests/relic_catalog.rs`

- [ ] **Step 1: Write failing WFCD relic mapping tests**

Use a minimal `Relics.json` fixture with two relics and reward item paths. Assert normalization from EE.log projection paths to relic identities and generation of deduplicated `RewardNeedle` values containing canonical display names and StoreItems/recipe path aliases.

- [ ] **Step 2: Run and verify failure**

Run: `cargo test -p warframe-acquisition --test relic_catalog -- --nocapture`

Expected: compilation fails because `RelicRewardIndex` does not exist.

- [ ] **Step 3: Implement and cache the relic index**

Add `RelicRewardIndex::from_wfcd_json`, `candidates_for_projection_paths`, a pinned WFCD `Relics.json` URL, and a separate atomically written cache generation. Reject malformed aggregate JSON but skip records that cannot map to inventory reward paths.

- [ ] **Step 4: Run catalog and cache tests**

Run: `cargo test -p warframe-acquisition --test relic_catalog --test catalog_cache -- --nocapture`

Expected: all tests pass and interrupted cache writes preserve the prior generation.

- [ ] **Step 5: Commit**

```bash
git add crates/warframe-acquisition/src/relic_catalog.rs crates/warframe-acquisition/src/lib.rs crates/warframe-acquisition/src/catalog_cache.rs crates/warframe-acquisition/tests/relic_catalog.rs
git commit -m "feat: map loaded relics to reward candidates"
```

### Task 5: Memory-First Coordinator with OCR Fallback

**Files:**
- Create: `app/src-tauri/src/reward_source.rs`
- Modify: `app/src-tauri/src/lib.rs`
- Modify: `app/src-tauri/src/reward_observer.rs`
- Test: `app/src-tauri/tests/reward_source.rs`

- [ ] **Step 1: Write failing coordinator tests**

Use fake memory and OCR adapters. Assert confirmed memory wins, incomplete/ambiguous/timed-out memory invokes OCR, disagreement is reported as degraded during validation mode, and solo events invoke neither multi-choice source.

- [ ] **Step 2: Run and verify failure**

Run: `cargo test -p warframe-helper --test reward_source -- --nocapture`

Expected: compilation fails because `RewardSourceCoordinator` does not exist.

- [ ] **Step 3: Implement source contracts and coordinator**

Define:

```rust
pub enum RewardChoiceSource { Memory, Ocr }

pub struct RewardChoiceSet {
    pub names: Vec<String>,
    pub source: RewardChoiceSource,
    pub elapsed: Duration,
}

pub trait MemoryRewardSource {
    fn baseline(&mut self, candidates: &[RewardNeedle]);
    fn choices(&mut self, expected: usize) -> RewardResolution;
}

pub trait VisualRewardSource {
    fn choices(&mut self, candidates: &[RewardCatalogEntry]) -> Result<Vec<RewardObservation>, &'static str>;
}
```

Run memory and OCR independently after `ChoicesReady`, publish confirmed memory immediately, retain OCR comparison diagnostics in experimental mode, and never block past the memory scanner timeout.

- [ ] **Step 4: Replace blind polling with lifecycle-driven calls**

Remove five-second unconditional OCR invocation. Feed complete new EE.log lines into `RewardLogMachine`; invoke baseline and choice acquisition only for emitted events; hide the overlay on `Closed`.

- [ ] **Step 5: Run focused tests and commit**

Run: `cargo test -p warframe-helper --test reward_source --test reward_log --test reward_observer -- --nocapture`

```bash
git add app/src-tauri/src/reward_source.rs app/src-tauri/src/reward_observer.rs app/src-tauri/src/lib.rs app/src-tauri/tests/reward_source.rs
git commit -m "feat: prefer gated memory reward recognition"
```

### Task 6: Experimental Diagnostics and Live Capture Harness

**Files:**
- Modify: `crates/warframe-acquisition/examples/scan_live_strings.rs`
- Modify: `crates/app-core/src/lib.rs`
- Modify: `app/src/App.tsx`
- Test: `crates/warframe-acquisition/tests/reward_memory.rs`
- Test: `crates/app-core/tests/vertical_slice.rs`

- [ ] **Step 1: Convert the throwaway scanner into bounded diagnostics**

Make the example accept candidate names and projection paths, reuse `RewardMemoryScanner`, and print only source, scan duration, bytes read, relative offsets, cluster status, and confirmation status. Do not print arbitrary memory contents, absolute process paths, authorization data, or full dumps.

- [ ] **Step 2: Expose source and performance health**

Extend capture health with messages such as `Memory reward observer ready (184 ms)` and `Memory ambiguous; OCR fallback active`. Preserve the existing secret-free serialization contract.

- [ ] **Step 3: Add fixture and view tests**

Assert diagnostic serialization contains source/timing but no addresses or candidate paths, and scanner fixtures remain within byte/time budgets.

- [ ] **Step 4: Run live validation commands**

Run the bounded example during at least two additional online reward screens and one solo fissure. Record expected visible choices, event timestamps, elapsed milliseconds, bytes read, selected relative cluster, and OCR agreement in `docs/research/memory-reward-live-validation.md`.

- [ ] **Step 5: Run complete verification**

Run:

```bash
cargo fmt --all --check
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
pnpm --dir app check
```

Expected: all commands pass; live validation documents three online captures total and one solo negative control before default enablement.

- [ ] **Step 6: Commit**

```bash
git add crates/warframe-acquisition/examples/scan_live_strings.rs crates/app-core/src/lib.rs crates/app-core/tests/vertical_slice.rs app/src/App.tsx docs/research/memory-reward-live-validation.md
git commit -m "test: validate memory reward recognition live"
```
