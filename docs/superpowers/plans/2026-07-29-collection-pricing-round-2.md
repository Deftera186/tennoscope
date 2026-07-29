# Collection Pricing Round 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Correct the price arithmetic, stop pricing things the player cannot sell, and replace the card affordances that shipped wrong.

**Architecture:** Six independent corrections on top of `feat/collection-platinum-pricing`. Three are data-correctness (per-trade division, relics sourced from a bounded live sweep instead of the daily dump, quantity-0 items unpriced); three are interface (the per-item Check button deleted, the LIVE badge replaced by dated provenance, the page-refresh control naming its own scope).

**Tech Stack:** Rust (`warframe-acquisition`, `app-core`), Tauri 2, React 19 + TypeScript, Vitest.

**Design documents:** `docs/design/collection-platinum-pricing.md` (amend as directed per task).

## Global Constraints

- No new dependencies.
- Every crate keeps `#![forbid(unsafe_code)]`.
- warframe.market's documented public limit is 3 requests per second, enforced globally by `MarketPriceCache`'s request floor (`MARKET_MIN_GAP`, 334ms). Nothing may bypass it.
- `platinum` on a warframe.market order is the price for a whole trade of `perTrade` units. The per-unit price is `platinum / perTrade`.
- A live price is valid for 15 minutes in `MarketPriceCache`. Anything needing to outlive that must be persisted in the collection price table.
- Rust tests: `cargo test --workspace`. Frontend: `pnpm -C app check`.
- Interface tasks (5, 6) must invoke the `impeccable` skill before writing interface code.
- Commit after every task with a Conventional Commits message.

## Measured Evidence

Gathered 2026-07-29 against the live API and the player's real 1,106-item collection.

| Measurement | Value |
| --- | --- |
| Relic sell orders with `perTrade > 1` | 5%–29% per relic, almost always `perTrade: 6` |
| Daily median inflation from bulk listings | 1.00x–1.50x (Axi A1: 25p raw vs 16.67p per-unit) |
| Distinct relic market names owned | 65, all at quantity ≥ 1 |
| Startup relic sweep cost | ~22s at 3 requests/second |
| Quantity-0 items currently priced | includes mastered-but-unowned equipment |

---

### Task 1: Per-unit prices from per-trade listings

**Files:**
- Modify: `crates/warframe-acquisition/src/market.rs` (`Order`, `lowest_sell_top`)
- Modify: `crates/warframe-acquisition/tests/market.rs`
- Modify: `docs/design/collection-platinum-pricing.md`

**Interfaces:**
- Consumes: nothing new.
- Produces: no signature change. `lowest_sell_top` returns the cheapest *per-unit* price.

- [ ] **Step 1: Write the failing test**

Add to `crates/warframe-acquisition/tests/market.rs`:

```rust
/// `platinum` is the price for a whole trade of `perTrade` units, not for one unit. Relic sellers
/// routinely list six at a time, so comparing a six-pack's total against a single's price ranks
/// two different quantities as if they were the same thing.
#[test]
fn a_bulk_listing_is_quoted_per_unit_not_per_trade() {
    let body = r#"{"data":{"sell":[
        {"platinum":20,"perTrade":1,"visible":true,"user":{"status":"ingame"}},
        {"platinum":18,"perTrade":6,"visible":true,"user":{"status":"ingame"}}
    ],"buy":[]}}"#;
    assert_eq!(lowest_sell_top(body.as_bytes()), PriceLookup::Priced(3));
}

/// A listing with no `perTrade` field is a single, not a free item.
#[test]
fn a_listing_without_a_per_trade_count_is_one_unit() {
    let body = r#"{"data":{"sell":[
        {"platinum":25,"visible":true,"user":{"status":"ingame"}}
    ],"buy":[]}}"#;
    assert_eq!(lowest_sell_top(body.as_bytes()), PriceLookup::Priced(25));
}

/// Integer division would report a 5-for-12 listing at 2p and understate every bulk seller.
#[test]
fn a_per_unit_price_rounds_rather_than_truncating() {
    let body = r#"{"data":{"sell":[
        {"platinum":12,"perTrade":5,"visible":true,"user":{"status":"ingame"}}
    ],"buy":[]}}"#;
    assert_eq!(lowest_sell_top(body.as_bytes()), PriceLookup::Priced(2));
}

/// A malformed `perTrade` of zero must not divide by zero or price the item at nothing.
#[test]
fn a_zero_per_trade_count_is_treated_as_one() {
    let body = r#"{"data":{"sell":[
        {"platinum":30,"perTrade":0,"visible":true,"user":{"status":"ingame"}}
    ],"buy":[]}}"#;
    assert_eq!(lowest_sell_top(body.as_bytes()), PriceLookup::Priced(30));
}
```

