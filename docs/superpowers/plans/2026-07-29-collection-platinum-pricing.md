# Collection Platinum Pricing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Show a platinum price and stack total on every priceable item in the collection, sourced from one daily warframe.market price dump.

**Architecture:** A single daily download from `relics.run/history` is parsed into a name-to-platinum table, cached on disk, and joined onto collection items inside `AppCore::current_view()`. No worker thread, no queue, no rate limiter — every price arrives in one file. The reward overlay keeps its separate live per-item path, which this plan only touches to make its failure outcomes distinguishable.

**Tech Stack:** Rust (workspace crates `warframe-acquisition`, `app-core`), Tauri 2, React 19 + TypeScript, Vitest, `reqwest` blocking, `serde`, `atomicwrites`.

**Design document:** `docs/design/collection-platinum-pricing.md` — read it before starting. It records why the `/v2/items` manifest was rejected and why the collection and overlay use different numbers.

## Global Constraints

- No new dependencies. Everything needed (`reqwest`, `serde`, `serde_json`, `atomicwrites`, `thiserror`) is already in `crates/warframe-acquisition/Cargo.toml`.
- Every crate keeps `#![forbid(unsafe_code)]`.
- `User-Agent` on every outbound request: `TennoScope/{CARGO_PKG_VERSION} (+https://github.com/Deftera186/tennoscope)`.
- Price source: `https://relics.run/history/price_history_<YYYY-MM-DD>.json`, walking back at most 5 days from today.
- Price field: the `sell` record's `median`, rounded to the nearest whole platinum.
- Response size cap: 32 MB (`MAX_DUMP_BYTES`). Measured file is 3.9 MB.
- Rust tests: `cargo test -p <crate>`. Frontend: `pnpm -C app test`. Full check: `pnpm -C app check`.
- Frontend tasks (7 and 8) must invoke the `impeccable` skill before writing interface code.
- Commit after every task with a Conventional Commits message.

---

### Task 1: Price table and name resolution

**Files:**
- Create: `crates/warframe-acquisition/src/collection_prices.rs`
- Create: `crates/warframe-acquisition/tests/collection_prices.rs`
- Modify: `crates/warframe-acquisition/src/lib.rs:8-18` (add `mod collection_prices;`), `:32-34` (add exports)

**Interfaces:**
- Consumes: nothing.
- Produces: `PriceTable::from_dump_json(bytes: &[u8], dump_date: &str) -> Result<PriceTable, PriceDumpError>`, `PriceTable::price_for(&self, name: &str) -> Option<u32>`, `PriceTable::dump_date(&self) -> &str`, `PriceTable::len(&self) -> usize`, `enum PriceDumpError { Malformed }`. `PriceTable` derives `Clone, Debug, Default, Serialize, Deserialize`.

- [ ] **Step 1: Write the failing tests**

Create `crates/warframe-acquisition/tests/collection_prices.rs`:

```rust
use warframe_acquisition::{PriceTable, PriceDumpError};

/// A trimmed copy of the real dump's shape: keyed by English name, one record per order type.
const DUMP: &str = r#"{
    "Mirage Prime Systems Blueprint": [
        {"order_type":"closed","median":18.0,"volume":4},
        {"order_type":"sell","median":20.0,"min_price":10,"volume":1127},
        {"order_type":"buy","median":12.0,"volume":172}
    ],
    "Axi A1 Relic": [
        {"order_type":"sell","median":20.0,"volume":30}
    ],
    "Serration": [
        {"order_type":"sell","median":50.0,"volume":12}
    ],
    "Zephyr Prime Chassis Blueprint": [
        {"order_type":"sell","median":27.5,"volume":9}
    ],
    "Bottomless Pit": [
        {"order_type":"buy","median":3.0,"volume":2}
    ]
}"#;

fn table() -> PriceTable {
    PriceTable::from_dump_json(DUMP.as_bytes(), "2026-07-27").expect("fixture parses")
}

#[test]
fn a_sell_median_becomes_the_price() {
    assert_eq!(table().price_for("Serration"), Some(50));
}

/// The dump names the blueprint; the catalog names the part. Neither is wrong, so both resolve.
#[test]
fn a_part_resolves_to_its_blueprint_listing() {
    assert_eq!(table().price_for("Mirage Prime Systems"), Some(20));
}

#[test]
fn a_blueprint_resolves_to_a_listing_without_the_suffix() {
    let dump = r#"{"Forma": [{"order_type":"sell","median":8.0,"volume":3}]}"#;
    let table = PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").unwrap();
    assert_eq!(table.price_for("Forma Blueprint"), Some(8));
}

/// The catalog names a relic by refinement, the market by relic. All four tiers share one price,
/// which understates a radiant relic and is the accepted cost of pricing relics at all.
#[test]
fn every_relic_refinement_resolves_to_the_one_relic_listing() {
    let table = table();
    for name in [
        "Axi A1 Intact",
        "Axi A1 Exceptional",
        "Axi A1 Flawless",
        "Axi A1 Radiant",
    ] {
        assert_eq!(table.price_for(name), Some(20), "for {name}");
    }
}

#[test]
fn a_median_is_rounded_to_whole_platinum() {
    assert_eq!(table().price_for("Zephyr Prime Chassis"), Some(28));
}

/// An item nobody is selling has no sell record. It is unpriced, not free.
#[test]
fn an_item_with_no_sell_listing_has_no_price() {
    assert_eq!(table().price_for("Bottomless Pit"), None);
}

#[test]
fn an_unknown_name_has_no_price() {
    assert_eq!(table().price_for("Not An Item"), None);
}

#[test]
fn the_table_reports_what_it_parsed() {
    let table = table();
    assert_eq!(table.dump_date(), "2026-07-27");
    assert_eq!(table.len(), 4, "the buy-only item is not a price");
}

/// A truncated download must be rejected whole. Half a dump applied silently would halve the
/// reported worth of a collection with nothing to show that it had.
#[test]
fn a_malformed_dump_is_rejected_whole() {
    let truncated = &DUMP.as_bytes()[..DUMP.len() / 2];
    assert!(matches!(
        PriceTable::from_dump_json(truncated, "2026-07-27"),
        Err(PriceDumpError::Malformed)
    ));
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p warframe-acquisition --test collection_prices`
Expected: FAIL — `unresolved import warframe_acquisition::PriceTable`.

