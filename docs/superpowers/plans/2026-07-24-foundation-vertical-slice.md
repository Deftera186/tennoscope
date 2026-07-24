# Foundation Vertical Slice Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a runnable Linux Tauri application whose collection browser, local persistence, diagnostics, and reward advisor work end-to-end with deterministic fake game observations.

**Architecture:** A Rust workspace contains deep domain, storage, and application modules. The Tauri shell exposes a small command interface returning immutable views to a React/TypeScript frontend. Fake observations enter through the same application interface future log, memory, and screen adapters will use.

**Tech Stack:** Rust, Tauri 2, React, TypeScript, Vite, pnpm, rusqlite, serde, thiserror, Vitest, Testing Library, plain CSS design tokens.

---

## Prerequisite

The current workspace has an empty read-only `.git` directory. Before execution, provide a normal writable Git repository at `the repository root`; otherwise every commit step must be skipped and recorded. Do not delete or replace `.git` without explicit user authorization.

## File Map

- `Cargo.toml` — workspace members and shared package metadata.
- `rust-toolchain.toml` — reproducible Rust channel and components.
- `LICENSE` — GPLv3 license text.
- `crates/warframe-domain/src/lib.rs` — public domain exports.
- `crates/warframe-domain/src/catalog.rs` — stable item identities and categories.
- `crates/warframe-domain/src/inventory.rs` — coherent snapshots and collection views.
- `crates/warframe-domain/src/rewards.rs` — reward enrichment and advisor ranking.
- `crates/warframe-domain/tests/domain_contract.rs` — public interface contract tests.
- `crates/local-store/src/lib.rs` — SQLite-backed store interface and implementation.
- `crates/local-store/src/schema.sql` — initial schema.
- `crates/local-store/tests/sqlite_store.rs` — persistence and replacement tests.
- `crates/app-core/src/lib.rs` — application orchestration and immutable views.
- `crates/app-core/src/fake_session.rs` — deterministic development observations.
- `crates/app-core/tests/vertical_slice.rs` — fake-session integration test.
- `app/` — Vite React frontend.
- `app/src/api.ts` — typed Tauri command client.
- `app/src/model.ts` — frontend view types matching serialized Rust views.
- `app/src/App.tsx` — collection application shell.
- `app/src/features/collection/CollectionBrowser.tsx` — collection grid and filters.
- `app/src/features/diagnostics/HealthPanel.tsx` — backend health presentation.
- `app/src/features/overlay/RewardAdvisor.tsx` — four-card decision advisor.
- `app/src/styles.css` — accessible visual tokens and responsive layout.
- `app/src/**/*.test.tsx` — frontend behavior tests.
- `app/src-tauri/src/lib.rs` — Tauri state, commands, and overlay window lifecycle.
- `app/src-tauri/src/main.rs` — desktop entry point only.
- `app/src-tauri/tauri.conf.json` — collection and overlay window configuration.
- `README.md` — development commands, current fake-session scope, and risk statement.

### Task 1: Scaffold the workspace and test runners

**Files:**
- Create: `Cargo.toml`
- Create: `rust-toolchain.toml`
- Create: `LICENSE`
- Create: `crates/warframe-domain/Cargo.toml`
- Create: `crates/warframe-domain/src/lib.rs`
- Create: `crates/local-store/Cargo.toml`
- Create: `crates/local-store/src/lib.rs`
- Create: `crates/app-core/Cargo.toml`
- Create: `crates/app-core/src/lib.rs`
- Create: `app/` using the official Vite React TypeScript template

- [ ] **Step 1: Scaffold the frontend and Tauri shell**

Run:

```bash
pnpm create vite@latest app --template react-ts
cd app
pnpm install
pnpm add @tauri-apps/api
pnpm add -D @tauri-apps/cli vitest jsdom @testing-library/react @testing-library/jest-dom @testing-library/user-event
pnpm tauri init
```

Answer the Tauri prompts exactly:

```text
App name: Warframe Helper
Window title: Warframe Helper
Web assets: ../dist
Dev server: http://localhost:5173
Frontend dev command: pnpm dev
Frontend build command: pnpm build
```