Note the third test's expectation: `12 / 5 = 2.4`, which rounds to `2`. Choose the rounding that
makes that assertion true and state it in the code comment.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p warframe-acquisition --test market`
Expected: FAIL — the bulk listing currently prices at 18, not 3.

- [ ] **Step 3: Implement**

In `crates/warframe-acquisition/src/market.rs`, add the field to `Order`:

```rust
    /// How many units one trade moves. `platinum` is the price of that whole trade, so a seller
    /// listing six relics for 18p is asking 3p each, not 18p each.
    #[serde(default = "one", rename = "perTrade")]
    per_trade: u32,
```

with `fn one() -> u32 { 1 }`, and change the price computation in `lowest_sell_top` to divide by the
trade size, rounding to nearest:

```rust
        .map(|order| {
            // `platinum` buys `perTrade` units, so the per-unit price is the quotient. Rounded to
            // nearest rather than truncated: 12p for five is 2.4p each, and truncation would quote
            // 2p and understate every bulk seller. A malformed count of zero is treated as one
            // rather than dividing by it.
            let per_trade = order.per_trade.max(1);
            (order.platinum + per_trade / 2) / per_trade
        })
```

Check the arithmetic against the tests: `12 + 5/2 = 14`, `14 / 5 = 2`; `18 + 6/2 = 21`, `21 / 6 = 3`;
`25 + 0 = 25`, `25 / 1 = 25`. All integer arithmetic, no floats, no overflow at realistic prices.

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: PASS. Existing tests use `perTrade: 1` or omit it, so they must be unaffected — if any
existing expectation changed, stop and report rather than editing the expectation.

- [ ] **Step 5: Amend the design**

In `docs/design/collection-platinum-pricing.md`, add to the Evidence table that 5–29% of relic sell
orders are bulk (`perTrade: 6` typically), and record in the live-price section that a quoted price
is per unit, derived by dividing the trade total.

- [ ] **Step 6: Commit**

```bash
git add crates/warframe-acquisition/src/market.rs crates/warframe-acquisition/tests/market.rs docs/design/collection-platinum-pricing.md
git commit -m "fix: quote warframe.market prices per unit rather than per trade"
```

---

### Task 2: Relics leave the daily dump

**Files:**
- Modify: `crates/warframe-acquisition/src/collection_prices.rs`
- Modify: `crates/warframe-acquisition/tests/collection_prices.rs`
- Modify: `docs/design/collection-platinum-pricing.md`

**Interfaces:**
- Produces: `PriceTable::is_relic_name(name: &str) -> bool` (or equivalent), `PriceTable::insert_live(market_name: &str, platinum: u32)`, and `PriceTable::relic_market_names(&self) -> Vec<String>` — exact names your choice, but they must be what Task 3 consumes.

The daily dump is pre-aggregated and carries no `perTrade`, so a relic's median there is inflated by
bulk listings — measured up to 1.5x. Relics therefore stop being priced from the dump and are priced
from a live sweep instead (Task 3), whose results are persisted in the same table so they outlive the
15-minute live cache.

- [ ] **Step 1: Write the failing tests**

Add to `crates/warframe-acquisition/tests/collection_prices.rs`:

```rust
/// The dump cannot be corrected for bulk listings, so a relic's daily median runs high — measured
/// at 1.5x on Axi A1. Relics are priced from a live sweep instead, and until that lands they have
/// no price rather than an inflated one.
#[test]
fn a_relic_is_not_priced_from_the_dump() {
    let table = table();
    assert_eq!(table.price_for("Axi A1 Radiant"), None);
    assert_eq!(table.price_for("Axi A1 Relic"), None);
}