- [ ] **Step 3: Write the implementation**

Create `crates/warframe-acquisition/src/collection_prices.rs`:

```rust
//! Collection pricing from the daily warframe.market price dump.
//!
//! The overlay prices four cards live because a reward screen is a decision made in fifteen
//! seconds. A collection is a valuation of hundreds of items, which is a different question and
//! gets a different answer: one file a day, every price in it, no per-item requests at all.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum PriceDumpError {
    #[error("the price dump could not be read")]
    Malformed,
}

/// Refinement tiers the catalog appends to a relic's name. The market lists one price per relic
/// regardless of refinement, so all four resolve to the same listing.
const REFINEMENTS: [&str; 4] = [" Intact", " Exceptional", " Flawless", " Radiant"];

#[derive(Deserialize)]
struct DumpRecord {
    order_type: String,
    #[serde(default)]
    median: Option<f64>,
}

/// Every priceable item, keyed by the dump's own English name.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PriceTable {
    prices: HashMap<String, u32>,
    dump_date: String,
}

impl PriceTable {
    pub fn from_dump_json(bytes: &[u8], dump_date: &str) -> Result<Self, PriceDumpError> {
        let raw: HashMap<String, Vec<DumpRecord>> =
            serde_json::from_slice(bytes).map_err(|_| PriceDumpError::Malformed)?;
        let prices = raw
            .into_iter()
            .filter_map(|(name, records)| Some((name, sell_median(&records)?)))
            .collect();
        Ok(Self {
            prices,
            dump_date: dump_date.to_owned(),
        })
    }

    /// The catalog's name for an item and the market's are usually the same string, and where they
    /// are not the difference is one of three known shapes rather than a fuzzy match.
    pub fn price_for(&self, name: &str) -> Option<u32> {
        if let Some(price) = self.prices.get(name) {
            return Some(*price);
        }
        if let Some(price) = self.prices.get(&format!("{name} Blueprint")) {
            return Some(*price);
        }
        if let Some(base) = name.strip_suffix(" Blueprint")
            && let Some(price) = self.prices.get(base)
        {
            return Some(*price);
        }
        REFINEMENTS
            .iter()
            .find_map(|suffix| name.strip_suffix(suffix))
            .and_then(|base| self.prices.get(&format!("{base} Relic")))
            .copied()
    }

    pub fn dump_date(&self) -> &str {
        &self.dump_date
    }

    pub fn len(&self) -> usize {
        self.prices.len()
    }

    pub fn is_empty(&self) -> bool {
        self.prices.is_empty()
    }
}

/// The middle of the day's sell listings. `min_price` is the day's cheapest listing and counts
/// sellers who are offline, which reads 10p on an item an online seller wanted 19p for.
fn sell_median(records: &[DumpRecord]) -> Option<u32> {
    let median = records
        .iter()
        .find(|record| record.order_type == "sell")?
        .median?;
    (median.is_finite() && median >= 0.0).then(|| median.round() as u32)
}
```

- [ ] **Step 4: Export it**

In `crates/warframe-acquisition/src/lib.rs`, add `mod collection_prices;` to the module list (alphabetically, after `mod catalog_cache;`) and this to the exports:

```rust
pub use collection_prices::{PriceDumpError, PriceTable};
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p warframe-acquisition --test collection_prices`
Expected: PASS, 8 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/warframe-acquisition/src/collection_prices.rs crates/warframe-acquisition/src/lib.rs crates/warframe-acquisition/tests/collection_prices.rs
git commit -m "feat: resolve collection item names to warframe.market dump prices"
```

---

### Task 2: Fetching the newest dump

**Files:**
- Modify: `crates/warframe-acquisition/src/collection_prices.rs`
- Modify: `crates/warframe-acquisition/tests/collection_prices.rs`
- Modify: `crates/warframe-acquisition/src/lib.rs` (exports)

**Interfaces:**
- Consumes: `PriceTable::from_dump_json` from Task 1.
- Produces: `trait CollectionPriceSource { fn fetch(&self, date: &str) -> Result<Vec<u8>, PriceFetch>; }`, `enum PriceFetch { Missing, Unavailable, TooLarge }`, `fn civil_date(unix_seconds: u64) -> String`, `fn latest_dump(source: &dyn CollectionPriceSource, now_unix: u64) -> Result<PriceTable, PriceDumpError>`, `struct RelicsRunHttp`, constants `RELICS_RUN_HISTORY_URL`, `MAX_DUMP_BYTES`, `DUMP_LOOKBACK_DAYS`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/warframe-acquisition/tests/collection_prices.rs`:

```rust
use std::{cell::RefCell, collections::HashMap as Map};
use warframe_acquisition::{CollectionPriceSource, PriceFetch, civil_date, latest_dump};

struct FakeDumps {
    available: Map<String, String>,
    asked: RefCell<Vec<String>>,
}

impl FakeDumps {
    fn new(available: &[(&str, &str)]) -> Self {
        Self {
            available: available
                .iter()
                .map(|(date, body)| ((*date).to_owned(), (*body).to_owned()))
                .collect(),
            asked: RefCell::new(Vec::new()),
        }
    }
}

impl CollectionPriceSource for FakeDumps {
    fn fetch(&self, date: &str) -> Result<Vec<u8>, PriceFetch> {
        self.asked.borrow_mut().push(date.to_owned());
        self.available
            .get(date)
            .map(|body| body.as_bytes().to_vec())
            .ok_or(PriceFetch::Missing)
    }
}

/// 2026-07-29T00:00:00Z. The dumps lag: on that day the newest was dated the 27th.
const TODAY: u64 = 1_785_283_200;

#[test]
fn a_unix_time_becomes_the_dump_date_the_url_needs() {
    assert_eq!(civil_date(0), "1970-01-01");
    assert_eq!(civil_date(TODAY), "2026-07-29");
}

/// The dump for today usually does not exist yet, so asking only for today would price nothing.
#[test]
fn the_newest_available_dump_is_found_by_walking_back() {
    let source = FakeDumps::new(&[("2026-07-27", DUMP)]);

    let table = latest_dump(&source, TODAY).expect("an older dump is still a dump");

    assert_eq!(table.dump_date(), "2026-07-27");
    assert_eq!(
        source.asked.borrow().as_slice(),
        ["2026-07-29", "2026-07-28", "2026-07-27"],
        "each day is tried once, newest first"
    );
}

#[test]
fn the_newest_dump_wins_when_several_exist() {
    let source = FakeDumps::new(&[("2026-07-28", DUMP), ("2026-07-27", DUMP)]);

    assert_eq!(latest_dump(&source, TODAY).unwrap().dump_date(), "2026-07-28");
}

/// Walking back forever would hammer a dead host on every start, and a week-old valuation is not
/// worth the requests it would cost to find.
#[test]
fn the_walk_back_gives_up_rather_than_searching_forever() {
    let source = FakeDumps::new(&[]);

    assert!(latest_dump(&source, TODAY).is_err());
    assert_eq!(source.asked.borrow().len(), 6, "today plus five days back");
}

/// A dump that parses but is empty is a bad dump, not a collection where nothing is worth anything.
#[test]
fn a_dump_with_no_prices_is_not_accepted() {
    let source = FakeDumps::new(&[("2026-07-29", "{}")]);

    assert!(latest_dump(&source, TODAY).is_err());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p warframe-acquisition --test collection_prices`
Expected: FAIL — `unresolved import warframe_acquisition::latest_dump`.

- [ ] **Step 3: Write the implementation**

Append to `crates/warframe-acquisition/src/collection_prices.rs`:

```rust
use std::{io::Read, time::Duration};

use reqwest::blocking::Client;

pub const RELICS_RUN_HISTORY_URL: &str = "https://relics.run/history/";
/// The measured dump is 3.9 MB. The cap exists because the body is streamed into memory and an
/// uncapped read against a host we do not control is an out-of-memory waiting for a bad day.
pub const MAX_DUMP_BYTES: usize = 32 * 1024 * 1024;
/// How far back to look for a dump. Measured lag on 2026-07-29 was two days; five is slack for a
/// missed publication, and a bound so a dead host costs six requests rather than an unbounded walk.
pub const DUMP_LOOKBACK_DAYS: u64 = 5;
const USER_AGENT: &str = concat!(
    "TennoScope/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Deftera186/tennoscope)"
);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PriceFetch {
    /// No dump was published for that date. Expected, and the reason the walk-back exists.
    Missing,
    Unavailable,
    TooLarge,
}

pub trait CollectionPriceSource {
    fn fetch(&self, date: &str) -> Result<Vec<u8>, PriceFetch>;
}

/// The civil date at a unix time, as `YYYY-MM-DD`.
///
/// Hinnant's `civil_from_days`. Fifteen lines of arithmetic against a date crate the workspace
/// does not otherwise need, for the one place a date is formatted.
pub fn civil_date(unix_seconds: u64) -> String {
    let z = (unix_seconds / 86_400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!("{year:04}-{month:02}-{day:02}")
}

/// The newest dump on offer, starting at today and walking back.
pub fn latest_dump(
    source: &dyn CollectionPriceSource,
    now_unix: u64,
) -> Result<PriceTable, PriceDumpError> {
    for day in 0..=DUMP_LOOKBACK_DAYS {
        let date = civil_date(now_unix.saturating_sub(day * 86_400));
        let Ok(bytes) = source.fetch(&date) else {
            continue;
        };
        let Ok(table) = PriceTable::from_dump_json(&bytes, &date) else {
            continue;
        };
        // A dump that parses to nothing is a bad dump, not an account where nothing has value.
        if !table.is_empty() {
            return Ok(table);
        }
    }
    Err(PriceDumpError::Malformed)
}

pub struct RelicsRunHttp {
    client: Client,
}

impl RelicsRunHttp {
    pub fn new() -> Option<Self> {
        Client::builder()
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(60))
            .user_agent(USER_AGENT)
            .build()
            .ok()
            .map(|client| Self { client })
    }
}

impl CollectionPriceSource for RelicsRunHttp {
    fn fetch(&self, date: &str) -> Result<Vec<u8>, PriceFetch> {
        let url = format!("{RELICS_RUN_HISTORY_URL}price_history_{date}.json");
        let response = self
            .client
            .get(url)
            .send()
            .map_err(|_| PriceFetch::Unavailable)?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(PriceFetch::Missing);
        }
        let response = response.error_for_status().map_err(|_| PriceFetch::Unavailable)?;
        let mut body = Vec::new();
        response
            .take((MAX_DUMP_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|_| PriceFetch::Unavailable)?;
        if body.len() > MAX_DUMP_BYTES {
            return Err(PriceFetch::TooLarge);
        }
        Ok(body)
    }
}
```

- [ ] **Step 4: Export it**

Extend the export line in `crates/warframe-acquisition/src/lib.rs`:

```rust
pub use collection_prices::{
    CollectionPriceSource, DUMP_LOOKBACK_DAYS, MAX_DUMP_BYTES, PriceDumpError, PriceFetch,
    PriceTable, RELICS_RUN_HISTORY_URL, RelicsRunHttp, civil_date, latest_dump,
};
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p warframe-acquisition --test collection_prices`
Expected: PASS, 13 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/warframe-acquisition/src/collection_prices.rs crates/warframe-acquisition/src/lib.rs crates/warframe-acquisition/tests/collection_prices.rs
git commit -m "feat: fetch the newest published warframe.market price dump"
```

---

### Task 3: Caching prices on disk

**Files:**
- Modify: `crates/warframe-acquisition/src/collection_prices.rs`
- Modify: `crates/warframe-acquisition/tests/collection_prices.rs`
- Modify: `crates/warframe-acquisition/src/lib.rs` (exports)

**Interfaces:**
- Consumes: `PriceTable`, `latest_dump`, `CollectionPriceSource` from Tasks 1-2.
- Produces: `CollectionPriceCache::new(directory: impl Into<PathBuf>) -> Self`, `CollectionPriceCache::load_cached(&self) -> Option<PriceTable>`, `CollectionPriceCache::refresh(&self, source: &dyn CollectionPriceSource, now_unix: u64) -> Result<PriceTable, PriceDumpError>`.

- [ ] **Step 1: Write the failing tests**

Append to `crates/warframe-acquisition/tests/collection_prices.rs`:

```rust
use warframe_acquisition::CollectionPriceCache;

#[test]
fn a_refreshed_table_is_readable_without_the_network() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cache = CollectionPriceCache::new(directory.path());
    let source = FakeDumps::new(&[("2026-07-27", DUMP)]);

    cache.refresh(&source, TODAY).expect("refresh stores");
    let cached = cache.load_cached().expect("a stored table is readable");

    assert_eq!(cached.price_for("Serration"), Some(50));
    assert_eq!(cached.dump_date(), "2026-07-27");
}

#[test]
fn an_empty_cache_directory_yields_no_table() {
    let directory = tempfile::tempdir().expect("temp dir");

    assert!(CollectionPriceCache::new(directory.path()).load_cached().is_none());
}

/// A failed refresh must leave yesterday's prices alone. Discarding them because a download failed
/// would turn a network blip into a collection that reads as worthless.
#[test]
fn a_failed_refresh_leaves_the_cached_prices_in_place() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cache = CollectionPriceCache::new(directory.path());
    cache
        .refresh(&FakeDumps::new(&[("2026-07-27", DUMP)]), TODAY)
        .expect("first refresh stores");

    assert!(cache.refresh(&FakeDumps::new(&[]), TODAY).is_err());

    let cached = cache.load_cached().expect("the old table survives");
    assert_eq!(cached.dump_date(), "2026-07-27");
    assert_eq!(cached.price_for("Serration"), Some(50));
}

#[test]
fn a_corrupt_cache_file_yields_no_table_rather_than_a_panic() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(directory.path().join("collection-prices.json"), b"{not json")
        .expect("write corrupt cache");

    assert!(CollectionPriceCache::new(directory.path()).load_cached().is_none());
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p warframe-acquisition --test collection_prices`
Expected: FAIL — `unresolved import warframe_acquisition::CollectionPriceCache`.

- [ ] **Step 3: Write the implementation**

Append to `crates/warframe-acquisition/src/collection_prices.rs` (add `fs`, `io::Write`, `path::PathBuf` and `atomicwrites::{AtomicFile, OverwriteBehavior}` to the imports at the top of the file):

```rust
/// The parsed table on disk, so a start with no network still prices the collection.
///
/// What is stored is the resolved table rather than the download: the 3.9 MB dump reduces to a few
/// thousand name-and-price pairs, which loads instantly and costs nothing to keep.
pub struct CollectionPriceCache {
    directory: PathBuf,
}

impl CollectionPriceCache {
    pub fn new(directory: impl Into<PathBuf>) -> Self {
        Self {
            directory: directory.into(),
        }
    }

    fn path(&self) -> PathBuf {
        self.directory.join("collection-prices.json")
    }

    pub fn load_cached(&self) -> Option<PriceTable> {
        let bytes = fs::read(self.path()).ok()?;
        let table: PriceTable = serde_json::from_slice(&bytes).ok()?;
        (!table.is_empty()).then_some(table)
    }

    /// Fetch the newest dump and store it. On failure the previously stored table is untouched.
    pub fn refresh(
        &self,
        source: &dyn CollectionPriceSource,
        now_unix: u64,
    ) -> Result<PriceTable, PriceDumpError> {
        let table = latest_dump(source, now_unix)?;
        self.store(&table)?;
        Ok(table)
    }

    fn store(&self, table: &PriceTable) -> Result<(), PriceDumpError> {
        fs::create_dir_all(&self.directory).map_err(|_| PriceDumpError::Malformed)?;
        let bytes = serde_json::to_vec(table).map_err(|_| PriceDumpError::Malformed)?;
        AtomicFile::new(self.path(), OverwriteBehavior::AllowOverwrite)
            .write(|file| file.write_all(&bytes).and_then(|_| file.sync_all()))
            .map_err(|_| PriceDumpError::Malformed)
    }
}
```

- [ ] **Step 4: Export it**

Add `CollectionPriceCache` to the `collection_prices` export list in `crates/warframe-acquisition/src/lib.rs`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p warframe-acquisition --test collection_prices`
Expected: PASS, 17 tests.

- [ ] **Step 6: Commit**

```bash
git add crates/warframe-acquisition/src/collection_prices.rs crates/warframe-acquisition/src/lib.rs crates/warframe-acquisition/tests/collection_prices.rs
git commit -m "feat: cache collection prices so a start without network still prices"
```

---

### Task 4: Prices on the application view

**Files:**
- Modify: `crates/app-core/src/lib.rs:90-110` (`current_view`), `:388-440` (`CollectionItemView`), `:29-33` (`AppCore` fields)
- Create: `crates/app-core/tests/collection_pricing.rs`

**Interfaces:**
- Consumes: `PriceTable::price_for` from Task 1.
- Produces: `AppCore::set_collection_prices(&mut self, prices: Arc<PriceTable>)`, `CollectionItemView::platinum(&self) -> Option<u32>`, and the serialized field `platinum` on each collection item.

- [ ] **Step 1: Write the failing test**

Create `crates/app-core/tests/collection_pricing.rs`:

```rust
use std::sync::Arc;

use app_core::AppCore;
use local_store::SnapshotMeta;
use warframe_acquisition::PriceTable;
use warframe_domain::{CatalogItem, Category, InventoryEntry, InventorySnapshot};

const DUMP: &str = r#"{
    "Serration": [{"order_type":"sell","median":50.0,"volume":12}],
    "Mirage Prime Systems Blueprint": [{"order_type":"sell","median":20.0,"volume":9}]
}"#;

fn item(id: &str, name: &str, category: Category, quantity: u32) -> InventoryEntry {
    InventoryEntry::new(
        CatalogItem::new(id.to_owned(), name, category).expect("valid item"),
        quantity,
    )
}

fn core_with_items(entries: Vec<InventoryEntry>) -> AppCore {
    let mut core = AppCore::in_memory().expect("in-memory core");
    core.apply_inventory_snapshot(
        InventorySnapshot::coherent(entries).expect("coherent snapshot"),
        SnapshotMeta::fake("build").expect("meta"),
    )
    .expect("snapshot applies");
    core
}

#[test]
fn a_priced_item_carries_its_platinum_into_the_view() {
    let mut core = core_with_items(vec![
        item("/a", "Serration", Category::Resource, 1),
        item("/b", "Mirage Prime Systems", Category::PrimePart, 3),
    ]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(DUMP.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    let view = core.current_view().expect("view builds");
    let prices: Vec<_> = view
        .collection()
        .items()
        .iter()
        .map(|item| (item.name().to_owned(), item.platinum()))
        .collect();

    assert_eq!(
        prices,
        vec![
            ("Serration".to_owned(), Some(50)),
            ("Mirage Prime Systems".to_owned(), Some(20)),
        ]
    );
}

/// Before the dump loads, and for anything the dump does not list, the view says nothing rather
/// than zero. Zero would read as "worthless" for an item that is merely unpriced.
#[test]
fn an_item_the_dump_does_not_list_has_no_price() {
    let mut core = core_with_items(vec![item("/a", "Bottomless Pit", Category::Resource, 1)]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(DUMP.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    assert_eq!(core.current_view().unwrap().collection().items()[0].platinum(), None);
}

#[test]
fn a_view_built_before_any_prices_load_is_unpriced_rather_than_broken() {
    let core = core_with_items(vec![item("/a", "Serration", Category::Resource, 1)]);

    assert_eq!(core.current_view().unwrap().collection().items()[0].platinum(), None);
}
```

Note: `current_view()` sorts items by ID, so `/a` precedes `/b`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p app-core --test collection_pricing`
Expected: FAIL — no method `set_collection_prices`.

- [ ] **Step 3: Write the implementation**

In `crates/app-core/src/lib.rs`:

Add to the imports:

```rust
use std::sync::Arc;
use warframe_acquisition::PriceTable;
```

Add the field to `AppCore` (after `health`):

```rust
    prices: Option<Arc<PriceTable>>,
```

Set it to `None` in `from_store`'s struct literal, and add the setter next to the other recorders:

```rust
    /// The daily price table, once it has loaded. Held rather than passed in on every call
    /// because the view is rebuilt every 2.5 seconds and the table changes once a day.
    pub fn set_collection_prices(&mut self, prices: Arc<PriceTable>) {
        self.prices = Some(prices);
    }
```

Change `current_view`'s item construction from `.map(CollectionItemView::from)` to:

```rust
        let mut items = collection
            .entries()
            .map(|entry| {
                let platinum = self
                    .prices
                    .as_ref()
                    .and_then(|prices| prices.price_for(&entry.item.name));
                CollectionItemView::priced(entry, platinum)
            })
            .collect::<Vec<_>>();
```

Add the field to `CollectionItemView` (after `image_url`):

```rust
    #[serde(skip_serializing_if = "Option::is_none")]
    platinum: Option<u32>,
```

Add its accessor and constructor, and keep the existing `From` impl delegating so no other caller changes:

```rust
impl CollectionItemView {
    pub fn platinum(&self) -> Option<u32> {
        self.platinum
    }

    fn priced(entry: &warframe_domain::InventoryEntry, platinum: Option<u32>) -> Self {
        Self {
            platinum,
            ..Self::from(entry)
        }
    }
}
```

Add `platinum: None` to the `From<&InventoryEntry>` struct literal.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p app-core`
Expected: PASS, including the existing app-core suites.

- [ ] **Step 5: Commit**

```bash
git add crates/app-core/src/lib.rs crates/app-core/tests/collection_pricing.rs
git commit -m "feat: publish collection platinum prices in the application view"
```

---

### Task 5: Loading prices in the desktop shell

**Files:**
- Modify: `app/src-tauri/src/lib.rs:20-24` (imports), `:1449-1490` (`run`), and add a `start_collection_prices` function near `start_monitor` at `:1411`

**Interfaces:**
- Consumes: `CollectionPriceCache`, `RelicsRunHttp` from Tasks 2-3; `AppCore::set_collection_prices` from Task 4.
- Produces: `fn start_collection_prices(shared: SharedRuntime)`.

- [ ] **Step 1: Write the implementation**

Add to the `warframe_acquisition` import list in `app/src-tauri/src/lib.rs`:

```rust
    CollectionPriceCache, RelicsRunHttp,
```

Add next to `start_monitor`:

```rust
/// Price the collection: cached table first so items are priced before any request is made, then
/// one download for the day's dump.
///
/// There is nothing to schedule here. The whole collection is priced by a single file, so this
/// runs once at start and is done -- no queue, no worker, no rate limiting, because there are no
/// per-item requests to pace.
fn start_collection_prices(shared: SharedRuntime) {
    std::thread::spawn(move || {
        let Some(app_data) = shared.lock().ok().map(|runtime| runtime.app_data.clone()) else {
            return;
        };
        let cache = CollectionPriceCache::new(&app_data);
        if let Some(table) = cache.load_cached()
            && let Ok(mut runtime) = shared.lock()
        {
            let priced = table.len();
            let date = table.dump_date().to_owned();
            runtime.core.set_collection_prices(Arc::new(table));
            let _ = runtime.core.record_market_ready(priced, date);
        }
        let Some(source) = RelicsRunHttp::new() else {
            return;
        };
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|elapsed| elapsed.as_secs())
            .unwrap_or_default();
        match cache.refresh(&source, now) {
            Ok(table) => {
                if let Ok(mut runtime) = shared.lock() {
                    let priced = table.len();
                    let date = table.dump_date().to_owned();
                    runtime.core.set_collection_prices(Arc::new(table));
                    let _ = runtime.core.record_market_ready(priced, date);
                }
            }
            Err(_) => {
                if let Ok(mut runtime) = shared.lock() {
                    let _ = runtime
                        .core
                        .record_market_degraded("No warframe.market price dump could be read");
                }
            }
        }
    });
}
```