Expected: `app/src-tauri/tauri.conf.json` exists and `pnpm tauri dev` reaches compilation. If system WebKitGTK packages are absent, stop after the compiler reports those explicit prerequisites; do not change the architecture.

- [ ] **Step 2: Add the Rust workspace manifest**

Create `Cargo.toml`:

```toml
[workspace]
resolver = "2"
members = [
  "app/src-tauri",
  "crates/app-core",
  "crates/local-store",
  "crates/warframe-domain",
]

[workspace.package]
edition = "2024"
license = "GPL-3.0-only"
rust-version = "1.85"

[workspace.dependencies]
serde = { version = "1", features = ["derive"] }
thiserror = "2"
```

Create `rust-toolchain.toml`:

```toml
[toolchain]
channel = "stable"
components = ["clippy", "rustfmt"]
profile = "minimal"
```

Expected: `cargo metadata --no-deps` lists four workspace members.

- [ ] **Step 3: Create minimal crate manifests**

Create `crates/warframe-domain/Cargo.toml`:

```toml
[package]
name = "warframe-domain"
version = "0.1.0"
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
serde.workspace = true
thiserror.workspace = true
```

Create `crates/local-store/Cargo.toml`:

```toml
[package]
name = "local-store"
version = "0.1.0"
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
rusqlite = { version = "0.37", features = ["bundled"] }
warframe-domain = { path = "../warframe-domain" }
```

Create `crates/app-core/Cargo.toml`:

```toml
[package]
name = "app-core"
version = "0.1.0"
edition.workspace = true
license.workspace = true
rust-version.workspace = true

[dependencies]
serde.workspace = true
thiserror.workspace = true
local-store = { path = "../local-store" }
warframe-domain = { path = "../warframe-domain" }
```

Set each `src/lib.rs` to:

```rust
#![forbid(unsafe_code)]
```

- [ ] **Step 4: Configure frontend tests**

Add these scripts to `app/package.json`:

```json
"test": "vitest run",
"test:watch": "vitest",
"check": "tsc --noEmit && vitest run"
```

Update `app/vite.config.ts`:

```ts
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  test: {
    environment: "jsdom",
    setupFiles: "./src/test/setup.ts",
  },
});
```

Create `app/src/test/setup.ts`:

```ts
import "@testing-library/jest-dom/vitest";
```

- [ ] **Step 5: Verify the empty foundation**

Run:

```bash
cargo fmt --all --check
cargo test --workspace
cd app && pnpm check && pnpm build
```

Expected: all commands exit 0.

- [ ] **Step 6: Commit**

```bash
git add Cargo.toml rust-toolchain.toml LICENSE crates app
git commit -m "chore: scaffold Rust and Tauri workspace"
```

### Task 2: Define the domain interface

**Files:**
- Create: `crates/warframe-domain/src/catalog.rs`
- Create: `crates/warframe-domain/src/inventory.rs`
- Create: `crates/warframe-domain/src/rewards.rs`
- Modify: `crates/warframe-domain/src/lib.rs`
- Test: `crates/warframe-domain/tests/domain_contract.rs`

- [ ] **Step 1: Write the failing collection contract test**

Create `crates/warframe-domain/tests/domain_contract.rs`:

```rust
use warframe_domain::{
    CatalogItem, Category, Collection, InventoryEntry, InventorySnapshot, ItemId,
};

#[test]
fn coherent_snapshot_replaces_quantities_and_deletions() {
    let paris = CatalogItem::new(ItemId::new("paris-prime-string").unwrap(), "Paris Prime String", Category::PrimePart);
    let lex = CatalogItem::new(ItemId::new("lex-prime-receiver").unwrap(), "Lex Prime Receiver", Category::PrimePart);
    let mut collection = Collection::default();

    collection.replace(InventorySnapshot::coherent(vec![
        InventoryEntry::new(paris.clone(), 2),
        InventoryEntry::new(lex.clone(), 1),
    ]));
    collection.replace(InventorySnapshot::coherent(vec![InventoryEntry::new(paris, 1)]));

    assert_eq!(collection.quantity(&ItemId::new("paris-prime-string").unwrap()), 1);
    assert_eq!(collection.quantity(&ItemId::new("lex-prime-receiver").unwrap()), 0);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p warframe-domain --test domain_contract`

