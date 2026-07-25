# Player Reward Records Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Resolve ordered relic rewards from transient player reward records before the 15-second selection timer is materially consumed.

**Architecture:** Extend the log state machine to expose ordered responder events and the earliest complete-set event. Add a focused memory resolver whose interface accepts responder order, local identity/reward, and candidates, while hiding scanning and ambiguity rejection.

**Tech Stack:** Rust, `/proc/<pid>/mem`, Aho-Corasick, Cargo tests.

---

### Task 1: Expose the reward response lifecycle

**Files:**
- Modify: `app/src-tauri/src/reward_log.rs`
- Test: `app/src-tauri/tests/reward_log.rs`

- [ ] Add a failing replay test for the July 25 four-response sequence, asserting ordered responder events and an immediate complete-set event.
- [ ] Run `cargo test -p app --test reward_log` and confirm the new assertion fails.
- [ ] Add the minimal event variants and state required to pass the replay.
- [ ] Re-run the focused test and commit.

### Task 2: Resolve player reward records without stale-cluster fallback

**Files:**
- Modify: `crates/warframe-acquisition/src/reward_memory.rs`
- Modify: `crates/warframe-acquisition/src/lib.rs`
- Test: `crates/warframe-acquisition/tests/reward_memory.rs`

- [ ] Add a failing fixture with responder IDs beside four real reward identities and a tighter unrelated stale reward cluster.
- [ ] Run the focused test and confirm it fails because the record resolver does not exist.
- [ ] Implement one bounded scan and strict per-responder ambiguity rejection.
- [ ] Run all acquisition memory tests and commit.

### Task 3: Integrate early memory resolution

**Files:**
- Modify: `app/src-tauri/src/lib.rs`
- Modify: `app/src-tauri/src/reward_source.rs`
- Test: `app/src-tauri/tests/reward_source.rs`

- [ ] Add a failing coordinator test proving a complete player-record result bypasses OCR and preserves order.
- [ ] Integrate responder collection and resolve at the complete-set event, retaining current OCR fallback at the timer event.
- [ ] Verify focused app tests, then run `cargo test --workspace` and `pnpm --dir app test`.
- [ ] Confirm the live debug build restarts without publishing any cached reward set.
