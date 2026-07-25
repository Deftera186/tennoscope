# Persistent Reward UI Memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Discover and read Warframe's persistent ordered relic-reward UI model without fixed addresses or transient response races.

**Architecture:** A deep resolver builds a bounded reverse-pointer graph from candidate reward strings, identifies a unique shared ordered container, and confirms it across repeated reads. The Tauri monitor invokes this resolver during the reward window; transient responder records cannot publish.

**Tech Stack:** Rust, existing `MemoryReader` seam, Linux `/proc` adapter, deterministic memory fixtures, archived bounded captures.

---

### Task 1: Pointer Graph Primitives

**Files:**
- Create: `crates/warframe-acquisition/src/reward_ui_memory.rs`
- Modify: `crates/warframe-acquisition/src/lib.rs`
- Create: `crates/warframe-acquisition/tests/reward_ui_memory.rs`

- [ ] Write failing tests for aligned 64-bit reverse references, decoy 32-bit values, duplicate targets, and bounded graph depth.
- [ ] Run `cargo test -p warframe-acquisition --test reward_ui_memory` and verify the missing resolver fails compilation.
- [ ] Implement candidate seed discovery and bounded reverse-reference indexing behind `PersistentRewardResolver::resolve`.
- [ ] Run the focused tests and commit the graph primitives.

### Task 2: Stable Ordered Container Resolution

**Files:**
- Modify: `crates/warframe-acquisition/src/reward_ui_memory.rs`
- Modify: `crates/warframe-acquisition/tests/reward_ui_memory.rs`

- [ ] Write failing fixtures where four ordered child objects reach reward seeds through different intermediate layouts.
- [ ] Add decoy, duplicate-reward, competing-container, and unstable-second-read fixtures.
- [ ] Implement container ranking that requires the expected card count, preserves duplicate slots, rejects competing graphs, and confirms identical order twice.
- [ ] Run the focused tests and commit stable container resolution.

### Task 3: Runtime Integration

**Files:**
- Modify: `app/src-tauri/src/reward_source.rs`
- Modify: `app/src-tauri/src/lib.rs`
- Modify: `app/src-tauri/tests/reward_source.rs`

- [ ] Write failing coordinator tests proving only confirmed persistent UI results can publish.
- [ ] Replace transient squad publication with the persistent resolver during `ChoicesReady`.
- [ ] Keep responder scans diagnostic-only and reject proximity/allocation-order results.
- [ ] Run reward source and log tests, then commit the integration.

### Task 4: Archived Capture Replay and Verification

**Files:**
- Create: `scripts/analyze_reward_ui_graph.py`
- Modify: `docs/research/memory-reward-live-validation.md`

- [ ] Decode archived contexts using full 64-bit pointers and report candidate graph neighborhoods without assuming addresses.
- [ ] Add replay fixtures for every structurally complete archived neighborhood.
- [ ] Run `cargo fmt --all --check`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Document supported evidence and any remaining live acceptance requirement, then commit.