Call it in `run`'s setup, beside the existing `start_monitor` call:

```rust
            if should_refresh {
                start_collection_prices(Arc::clone(app.state::<SharedRuntime>().inner()));
                start_monitor(
                    Arc::clone(app.state::<SharedRuntime>().inner()),
                    app.handle().clone(),
                );
            }
```

And in `accept_risk_disclosure`, beside its `start_monitor` call:

```rust
    if result.is_ok() {
        start_collection_prices(Arc::clone(state.inner()));
        start_monitor(Arc::clone(state.inner()), app);
    }
```

- [ ] **Step 2: Verify it builds and the suite still passes**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 3: Commit**

```bash
git add app/src-tauri/src/lib.rs
git commit -m "feat: load the daily price dump when the shell starts"
```

---

### Task 6: Distinct outcomes for the overlay's live lookup

This task is independent of Tasks 1-5 and touches only the reward overlay's own pricing path. It fixes the hole where four different facts all return `None`, and moves that path to the smaller endpoint.

**Files:**
- Modify: `crates/warframe-acquisition/src/market.rs:20-21` (URL and cap), `:40-44` (trait), `:70-80` (parser), `:98-121` (HTTP impl), `:167-184` (`warm`)
- Modify: `crates/warframe-acquisition/tests/market.rs`
- Modify: `crates/warframe-acquisition/src/lib.rs:32-34` (exports)

**Interfaces:**
- Produces: `enum PriceLookup { Priced(u32), NoSellers, Unavailable, Oversize }`, `fn lowest_sell_top(body: &[u8]) -> PriceLookup`, `MarketPriceSource::lowest_sell(&self, name: &str) -> PriceLookup`.

- [ ] **Step 1: Write the failing tests**

Replace the `lowest_sell_price` tests in `crates/warframe-acquisition/tests/market.rs` with:

```rust
use warframe_acquisition::{PriceLookup, lowest_sell_top};

/// The shape of `/v2/orders/item/{slug}/top`: top buy and sell orders, already filtered to sellers
/// who are online, at 4.9 KB against 184 KB for the full order book.
const TOP: &str = r#"{"apiVersion":"0.25.0","data":{
    "sell":[
        {"type":"sell","platinum":19,"visible":true,"user":{"status":"ingame"}},
        {"type":"sell","platinum":20,"visible":true,"user":{"status":"ingame"}}
    ],
    "buy":[{"type":"buy","platinum":12,"visible":true,"user":{"status":"ingame"}}]
},"error":null}"#;

#[test]
fn the_cheapest_online_seller_sets_the_price() {
    assert_eq!(lowest_sell_top(TOP.as_bytes()), PriceLookup::Priced(19));
}

/// An offline seller's price is a number nobody can trade at. Counting them makes every item look
/// cheaper than it is, which would push the advisor toward the wrong card.
#[test]
fn an_offline_or_hidden_seller_is_not_quotable() {
    let body = r#"{"data":{"sell":[
        {"type":"sell","platinum":2,"visible":true,"user":{"status":"offline"}},
        {"type":"sell","platinum":3,"visible":false,"user":{"status":"ingame"}},
        {"type":"sell","platinum":25,"visible":true,"user":{"status":"ingame"}}
    ],"buy":[]}}"#;
    assert_eq!(lowest_sell_top(body.as_bytes()), PriceLookup::Priced(25));
}

#[test]
fn an_item_with_no_online_seller_is_distinct_from_a_failure() {
    let body = r#"{"data":{"sell":[
        {"type":"sell","platinum":2,"visible":true,"user":{"status":"offline"}}
    ],"buy":[]}}"#;
    assert_eq!(lowest_sell_top(body.as_bytes()), PriceLookup::NoSellers);
}

/// The failure that arrives the day warframe.market widens its payload. As an absent price it
/// would present as "every item is worthless", with nothing anywhere saying otherwise.
#[test]
fn an_unreadable_body_is_reported_rather_than_priced_at_nothing() {
    assert_eq!(lowest_sell_top(b"{not json"), PriceLookup::Unavailable);
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p warframe-acquisition --test market`
Expected: FAIL — `unresolved import warframe_acquisition::PriceLookup`.

- [ ] **Step 3: Write the implementation**

In `crates/warframe-acquisition/src/market.rs`:

```rust
const ORDERS_URL: &str = "https://api.warframe.market/v2/orders/item/";
/// The top orders endpoint returns at most five buy and five sell orders, measured at 4.9 KB
/// against 184 KB for the same item's full book. The cap is generous against that so a widened
/// payload does not silently stop every price.
const MAX_ORDERS_BYTES: usize = 256 * 1024;
const USER_AGENT: &str = concat!(
    "TennoScope/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Deftera186/tennoscope)"
);

/// Why an item has no price, kept distinct because the four reasons want different responses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PriceLookup {
    Priced(u32),
    /// The item is listed, but nobody selling it is online.
    NoSellers,
    Unavailable,
    /// The response exceeded the size cap. Its own outcome so diagnostics can name it.
    Oversize,
}

impl PriceLookup {
    pub fn price(self) -> Option<u32> {
        match self {
            Self::Priced(platinum) => Some(platinum),
            _ => None,
        }
    }
}

pub trait MarketPriceSource {
    fn lowest_sell(&self, name: &str) -> PriceLookup;
}

#[derive(Deserialize)]
struct TopResponse {
    data: TopOrders,
}

#[derive(Deserialize)]
struct TopOrders {
    #[serde(default)]
    sell: Vec<Order>,
}

pub fn lowest_sell_top(body: &[u8]) -> PriceLookup {
    let Ok(response) = serde_json::from_slice::<TopResponse>(body) else {
        return PriceLookup::Unavailable;
    };
    response
        .data
        .sell
        .into_iter()
        .filter(|order| order.visible && order.user.status == "ingame")
        .map(|order| order.platinum)
        .min()
        .map_or(PriceLookup::NoSellers, PriceLookup::Priced)
}
```

