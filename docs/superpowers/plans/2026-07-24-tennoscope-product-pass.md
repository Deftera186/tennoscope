# TennoScope Product Pass Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a distinctive TennoScope MVP with canonical item artwork, paginated smooth collection browsing, explicit sync freshness, and an automatic Linux relic reward overlay.

**Architecture:** Extend WFCD normalization and immutable application views with image and snapshot metadata, while keeping UI-only pagination/freshness logic in small tested TypeScript modules. Put capture, recognition, debounce, and native overlay placement behind focused Rust modules so inventory acquisition remains independent.

**Tech Stack:** Rust, Tauri 2, React 19, TypeScript, Vitest/Testing Library, WFCD catalog, Tesseract CLI, grim/X11 capture, GTK layer-shell.

---

### Task 1: Product identity and migration-safe paths

**Files:**
- Modify: `Cargo.toml`
- Modify: `app/package.json`
- Modify: `app/index.html`
- Modify: `app/src-tauri/Cargo.toml`
- Modify: `app/src-tauri/tauri.conf.json`
- Modify: `app/src-tauri/src/lib.rs`
- Modify: `README.md`
- Modify: `packaging/arch/PKGBUILD`
- Modify: `packaging/gentoo/warframe-helper-0.1.0.ebuild`
- Test: `app/src-tauri/tests/setup_contract.rs`

- [ ] Write a failing migration test that seeds the legacy app-data filenames and expects TennoScope initialization helpers to reuse them.
- [ ] Run `cargo test -p warframe-helper --test setup_contract` and confirm the new assertion fails.
- [ ] Rename user-facing/package identity to TennoScope, keep the stable Tauri identifier, and add explicit legacy database/setup path selection.
- [ ] Run the focused Rust test and package metadata assertions.
- [ ] Commit with `feat: establish TennoScope identity`.

### Task 2: Canonical artwork through the data pipeline

**Files:**
- Modify: `crates/warframe-acquisition/src/catalog.rs`
- Modify: `crates/warframe-acquisition/tests/catalog_index.rs`
- Modify: `crates/warframe-domain/src/catalog.rs`
- Modify: `crates/app-core/src/lib.rs`
- Modify: `crates/app-core/tests/vertical_slice.rs`
- Modify: `app/src/backend.ts`

- [ ] Write failing catalog and serialization tests expecting `image_name` for parent items and nested Prime components.
- [ ] Run the focused acquisition/core tests and confirm field/assertion failures.
- [ ] Add validated image identity to catalog metadata and domain items, propagate it to `CollectionItemView`, and expose a CDN URL only in the presentation contract.
- [ ] Re-run focused tests and commit with `feat: expose canonical item artwork`.

### Task 3: Pagination, freshness, and collection interaction

**Files:**
- Create: `app/src/collection.ts`
- Create: `app/src/collection.test.ts`
- Create: `app/src/freshness.ts`
- Create: `app/src/freshness.test.ts`
- Modify: `app/src/App.tsx`
- Modify: `app/src/App.test.tsx`
- Modify: `app/src/App.css`
- Modify: `app/src/index.css`

- [ ] Write failing unit tests for 48-item pages, bounded page labels, page clamping/reset, relative/exact freshness labels, and image fallback.
- [ ] Run the focused Vitest files and verify expected failures.
- [ ] Implement the pure helpers, render only the active page, reset pagination on query/filter/sort changes, and add accessible controls.
- [ ] Replace fake letter art with canonical images plus category fallbacks and expose exact snapshot details via tooltip/focus popover.
- [ ] Remove nested scroll behavior and revise the visual system into the TennoScope field-console language.
- [ ] Run `pnpm check` and commit with `feat: build visual paginated collection`.

### Task 4: Snapshot metadata contract

**Files:**
- Modify: `crates/local-store/src/lib.rs`
- Modify: `crates/app-core/src/lib.rs`
- Modify: `crates/app-core/tests/vertical_slice.rs`
- Modify: `app/src/backend.ts`

- [ ] Write a failing core serialization test expecting `collection.snapshot.observed_at`, `source`, and `game_build`.
- [ ] Run the focused test and confirm it fails on the absent object.
- [ ] Publish persisted snapshot metadata in `CollectionView`, preserving `null` when no coherent snapshot exists.
- [ ] Run focused and workspace tests and commit with `feat: expose snapshot freshness metadata`.

### Task 5: Reward recognition state machine

**Files:**
- Create: `app/src-tauri/src/reward_observer.rs`
- Create: `app/src-tauri/tests/reward_observer.rs`
- Modify: `app/src-tauri/src/lib.rs`
- Modify: `app/src-tauri/Cargo.toml`

- [ ] Write failing tests for OCR normalization, fuzzy catalog resolution, four-choice validation, consecutive-hit show debounce, and consecutive-miss hide debounce.
- [ ] Run `cargo test -p warframe-helper --test reward_observer` and verify the missing module/API failures.
- [ ] Implement pure recognizer/state-machine types first, then a bounded command-based frame source for `grim` and Tesseract with timeouts and scrubbed diagnostics.
- [ ] Connect observer lifecycle to Warframe process monitoring without coupling failures to inventory refresh.
- [ ] Run focused tests and commit with `feat: detect relic reward choices`.

### Task 6: Real native overlay behavior

**Files:**
- Create: `app/src-tauri/src/overlay_window.rs`
- Create: `app/src-tauri/tests/overlay_geometry.rs`
- Modify: `app/src-tauri/src/lib.rs`
- Modify: `app/src-tauri/tauri.conf.json`
- Modify: `app/src/RewardOverlay.tsx`
- Modify: `app/src/RewardOverlay.test.tsx`
- Modify: `app/src/RewardCards.tsx`
- Modify: `app/src/App.css`

- [ ] Write failing geometry tests for 16:9, ultrawide, and scaled displays plus UI tests that prohibit title/close chrome and require four aligned choices.
- [ ] Run focused Rust and frontend tests and verify failures.
- [ ] Implement reward-relative overlay geometry, non-focusable show/hide behavior, click-through where supported, and wlroots layer placement through the native Linux adapter.
- [ ] Redesign overlay markup/CSS as translucent aligned decision labels over the game rather than a framed mini-window.
- [ ] Run focused tests and commit with `feat: ship in-game reward overlay`.

### Task 7: Live verification and release artifacts

**Files:**
- Modify: `README.md`
- Modify: `packaging/README.md`
- Modify: `THIRD_PARTY_NOTICES.md`

- [ ] Run `cargo fmt --all -- --check`, `cargo test --workspace`, and `cargo clippy --workspace --all-targets -- -D warnings`.
- [ ] Run `pnpm --dir app check` and `pnpm --dir app build`.
- [ ] Launch the release build with Warframe under the current Wayland/wlroots session; verify collection images, pagination, timestamp, overlay placement, focus behavior, and automatic hide.
- [ ] Build AppImage, deb, and rpm artifacts and inspect names, desktop entry, executable, license, and notices.
- [ ] Update runbook with capture dependencies and honest compositor support; commit with `docs: document TennoScope MVP`.