/// Resolution still works, because the live sweep needs the market name to build its slug.
#[test]
fn a_relic_still_resolves_to_its_market_name() {
    assert_eq!(table().market_name("Axi A1 Radiant"), Some("Axi A1 Relic"));
}

#[test]
fn a_swept_relic_price_is_served_like_any_other() {
    let mut table = table();
    table.insert_live("Axi A1 Relic", 17);
    assert_eq!(table.price_for("Axi A1 Radiant"), Some(17));
    assert_eq!(table.price_for("Axi A1 Relic"), Some(17));
}

#[test]
fn the_relics_needing_a_sweep_are_the_ones_the_dump_lists() {
    let names = table().relic_market_names();
    assert_eq!(names, vec!["Axi A1 Relic".to_owned()]);
}

/// A swept price survives the cache round-trip, or every restart would cost another sweep.
#[test]
fn swept_relic_prices_survive_the_disk_cache() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cache = CollectionPriceCache::new(directory.path());
    let mut table = cache
        .refresh(&FakeDumps::new(&[("2026-07-27", DUMP)]), TODAY)
        .expect("refresh stores");
    table.insert_live("Axi A1 Relic", 17);
    cache.store_table(&table).expect("store");

    let reloaded = cache.load_cached().expect("a stored table is readable");
    assert_eq!(reloaded.price_for("Axi A1 Radiant"), Some(17));
}

#[test]
fn a_non_relic_is_still_priced_from_the_dump() {
    assert_eq!(table().price_for("Serration"), Some(50));
}
```

`store_table` is a new public method on `CollectionPriceCache` exposing what `store` already does
privately; add it.

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p warframe-acquisition --test collection_prices`
Expected: FAIL — relics are currently priced from the dump.

- [ ] **Step 3: Implement**

In `PriceTable`:
- Keep every dump key in the resolution map so `market_name` is unchanged, but exclude relic keys from the *price* map. A dump key ending in ` Relic` is a relic.
- Add a second map for swept relic prices, serialized with the rest so it round-trips through the cache.
- `price_for` consults the swept map as well as the dump map.
- `relic_market_names` returns the relic keys the dump listed, for the sweep to work through.
- Document why relics are excluded, with the measured inflation figure, so nobody restores them.

- [ ] **Step 4: Run the tests**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 5: Amend the design**

Rewrite the relic paragraph in `docs/design/collection-platinum-pricing.md`'s Name Resolution
section: rule 3 still resolves the name, but the price no longer comes from the dump. State the
measured inflation and that relics are swept live instead. The existing note about refinement tiers
sharing one price stays true and stays.

- [ ] **Step 6: Commit**

```bash
git add crates/warframe-acquisition/src/collection_prices.rs crates/warframe-acquisition/tests/collection_prices.rs docs/design/collection-platinum-pricing.md
git commit -m "fix: stop pricing relics from the bulk-inflated daily dump"
```

---

### Task 3: The startup relic sweep

**Files:**
- Modify: `crates/app-core/src/lib.rs`
- Modify: `crates/app-core/tests/collection_pricing.rs`
- Modify: `app/src-tauri/src/lib.rs` (`start_collection_prices`)

**Interfaces:**
- Consumes: `PriceTable::relic_market_names`, `PriceTable::insert_live`, `CollectionPriceCache::store_table` from Task 2; `MarketPriceCache::warm` and its global request floor.
- Produces: `AppCore::owned_relic_market_names(&self) -> Result<Vec<String>, AppError>`.

- [ ] **Step 1: Write the failing test**

Add to `crates/app-core/tests/collection_pricing.rs`:

```rust
/// The sweep is bounded by what the player owns, not by what exists. Measured against a real
/// collection that is 65 relics and about 22 seconds; the dump lists 772.
#[test]
fn only_owned_relics_are_swept() {
    let dump = r#"{
        "Axi A1 Relic": [{"order_type":"sell","median":20.0,"volume":30}],
        "Meso B2 Relic": [{"order_type":"sell","median":9.0,"volume":12}],
        "Serration": [{"order_type":"sell","median":50.0,"volume":12}]
    }"#;
    let mut core = core_with_items(vec![
        item("/a", "Axi A1 Radiant", Category::Relic, 2),
        item("/b", "Meso B2 Intact", Category::Relic, 0),
        item("/c", "Serration", Category::Resource, 1),
    ]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    assert_eq!(
        core.owned_relic_market_names().expect("resolves"),
        vec!["Axi A1 Relic".to_owned()],
        "a relic at quantity 0 is not owned, and a resource is not a relic"
    );
}

/// Four refinement tiers of one relic are one request.
#[test]
fn relic_refinements_collapse_before_the_sweep() {
    let dump = r#"{"Axi A1 Relic": [{"order_type":"sell","median":20.0,"volume":30}]}"#;
    let mut core = core_with_items(vec![
        item("/a", "Axi A1 Intact", Category::Relic, 1),
        item("/b", "Axi A1 Radiant", Category::Relic, 3),
    ]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    assert_eq!(core.owned_relic_market_names().unwrap().len(), 1);
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app-core --test collection_pricing`
Expected: FAIL — no method `owned_relic_market_names`.

- [ ] **Step 3: Implement `owned_relic_market_names`**

Resolve every collection entry with `quantity >= 1` through `PriceTable::market_name`, keep the ones
that are relic names, sort and deduplicate. Reuse the shape of the existing `market_names_for`.

- [ ] **Step 4: Wire the sweep**

In `app/src-tauri/src/lib.rs`'s `start_collection_prices`, after the dump has loaded and been applied:
run the sweep on the same background thread, through `MarketPriceCache::warm` so it obeys the global
request floor, then write each result into the table with `insert_live`, persist with
`store_table`, and re-publish via `set_collection_prices`.

Requirements:
- Do not hold the runtime mutex across the sweep.
- Skip relics that already carry a swept price no older than the dump's own refresh cadence — a restart within the day must not re-sweep. Put that decision next to `dump_is_current` and test it.
- Report progress through the collection-pricing health row so a 22-second sweep is visible in Diagnostics rather than silent.
- The sweep is bounded by `owned_relic_market_names`; it must never fall back to sweeping every relic the dump lists.