Delete `OrdersResponse` and `lowest_sell_price`, and drop `order_type` from `Order` — the top endpoint separates buy and sell into their own arrays, so filtering by type is no longer needed.

Update the HTTP implementation's URL and returns:

```rust
impl MarketPriceSource for WarframeMarketHttp {
    fn lowest_sell(&self, name: &str) -> PriceLookup {
        let slug = market_slug(name);
        if slug.is_empty() {
            return PriceLookup::Unavailable;
        }
        let Ok(response) = self.client.get(format!("{ORDERS_URL}{slug}/top")).send() else {
            return PriceLookup::Unavailable;
        };
        let Ok(response) = response.error_for_status() else {
            return PriceLookup::Unavailable;
        };
        let mut body = Vec::new();
        if response
            .take((MAX_ORDERS_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .is_err()
        {
            return PriceLookup::Unavailable;
        }
        if body.len() > MAX_ORDERS_BYTES {
            return PriceLookup::Oversize;
        }
        lowest_sell_top(&body)
    }
}
```

In `MarketPriceCache::warm`, change the store line to use the new type:

```rust
            if let PriceLookup::Priced(price) = source.lowest_sell(name) {
                self.insert(name, price);
                stored += 1;
            }
```

Update the export line in `crates/warframe-acquisition/src/lib.rs`:

```rust
pub use market::{
    MarketPriceCache, MarketPriceSource, PriceLookup, WarframeMarketHttp, lowest_sell_top,
    market_slug,
};
```

- [ ] **Step 4: Fix the remaining callers**

The fake `MarketPriceSource` implementations in `crates/warframe-acquisition/tests/market.rs` must return `PriceLookup` — replace their `Some(price)` with `PriceLookup::Priced(price)` and `None` with `PriceLookup::NoSellers`.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test --workspace`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/warframe-acquisition/src/market.rs crates/warframe-acquisition/src/lib.rs crates/warframe-acquisition/tests/market.rs
git commit -m "fix: tell an oversize price response apart from an unpriced item"
```

---

### Task 7: Prices on the collection card

**Files:**
- Modify: `app/src/backend.ts:7` (`CollectionItem`)
- Modify: `app/src/App.tsx:353-373` (`CollectionEntry`)
- Modify: `app/src/App.css` (a `.price` rule near `.hallmark` at `:298`)
- Modify: `app/src/App.test.tsx`

**Interfaces:**
- Consumes: the `platinum` field published in Task 4.
- Produces: `CollectionItem.platinum?: number`; a `stackValue(item)` helper exported from `app/src/collection.ts`.

- [ ] **Step 1: Invoke the impeccable skill**

The card is existing, deliberately styled interface. Run the `impeccable` skill before writing markup or CSS, and follow it for the price line's typography, weight and placement within `.marks`.

- [ ] **Step 2: Write the failing tests**

Add to `app/src/collection.test.ts`:

```ts
import { stackValue } from './collection'

it('values a stack at its unit price times what is owned', () => {
  expect(stackValue({ quantity: 3, platinum: 19 })).toBe(57)
})

it('has no value for an item with no price', () => {
  expect(stackValue({ quantity: 3 })).toBeNull()
})
```

Add to `app/src/App.test.tsx`, inside the existing describe block, and add `platinum: 19` to the `lex-prime-receiver` item and `platinum: 20` with `quantity: 7` to `lith-a1` in the shared `view` fixture:

```tsx
it('shows the unit price, and the stack total only when more than one is owned', async () => {
  render(<App/>)
  const single = await screen.findByRole('article', { name: 'Lex Prime Receiver' })
  expect(within(single).getByText('19p')).toBeInTheDocument()
  expect(within(single).queryByText(/total/)).not.toBeInTheDocument()

  const stack = await screen.findByRole('article', { name: 'Lith A1 Relic' })
  expect(within(stack).getByText('20p')).toBeInTheDocument()
  expect(within(stack).getByText('140p total')).toBeInTheDocument()
})

it('says nothing rather than zero for an item with no price', async () => {
  render(<App/>)
  const unpriced = await screen.findByRole('article', { name: 'Rhino' })
  expect(within(unpriced).queryByText(/p$/)).not.toBeInTheDocument()
})
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `pnpm -C app test`
Expected: FAIL — `stackValue` is not exported; no price text rendered.

- [ ] **Step 4: Write the implementation**

In `app/src/backend.ts`, extend the interface:

```ts
export interface CollectionItem { id: string; name: string; category: ItemCategory; quantity: number; mastered: boolean; image_url?: string; platinum?: number }
```

In `app/src/collection.ts`:

```ts
/** What the pile is worth, or null when the item has no price. */
export function stackValue(item: { quantity: number; platinum?: number }): number | null {
  return item.platinum === undefined ? null : item.platinum * item.quantity
}
```

In `app/src/App.tsx`, import `stackValue` from `./collection` and add the price line to `CollectionEntry`'s `.marks` block:

```tsx
      <div className="marks">
        {missing
          ? <span className="hallmark absent">Missing</span>
          : <span className="hallmark owned">Owned ×{item.quantity}</span>}
        {item.mastered && <span className="hallmark mastered">Mastered</span>}
        {item.platinum !== undefined && <span className="price">
          {item.platinum}p{item.quantity > 1 && <em> · {stackValue(item)}p total</em>}
        </span>}
      </div>