Expected: FAIL because the exported domain types do not exist.

- [ ] **Step 3: Implement catalog and inventory types**

Create `catalog.rs` with `ItemId`, `Category`, and `CatalogItem`; create `inventory.rs` with `InventoryEntry`, the privately constructed coherent `InventorySnapshot`, and `Collection::replace`, `quantity`, and `entries`. Derive `Clone`, `Debug`, `Eq`, `PartialEq`, `Serialize`, and `Deserialize` on serialized types. Reject empty IDs and negative quantities at constructors by using unsigned quantities and `Result` for IDs.

The public shape must be:

```rust
pub struct ItemId(String);
pub enum Category { Frame, Weapon, Companion, PrimePart, Relic }
pub struct CatalogItem { pub id: ItemId, pub name: String, pub category: Category }
pub struct InventoryEntry { pub item: CatalogItem, pub quantity: u32, pub mastered: bool }
pub struct InventorySnapshot { entries: Vec<InventoryEntry> }
pub struct Collection { entries: std::collections::BTreeMap<ItemId, InventoryEntry> }
```

Expose constructors matching the test plus `InventorySnapshot::entries(&self) -> &[InventoryEntry]` for persistence. `ItemId::new` returns an error for empty or whitespace-only IDs. `Collection::replace` must clear the map before inserting the coherent snapshot so absent entries become legitimate deletions.

- [ ] **Step 4: Add the failing reward ranking test**

Append:

```rust
use warframe_domain::{RewardCandidate, RewardAdvisor};

#[test]
fn advisor_excludes_uncertain_rewards_from_best_value() {
    let uncertain = RewardCandidate::new("Forma Blueprint", 20, 25, 0, false, 0.40);
    let certain = RewardCandidate::new("Lex Prime Receiver", 8, 15, 0, true, 0.99);
    let view = RewardAdvisor::advise(vec![uncertain, certain]);

    assert_eq!(view.best_value_name(), Some("Lex Prime Receiver"));
    assert!(view.cards[1].mastery_relevant);
}
```

- [ ] **Step 5: Implement deterministic reward advice**

Create `rewards.rs` with:

```rust
pub struct RewardCandidate {
    pub name: String,
    pub platinum: u32,
    pub ducats: u32,
    pub owned: u32,
    pub mastery_relevant: bool,
    pub confidence: f32,
}

pub struct RewardView {
    pub cards: Vec<RewardCandidate>,
    best_value_index: Option<usize>,
}

pub struct RewardAdvisor;
```

`RewardAdvisor::advise` considers confidence `>= 0.80`, chooses maximum platinum value, resolves ties by ducats then input order, and returns no recommendation if every candidate is uncertain. Add `best_value_name()`.

- [ ] **Step 6: Export and verify the interface**

Update `lib.rs`:

```rust
#![forbid(unsafe_code)]

mod catalog;
mod inventory;
mod rewards;

pub use catalog::{CatalogItem, Category, ItemId};
pub use inventory::{Collection, InventoryEntry, InventorySnapshot};
pub use rewards::{RewardAdvisor, RewardCandidate, RewardView};
```

Run: `cargo test -p warframe-domain`

