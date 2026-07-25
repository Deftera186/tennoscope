# Reward Card Memory Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Read Warframe's active reward-card slots from memory and publish exact left-to-right choices without OCR.

**Architecture:** Extend the existing fingerprint module to collect one-based card-slot evidence while it scans candidate paths. Resolve complete slot maps before temporal heuristics, using EE.log's local reward as Item1.

**Tech Stack:** Rust, Aho-Corasick, existing `MemoryReader` fixtures, Tauri reward-source adapter.

---

### Task 1: Capture card-slot evidence

**Files:**
- Modify: `crates/warframe-acquisition/src/reward_memory.rs`
- Modify: `crates/warframe-acquisition/tests/reward_memory.rs`

- [ ] Add a failing fixture with Item2–Item4 tags followed by candidate internal paths.
- [ ] Run `cargo test -p warframe-acquisition --test reward_memory` and verify the new test fails.
- [ ] Collect unambiguous one-based card slots inside `RewardFingerprint` during the existing scan.
- [ ] Re-run the focused tests and verify they pass.

### Task 2: Resolve anchored ordered cards

**Files:**
- Modify: `crates/warframe-acquisition/src/reward_memory.rs`
- Modify: `crates/warframe-acquisition/tests/reward_memory.rs`

- [ ] Add failing tests for ordered four-card and three-card results, missing slots, duplicate names, and conflicting slot values.
- [ ] Add `resolve_reward_choices_with_anchor` and prefer complete card-slot evidence before temporal clusters.
- [ ] Run the reward-memory tests and verify all cases pass.

### Task 3: Pass the local log anchor through the source seam

**Files:**
- Modify: `app/src-tauri/src/reward_source.rs`
- Modify: `app/src-tauri/src/lib.rs`
- Modify: `app/src-tauri/tests/reward_source.rs`

- [ ] Add a failing coordinator test that passes a local reward anchor to memory.
- [ ] Extend the memory-source interface with the optional anchor and resolve the local path before binding the scanner.
- [ ] Remove post-resolution local-name reordering because the memory module now owns ordering.
- [ ] Run reward-source, reward-log, and observer tests.

### Task 4: Verify and clean diagnostics

**Files:**
- Modify: `app/src-tauri/src/lib.rs`
- Review: `scripts/capture_reward_*.py`

- [ ] Run `cargo fmt --all --check`, targeted tests, and `cargo check --manifest-path app/src-tauri/Cargo.toml --tests`.
- [ ] Remove temporary source logging and stop all capture watchers.
- [ ] Preserve capture scripts as untracked diagnostics until live validation succeeds.
- [ ] Commit the production code and tests.