```

Style `.price` in `app/src/App.css` per the impeccable pass.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `pnpm -C app check`
Expected: PASS, lint and types clean.

- [ ] **Step 6: Commit**

```bash
git add app/src/backend.ts app/src/collection.ts app/src/collection.test.ts app/src/App.tsx app/src/App.css app/src/App.test.tsx
git commit -m "feat: show platinum prices on collection cards"
```

---

### Task 8: Value sort, tradeable filter and collection worth

**Files:**
- Modify: `app/src/App.tsx:21` (`Sort`/`Ownership` types), `:35-39` (`sortOptions`), `:247-343` (`CollectionPage`)
- Modify: `app/src/App.test.tsx`
- Modify: `app/src/App.css` (band cell, if the impeccable pass calls for it)
- Modify: `CHANGELOG.md`

**Interfaces:**
- Consumes: `stackValue` from Task 7.
- Produces: nothing downstream.

- [ ] **Step 1: Invoke the impeccable skill**

The worth cell joins three existing band cells and the filter joins an existing tally row. Run `impeccable` and follow it for the cell's figure, label and note.

- [ ] **Step 2: Write the failing tests**

Add to `app/src/App.test.tsx`:

```tsx
it('sorts by stack value and sinks unpriced items to the bottom', async () => {
  const user = userEvent.setup()
  render(<App/>)
  await user.click(await screen.findByRole('button', { name: 'Value' }))

  const names = screen.getAllByRole('article').map(article => article.getAttribute('aria-label'))
  expect(names.slice(0, 2)).toEqual(['Lith A1 Relic', 'Lex Prime Receiver'])
  expect(names.at(-1)).toBe('Rhino')
})

it('narrows to items that have a price', async () => {
  const user = userEvent.setup()
  render(<App/>)
  await user.click(await screen.findByRole('button', { name: 'Tradeable' }))

  const names = screen.getAllByRole('article').map(article => article.getAttribute('aria-label'))
  expect(names).toEqual(['Lex Prime Receiver', 'Lith A1 Relic'])
})

/// A partial sum shown as a total is a lie the reader cannot detect, so the cell carries its count.
it('sums the priced stacks and says how many it counted', async () => {
  render(<App/>)
  const worth = await screen.findByTestId('band-worth')
  expect(within(worth).getByText('159')).toBeInTheDocument()
  expect(within(worth).getByText(/2 of 8 items priced/)).toBeInTheDocument()
})
```

The expected total is `19 × 1 + 20 × 7 = 159`.

- [ ] **Step 3: Run the tests to verify they fail**

Run: `pnpm -C app test`
Expected: FAIL — no `Value` button, no `Tradeable` button, no `band-worth` element.

- [ ] **Step 4: Write the implementation**

In `app/src/App.tsx`:

```tsx
type Ownership = 'all' | 'owned' | 'mastered' | 'missing' | 'tradeable'
type Sort = 'name-asc' | 'quantity-desc' | 'category-asc' | 'value-desc'
```

```tsx
const sortOptions: Array<{ value: Sort; label: string }> = [
  { value: 'name-asc', label: 'Name A–Z' },
  { value: 'quantity-desc', label: 'Quantity' },
  { value: 'category-asc', label: 'Category' },
  { value: 'value-desc', label: 'Value' },
]
```

In `CollectionPage`, add the filter case and the sort branch:

```tsx
      .filter(item => ownership === 'all'
        || (ownership === 'owned' && item.quantity > 0)
        || (ownership === 'mastered' && item.mastered)
        || (ownership === 'missing' && item.quantity === 0)
        || (ownership === 'tradeable' && item.platinum !== undefined))
      .toSorted((left, right) => sort === 'quantity-desc'
        ? right.quantity - left.quantity || left.name.localeCompare(right.name)
        : sort === 'category-asc'
          ? left.category.localeCompare(right.category) || left.name.localeCompare(right.name)
          : sort === 'value-desc'
            ? (stackValue(right) ?? -1) - (stackValue(left) ?? -1) || left.name.localeCompare(right.name)
            : left.name.localeCompare(right.name))
```

Add `'tradeable'` to the ownership tally row's array at `:322`.

Compute the worth above the return:

```tsx
  const priced = view.collection.items.filter(item => item.platinum !== undefined)
  const worth = priced.reduce((total, item) => total + (stackValue(item) ?? 0), 0)
```

And add the fourth band cell:

```tsx
      <BandCell kind="worth" value={worth} label="Collection worth" note={`${priced.length} of ${view.collection.items.length} items priced`}/>
```

`BandCell` already sets `data-summary={kind}`; add `data-testid={`band-${kind}`}` to it so the test can address the cell.

- [ ] **Step 5: Run the tests to verify they pass**

Run: `pnpm -C app check` and `cargo test --workspace`
Expected: PASS on both.

- [ ] **Step 6: Record the change**

Add to `CHANGELOG.md` under Unreleased:

```markdown
### Added
- Collection items show a platinum price and stack total, from the daily warframe.market price dump.
- Collection sorting by value, a tradeable filter, and a collection worth summary.
```

- [ ] **Step 7: Commit**

```bash
git add app/src/App.tsx app/src/App.css app/src/App.test.tsx CHANGELOG.md
git commit -m "feat: sort, filter and total the collection by platinum value"
```

---

## Self-Review Notes

Spec coverage checked against `docs/design/collection-platinum-pricing.md`: price source (Task 2), four name rules (Task 1), cache and refresh (Tasks 3, 5), application view (Task 4), presentation including the honest count (Tasks 7, 8), the overlay's distinct outcomes (Task 6), failure handling (Tasks 2, 3, 6).

Two spec statements are deliberately **not** implemented as tasks and are recorded in the design's own "Out of Scope": relic refinement pricing, and extending the catalog index to non-prime weapon components.

One spec line has no task and should be picked up if the market health row proves unclear in use: the design says the collection "labels its prices with the date of the dump they came from". Task 5 puts the dump date in the market health row's `last_success` field, which the Diagnostics page renders. If that reads as too buried once the feature is running, surfacing the date on the collection page itself is a follow-up, not a gap in this plan.