- [ ] **Step 5: Run everything**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/app-core/src/lib.rs crates/app-core/tests/collection_pricing.rs app/src-tauri/src/lib.rs
git commit -m "feat: price owned relics with a bounded live sweep at startup"
```

---

### Task 4: Nothing unowned carries a price

**Files:**
- Modify: `crates/app-core/src/lib.rs` (`current_view`)
- Modify: `crates/app-core/tests/collection_pricing.rs`

An item at quantity 0 is mastered, or known, but not held. Pricing it puts platinum against something
the player cannot sell and inflates the collection worth with inventory that does not exist.

- [ ] **Step 1: Write the failing test**

```rust
/// Mastery is not ownership. An item at quantity 0 is not in the inventory and must not carry a
/// price, appear under the tradeable filter, or contribute to the collection's worth.
#[test]
fn an_item_the_player_does_not_own_is_not_priced() {
    let mut core = core_with_items(vec![
        item("/a", "Serration", Category::Resource, 0).with_mastered(true),
        item("/b", "Mirage Prime Systems", Category::PrimePart, 2),
    ]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(DUMP.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    let view = core.current_view().expect("view builds");
    let items = view.collection().items();
    assert_eq!(items[0].platinum(), None, "quantity 0 is not owned");
    assert_eq!(items[1].platinum(), Some(20));
}
```

`InventoryEntry::with_mastered` already exists; check its exact form and match it.

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p app-core --test collection_pricing`
Expected: FAIL — the unowned item is priced at 50.

- [ ] **Step 3: Implement**

In `current_view`, skip both the live and dump lookups when `entry.quantity == 0`, leaving
`platinum: None` and `live: false`. Comment why.

- [ ] **Step 4: Run everything**

Run: `cargo test --workspace` and `pnpm -C app check`
Expected: PASS. If a frontend fixture assumed an unowned item carried a price, fix the fixture.

- [ ] **Step 5: Commit**

```bash
git add crates/app-core/src/lib.rs crates/app-core/tests/collection_pricing.rs
git commit -m "fix: price only items the player actually owns"
```

---

### Task 5: Delete the per-item Check button

**Files:**
- Modify: `app/src/App.tsx` (`CollectionEntry`, `CollectionPage`, `App`)
- Modify: `app/src/App.css`
- Modify: `app/src/App.test.tsx`

**Interfaces:**
- The Tauri `refresh_prices` command and `refreshPrices` binding stay — the page-level control in Task 6 is their only caller.

- [ ] **Step 1: Invoke the impeccable skill**

Read `app/src/App.css` and `App.tsx` first. This is a deletion, so the skill's job here is making
sure nothing is left dangling: the card's corner geometry, the `.marks` row's balance, and the
`pricing` prop chain.

- [ ] **Step 2: Remove the test that asserts the button exists**

Delete the `'prices one item live when it is selected'` test from `app/src/App.test.tsx`. It asserts
behaviour that is being removed by decision, not by accident — note that in the commit message.

- [ ] **Step 3: Implement**

- Delete the `.price-check` button from `CollectionEntry` and its `onPriceLive` prop.
- Delete `.price-check` from `App.css`, and any rule that existed only to make room for it.
- `CollectionPage` keeps `onPriceLive` — Task 6's page control uses it — but stops passing it to `CollectionEntry`.
- `CollectionEntry` no longer needs `pricing` unless something else uses it; remove it if not.

- [ ] **Step 4: Run**

Run: `pnpm -C app check`
Expected: PASS, no unused-variable lint.

- [ ] **Step 5: Commit**

```bash
git add app/src/App.tsx app/src/App.css app/src/App.test.tsx
git commit -m "refactor: remove the per-item live price control"
```

---

### Task 6: Dated provenance, and a refresh control that names its scope

**Files:**
- Modify: `app/src/App.tsx`, `app/src/App.css`, `app/src/App.test.tsx`
- Modify: `app/src/backend.ts` and `crates/app-core/src/lib.rs` if the dump date is not already reaching the frontend
- Modify: `CHANGELOG.md`

Two problems, one vocabulary. The `LIVE` badge shouted a word with nothing on screen to explain it.
The refresh control did not say what it would refresh. Both are answered by stating provenance
plainly: the register says where prices came from, and a card that was checked live says so itself.

- [ ] **Step 1: Invoke the impeccable skill**

The approved direction, to be realised in the established assay-register language:

```
REGISTER BAR
  All · Owned · Mastered · Missing · Tradeable
  Prices from the 27 Jul market summary     [ Price these 31 ]   1–48 of 312

CARD (daily)              CARD (checked live)
┌──────────────────┐      ┌──────────────────┐
│ RELIC            │      │ PRIME PART       │
│ Axi A1 Radiant   │      │ Mirage Prime     │
│ Owned ×7         │      │ Systems          │
│ 20p · 140p total │      │ Owned ×3         │
└──────────────────┘      │ 19p · 57p total  │
                          │ checked just now │
                          └──────────────────┘
```

Follow the skill for the freshness line's weight and the control's states. It must have default,
hover, focus, disabled and in-progress states, and the in-progress state must show real progress —
a page of 48 takes about 16 seconds and an unchanged button reads as broken.

- [ ] **Step 2: Write the failing tests**

```tsx
// The badge said "LIVE" with nothing on screen to explain it. A date explains itself.
it('states where the daily prices came from', async () => {
  render(<App/>)
  expect(await screen.findByText(/27 Jul/)).toBeInTheDocument()
})

it('marks a card checked live with its freshness, not a badge', async () => {
  render(<App/>)
  const live = await screen.findByRole('article', { name: 'Lith A1 Relic' })
  const daily = await screen.findByRole('article', { name: 'Lex Prime Receiver' })

  expect(within(live).getByText(/checked/i)).toBeInTheDocument()
  expect(within(live).queryByText('Live')).not.toBeInTheDocument()
  expect(within(daily).queryByText(/checked/i)).not.toBeInTheDocument()
})

// Someone who clicks it should not have to guess whether it prices the page or the collection.
it('names how many items the refresh will price', async () => {
  render(<App/>)
  expect(await screen.findByRole('button', { name: /Price these 2/ })).toBeInTheDocument()
})
```

The count is the visible items that carry a price, since those are exactly the ones with a known
market name; verify that against the fixture and use the number it actually produces.

- [ ] **Step 3: Run to verify they fail**

Run: `pnpm -C app test`
Expected: FAIL.

- [ ] **Step 4: Implement**

- Remove the `Live` badge from `CollectionEntry`; replace with a freshness line rendered only when `item.live`.
- Publish the dump date to the frontend if it is not already there, and render it in the register bar. Format it as a short human date, not a raw ISO string or a unix integer.
- Rename and relocate the refresh control next to the range readout, labelled with the count it will price, and give it a progress-bearing in-flight state.
- Keep `aria-live` or equivalent on the progress so it is not sighted-only.

- [ ] **Step 5: Run everything**

Run: `pnpm -C app check` and `cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Record the change**

Add to `CHANGELOG.md` under Unreleased, describing the corrections in the user's terms: prices quoted
per unit rather than per trade, relics priced live because the daily summary overstates them, only
owned items priced, and the card affordances replaced.

- [ ] **Step 7: Commit**

```bash
git add app/src/App.tsx app/src/App.css app/src/App.test.tsx app/src/backend.ts crates/app-core/src/lib.rs CHANGELOG.md
git commit -m "feat: date collection prices and name the refresh control's scope"
```

---

### Task 7: Sort by unit price

**Files:**
- Modify: `app/src/App.tsx`
- Modify: `app/src/App.test.tsx`

Sorting by stack value ranks a pile of cheap relics above a single expensive part, which answers
"where is my platinum" rather than "what is worth the most". The latter is the question the sort is
for; the stack total stays on the card for the former.

- [ ] **Step 1: Write the failing test**

Replace the existing value-sort test's expectations. With the current fixture — `lith-a1` at 20p ×7
(140 total) and `lex-prime-receiver` at 19p ×1 — a unit-price sort puts `lith-a1` first at 20p and
`lex-prime-receiver` second at 19p, which does not distinguish the two orders. Add a fixture item
priced above 20p at quantity 1 so the two orderings differ, and assert the unit ordering:

```tsx
// Sorting by stack value answers "where is my platinum"; sorting by unit price answers "what is
// worth the most". The sort is for the second question, and the card still shows the first.
it('sorts by unit price, not by what the stack is worth', async () => {
  const user = userEvent.setup()
  render(<App/>)
  await user.click(await screen.findByRole('button', { name: 'Value' }))

  const names = screen.getAllByRole('article').map(a => a.getAttribute('aria-label'))
  expect(names[0]).toBe('Ash Prime Blueprint')  // 45p × 1
  expect(names[1]).toBe('Lith A1 Relic')        // 20p × 7 = 140 total, but 20p each
})
```

Use whatever fixture item and price make the two orderings provably different, and keep the existing
assertion that unpriced items sink to the bottom.

- [ ] **Step 2: Run to verify it fails**

Run: `pnpm -C app test`
Expected: FAIL — stack sort puts `Lith A1 Relic` first.

- [ ] **Step 3: Implement**

Change the `value-desc` comparator to use `item.platinum` rather than `stackValue(item)`, keeping
unpriced items last and the name tie-break. `stackValue` stays — the card and the worth cell use it.

- [ ] **Step 4: Run**

Run: `pnpm -C app check`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add app/src/App.tsx app/src/App.test.tsx
git commit -m "fix: sort collection value by unit price rather than stack total"
```