Expected: all domain tests PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/warframe-domain
git commit -m "feat: define collection and reward domain"
```

### Task 3: Persist authoritative collection snapshots

**Files:**
- Create: `crates/local-store/src/schema.sql`
- Modify: `crates/local-store/src/lib.rs`
- Test: `crates/local-store/tests/sqlite_store.rs`

- [ ] **Step 1: Write the failing SQLite replacement test**

Create `tests/sqlite_store.rs` that opens `SqliteStore::in_memory()`, commits a two-item coherent snapshot, commits a one-item snapshot, reloads the collection, and asserts the absent item quantity is zero. Also assert `audit_count() == 2`.

Use the same Paris and Lex fixtures from Task 2 and this interface:

```rust
let store = SqliteStore::in_memory()?;
store.replace_collection(&first, &SnapshotMeta::fake("build-a"))?;
store.replace_collection(&second, &SnapshotMeta::fake("build-a"))?;
let loaded = store.load_collection()?;
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p local-store --test sqlite_store`

Expected: FAIL because `SqliteStore` and `SnapshotMeta` do not exist.

- [ ] **Step 3: Add the schema**

Create `schema.sql`:

```sql
PRAGMA foreign_keys = ON;
CREATE TABLE IF NOT EXISTS inventory (
  item_id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  category TEXT NOT NULL,
  quantity INTEGER NOT NULL CHECK(quantity >= 0),
  mastered INTEGER NOT NULL CHECK(mastered IN (0, 1))
);
CREATE TABLE IF NOT EXISTS snapshot_audit (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  observed_at TEXT NOT NULL,
  game_build TEXT NOT NULL,
  source TEXT NOT NULL,
  item_count INTEGER NOT NULL CHECK(item_count >= 0)
);
```

- [ ] **Step 4: Implement atomic replacement**

Implement `SqliteStore` around `rusqlite::Connection`. `replace_collection` starts one transaction, deletes current inventory, inserts every snapshot entry, appends one audit row, and commits. `load_collection` converts rows back to domain entries. `in_memory` applies `schema.sql`. Map every database error into a public `StoreError` using `thiserror`.

Do not expose the connection or SQL through the public interface.

- [ ] **Step 5: Add rollback coverage**

Add a test-only `replace_collection_with_hook` internal helper that can return an error after deletion but before insertion. Verify a failed transaction retains the original collection and does not increment `audit_count`.

- [ ] **Step 6: Run tests and lint**

Run:

```bash
cargo test -p local-store
cargo clippy -p local-store --all-targets -- -D warnings
```

Expected: PASS and no warnings.

- [ ] **Step 7: Commit**

```bash
git add crates/local-store
git commit -m "feat: persist atomic collection snapshots"
```

### Task 4: Build the application module and fake session

**Files:**
- Modify: `crates/app-core/src/lib.rs`
- Create: `crates/app-core/src/fake_session.rs`
- Test: `crates/app-core/tests/vertical_slice.rs`

- [ ] **Step 1: Write a failing vertical-slice test**

Create a test that builds `AppCore::in_memory()`, calls `load_fake_session()`, and asserts:

```rust
assert_eq!(view.collection.total_entries, 5);
assert_eq!(view.health.game_reader.state, HealthState::Ready);
assert_eq!(view.reward.cards.len(), 4);
assert_eq!(view.reward.best_value_name(), Some("Forma Blueprint"));
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app-core --test vertical_slice`

Expected: FAIL because `AppCore` and its views do not exist.

- [ ] **Step 3: Implement immutable application views**

Define serializable `AppView`, `CollectionView`, `CollectionItemView`, `HealthView`, `BackendHealth`, and `HealthState`. `AppCore` owns `SqliteStore` and current `RewardView`. Its public interface is exactly:

```rust
pub fn in_memory() -> Result<Self, AppError>;
pub fn open(path: &std::path::Path) -> Result<Self, AppError>;
pub fn current_view(&self) -> Result<AppView, AppError>;
pub fn apply_inventory_snapshot(&mut self, snapshot: InventorySnapshot, meta: SnapshotMeta) -> Result<AppView, AppError>;
pub fn apply_reward_candidates(&mut self, rewards: Vec<RewardCandidate>) -> Result<AppView, AppError>;
pub fn load_fake_session(&mut self) -> Result<AppView, AppError>;
```

- [ ] **Step 4: Implement deterministic fake observations**

`fake_session.rs` returns five catalog entries spanning PrimePart, Relic, Frame, and Weapon, and four reward candidates. Include Forma Blueprint at `12p/25d`, Lex Prime Receiver marked mastery-relevant, and one low-confidence candidate to exercise uncertainty.

- [ ] **Step 5: Verify orchestration**

Run:

```bash
cargo test -p app-core
cargo clippy -p app-core --all-targets -- -D warnings
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/app-core
git commit -m "feat: add fake-session application core"
```

### Task 5: Expose the application through Tauri commands

**Files:**
- Modify: `app/src-tauri/Cargo.toml`
- Modify: `app/src-tauri/src/lib.rs`
- Modify: `app/src-tauri/src/main.rs`
- Modify: `app/src-tauri/tauri.conf.json`
- Test: unit tests inside `app/src-tauri/src/lib.rs`

- [ ] **Step 1: Add a failing command-state test**

Test a plain Rust helper `current_view(&ManagedApp)` without starting a webview. Initialize managed state, call `load_fake_session`, then assert the returned serialized view has five entries.

- [ ] **Step 2: Add workspace dependencies**

Add `app-core`, `serde`, and `thiserror` to `app/src-tauri/Cargo.toml`. Keep Tauri-generated dependencies and set the package license to `GPL-3.0-only`.

- [ ] **Step 3: Implement managed state and commands**

Create:

```rust
pub struct ManagedApp(std::sync::Mutex<app_core::AppCore>);

#[tauri::command]
fn get_app_view(state: tauri::State<'_, ManagedApp>) -> Result<app_core::AppView, String>;

#[tauri::command]
fn load_fake_session(state: tauri::State<'_, ManagedApp>) -> Result<app_core::AppView, String>;

#[tauri::command]
fn set_reward_overlay_visible(app: tauri::AppHandle, visible: bool) -> Result<(), String>;
```

Map poisoned mutexes and `AppError` into stable user-facing strings. Register all three commands with `tauri::generate_handler!`. During builder setup, resolve `app.path().app_data_dir()`, create it, and manage `AppCore::open(&app_data_dir.join("warframe-helper.sqlite3"))`; reserve `in_memory()` for tests. `set_reward_overlay_visible` gets the `reward-overlay` webview window and calls `show` when true or `hide` when false, returning a stable error if the configured window is missing.

- [ ] **Step 4: Configure two windows**

In `tauri.conf.json`, define:

- `main`: title `Warframe Helper`, `1200x760`, resizable, visible;
- `reward-overlay`: route `/overlay`, transparent, decorations disabled, always-on-top, skip-taskbar, initially hidden, `1100x190`.

Do not implement click-through platform behavior in this phase; the overlay is a visual development window until Phase 4.

- [ ] **Step 5: Verify Rust commands**

Run: `cargo test -p warframe-helper`

Expected: command helper tests PASS without launching a display server.

- [ ] **Step 6: Commit**

```bash
git add app/src-tauri
git commit -m "feat: expose application views through Tauri"
```

### Task 6: Build the collection browser

**Files:**
- Create: `app/src/model.ts`
- Create: `app/src/api.ts`
- Modify: `app/src/App.tsx`
- Create: `app/src/features/collection/CollectionBrowser.tsx`
- Create: `app/src/features/collection/CollectionBrowser.test.tsx`
- Create: `app/src/features/diagnostics/HealthPanel.tsx`
- Modify: `app/src/styles.css`

- [ ] **Step 1: Write the failing UI behavior test**

Render `CollectionBrowser` with fixtures containing a frame, weapon, prime part, and relic. Type `lex` into search and assert only `Lex Prime Receiver` remains. Select the `Relics` category and assert only relics remain. Verify quantity and mastery text are accessible by role/text.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cd app && pnpm vitest run src/features/collection/CollectionBrowser.test.tsx`

Expected: FAIL because the module does not exist.

- [ ] **Step 3: Define frontend view types and command client**

Mirror Rust serialization in `model.ts` using string unions for categories and health states. In `api.ts`, export:

```ts
export const getAppView = () => invoke<AppView>("get_app_view");
export const loadFakeSession = () => invoke<AppView>("load_fake_session");
```

- [ ] **Step 4: Implement collection filtering**

`CollectionBrowser` owns only presentation state: search text and selected category. It receives `CollectionView` as props, filters case-insensitively by name, and renders semantic buttons and a responsive card grid. It must not calculate mastery, prices, or set progress.

- [ ] **Step 5: Implement the application shell**

`App.tsx` loads `getAppView` on mount, offers a development-only `Load fake session` action, and renders navigation for Overview, Collection, Relics, and Activity. The Collection route is functional; other routes display explicit phase labels rather than dead controls.

- [ ] **Step 6: Apply modern visual tokens**

In `styles.css`, define dark neutral surfaces, gold value accents, blue mastery accents, green healthy states, 8px spacing increments, 10px card radii, visible keyboard focus, reduced-motion handling, and layouts that remain usable from 900px to ultrawide widths. Use system fonts; add no unlicensed game assets.

- [ ] **Step 7: Verify frontend behavior**

Run:

```bash
cd app
pnpm test
pnpm build
```

Expected: tests PASS and Vite builds without TypeScript errors.

- [ ] **Step 8: Commit**

```bash
git add app/src app/package.json app/pnpm-lock.yaml
git commit -m "feat: add collection browser interface"
```

### Task 7: Build the reward advisor and diagnostics views

**Files:**
- Create: `app/src/features/overlay/RewardAdvisor.tsx`
- Create: `app/src/features/overlay/RewardAdvisor.test.tsx`
- Modify: `app/src/features/diagnostics/HealthPanel.tsx`
- Create: `app/src/features/diagnostics/HealthPanel.test.tsx`
- Modify: `app/src/main.tsx`
- Modify: `app/src/styles.css`

- [ ] **Step 1: Write failing advisor tests**

Render four reward cards and assert:

- all names, platinum values, ducats, and owned quantities render;
- the best-value card has the accessible label `Best value`;
- a mastery candidate has `Mastery needed`;
- an uncertain card has `Uncertain recognition` and is not marked best value.

- [ ] **Step 2: Implement the advisor**

`RewardAdvisor` receives the immutable reward view and renders exactly four cards in source order. It uses domain-provided flags and never re-ranks rewards in TypeScript.

- [ ] **Step 3: Write and implement health-panel tests**

Test Ready, Degraded, and Failed states with a visible message and last-success timestamp. Implement one row each for game reader, capture, catalog, market, and database.

- [ ] **Step 4: Route the overlay window**

In `main.tsx`, render `RewardAdvisor` when `window.location.pathname === "/overlay"`; otherwise render `App`. Load the same fake application view through `get_app_view` so both windows share Rust-owned state.

- [ ] **Step 5: Verify frontend and desktop integration**

Run:

```bash
cd app && pnpm check
cargo test --workspace
```

Then run `cd app && pnpm tauri dev`, load the fake session, and invoke `set_reward_overlay_visible(true)` from a `Preview reward overlay` button in the Diagnostics view. Expected: the collection window and four-card overlay match the approved design hierarchy. Keep this button until live reward detection replaces it in Phase 4.

- [ ] **Step 6: Commit**

```bash
git add app/src app/src-tauri/tauri.conf.json
git commit -m "feat: add reward advisor and health views"
```

### Task 8: Document and verify the foundation release

**Files:**
- Create: `README.md`
- Create: `.gitignore`
- Modify: `docs/superpowers/plans/2026-07-24-warframe-helper-roadmap.md`

- [ ] **Step 1: Add repository hygiene**

Ignore `target/`, `app/node_modules/`, `app/dist/`, `.superpowers/`, local SQLite databases, editor files, and generated package artifacts. Do not ignore lockfiles.

- [ ] **Step 2: Document development and scope**

README sections must include Purpose, Current Phase, Risk Disclosure, Prerequisites, Development, Tests, Architecture, Packaging Status, Privacy, and GPLv3 License. State clearly that Phase 1 uses fake observations and performs no game-process access.

- [ ] **Step 3: Run the complete verification suite**

Run:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd app && pnpm check && pnpm build
```

Expected: every command exits 0.

- [ ] **Step 4: Perform the manual smoke test**

Run: `cd app && pnpm tauri dev`

Verify:

1. the application opens without Warframe;
2. fake-session loading produces five collection entries;
3. search and category filters work;
4. diagnostics show deterministic fake backend states;
5. the reward route renders four cards with best-value, mastery, and uncertainty labels; and
6. restarting the application reloads the persisted fake collection from SQLite.

- [ ] **Step 5: Mark roadmap phase complete**

Change roadmap Phase 1 to `Complete` only after all automated and manual checks pass. Add the exact successful toolchain versions from `rustc --version`, `cargo --version`, `node --version`, and `pnpm --version`.

- [ ] **Step 6: Commit**

```bash
git add .gitignore README.md docs/superpowers/plans
git commit -m "docs: complete foundation vertical slice"
```
