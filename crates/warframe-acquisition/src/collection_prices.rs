//! Collection pricing from the daily warframe.market price dump.
//!
//! The overlay prices four cards live because a reward screen is a decision made in fifteen
//! seconds. A collection is a valuation of hundreds of items, which is a different question and
//! gets a different answer: one file a day, every price in it, no per-item requests at all.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::time::Duration;

use atomicwrites::{AtomicFile, OverwriteBehavior};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, Eq, PartialEq)]
pub enum PriceDumpError {
    #[error("the price dump could not be read")]
    Malformed,
    #[error("collection price cache could not be written")]
    CacheWrite,
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

/// A dump key is a relic listing if it ends in this suffix, e.g. `Axi A1 Relic`.
const RELIC_SUFFIX: &str = " Relic";

/// Every priceable item, keyed by the dump's own English name.
///
/// Two maps, because there are two measurements. `prices` is the daily dump's median sell price.
/// `checked_prices` is what warframe.market answered when somebody asked it directly -- the startup
/// relic sweep or a page refresh -- and it wins wherever it exists, because a price checked against
/// the market minutes ago is better than the middle of a day-old file.
///
/// Relics are excluded from `prices` entirely: the dump has no `perTrade` to divide out, sellers
/// list them six at a time, and the resulting median runs up to 1.5x high (measured: Axi A1 at 25p
/// in the dump against 16.67p per unit live). A relic's dump key still lives in `relic_names` so
/// `market_name` keeps resolving it -- the live path needs that name to build its warframe.market
/// slug -- and its price can only ever come from `checked_prices`.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PriceTable {
    prices: HashMap<String, u32>,
    #[serde(default)]
    relic_names: std::collections::HashSet<String>,
    // The alias reads a cache written before this map held anything but swept relic prices, so an
    // upgrade does not throw away a sweep and re-spend its requests learning the same numbers.
    #[serde(default, alias = "relic_prices")]
    checked_prices: HashMap<String, u32>,
    dump_date: String,
}

impl PriceTable {
    pub fn from_dump_json(bytes: &[u8], dump_date: &str) -> Result<Self, PriceDumpError> {
        let raw: HashMap<String, Vec<DumpRecord>> =
            serde_json::from_slice(bytes).map_err(|_| PriceDumpError::Malformed)?;
        let mut prices = HashMap::new();
        let mut relic_names = std::collections::HashSet::new();
        for (name, records) in raw {
            if name.ends_with(RELIC_SUFFIX) {
                relic_names.insert(name);
            } else if let Some(price) = sell_median(&records) {
                prices.insert(name, price);
            }
        }
        Ok(Self {
            prices,
            relic_names,
            checked_prices: HashMap::new(),
            dump_date: dump_date.to_owned(),
        })
    }

    /// warframe.market's own name for an item the catalog calls `name`.
    ///
    /// The catalog's name and the market's are usually the same string, and where they are not the
    /// difference is one of two known shapes rather than a fuzzy match. What comes back is what
    /// the live lookup builds its slug from, so this is the identity map as much as the price map.
    ///
    /// There is deliberately no rule appending ` Blueprint`. Measured against a real 1,106-item
    /// collection it fired 25 times and was wrong every time: it matches *built* equipment against
    /// its blueprint's listing, pricing an `Ash Prime` the player has mastered at what somebody
    /// asks for the blueprint. A built Warframe cannot be sold, only its parts can, and every one
    /// of that collection's prime parts is in the dump under its own name already.
    pub fn market_name(&self, name: &str) -> Option<&str> {
        if let Some(key) = self.resolve(name) {
            return Some(key);
        }
        if let Some(base) = name.strip_suffix(" Blueprint")
            && let Some(key) = self.resolve(base)
        {
            return Some(key);
        }
        REFINEMENTS
            .iter()
            .find_map(|suffix| name.strip_suffix(suffix))
            .and_then(|base| self.resolve(&format!("{base} Relic")))
    }

    /// A dump key by its own name, whether or not it carries a dump price: a relic resolves here
    /// even though its price comes from the sweep instead.
    fn resolve(&self, name: &str) -> Option<&str> {
        if let Some((key, _)) = self.prices.get_key_value(name) {
            return Some(key);
        }
        self.relic_names.get(name).map(String::as_str)
    }

    /// The best price in hand: what warframe.market last answered directly, and the dump's median
    /// only where nothing has been checked.
    ///
    /// The order matters for every item that is *not* a relic. A relic has no dump price to shadow
    /// a checked one, but a prime part has, and consulting the dump first would quietly discard the
    /// price the player just spent a request on.
    pub fn price_for(&self, name: &str) -> Option<u32> {
        let key = self.market_name(name)?;
        self.checked_prices
            .get(key)
            .or_else(|| self.prices.get(key))
            .copied()
    }

    /// Records what warframe.market answered for this item, keyed by the same market name
    /// `market_name` resolves to. Written by the startup relic sweep and by the page refresh.
    pub fn insert_checked(&mut self, market_name: &str, platinum: u32) {
        self.checked_prices.insert(market_name.to_owned(), platinum);
    }

    /// Whether this item already carries a price checked against warframe.market. What the startup
    /// sweep uses to skip a relic it has already priced rather than re-spending a request on it,
    /// and what the view reads to say a price was checked live rather than taken from the dump.
    pub fn has_checked_price(&self, market_name: &str) -> bool {
        self.checked_prices.contains_key(market_name)
    }

    /// Carry a previous table's checked prices across a refresh of the *same* dump.
    ///
    /// The dumps lag -- on 2026-07-29 the newest published was dated the 27th -- so an ordinary
    /// launch re-downloads a file it already has and parses it into a table whose `checked_prices`
    /// is empty. Without this, every launch would destroy the sweep's work and re-spend 65
    /// requests re-learning the same numbers. A genuinely newer dump still clears them: a checked
    /// price belongs to the day it was made, and the sweep re-runs for the new one. That single
    /// rule is the whole freshness policy, and it is what bounds how stale a stored checked price
    /// can get.
    ///
    /// A price already checked into `self` wins, because it is the newer of the two.
    pub fn adopt_checked(&mut self, previous: &PriceTable) {
        if self.dump_date != previous.dump_date {
            return;
        }
        for (market_name, platinum) in &previous.checked_prices {
            self.checked_prices
                .entry(market_name.clone())
                .or_insert(*platinum);
        }
    }

    /// The relic dump keys the sweep needs to work through.
    pub fn relic_market_names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.relic_names.iter().cloned().collect();
        names.sort();
        names
    }

    pub fn dump_date(&self) -> &str {
        &self.dump_date
    }

    /// How many items this table can price, dump prices and checked prices together. What the
    /// collection price health row reports, so it grows as the sweep lands rather than standing
    /// still at the dump's count while 65 relics gain prices. An item the dump prices and a live
    /// check has since improved is one item, not two.
    pub fn len(&self) -> usize {
        self.prices.len()
            + self
                .checked_prices
                .keys()
                .filter(|name| !self.prices.contains_key(*name))
                .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
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

/// Whether a cached dump of this date is already as new as anything that could be published.
///
/// The dumps lag: on 2026-07-29 the newest published file was dated the 27th, so "not today's
/// date" is no evidence that a newer one exists. Today's or yesterday's is the freshest the
/// publisher ever offers, and downloading 3.9 MB on every launch to be told so is a request
/// nobody needed.
///
/// A dump older than that is refetched even when the publisher has not moved on, which costs one
/// download per launch on a day the feed is two days behind. Avoiding that means remembering when
/// we last asked, which is more state than the saving is worth.
pub fn dump_is_current(dump_date: &str, now_unix: u64) -> bool {
    [0, 86_400]
        .iter()
        .any(|back| civil_date(now_unix.saturating_sub(*back)) == dump_date)
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
        let response = response
            .error_for_status()
            .map_err(|_| PriceFetch::Unavailable)?;
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

    /// Stores a table: the dump just downloaded, or one that has since gained a checked price.
    ///
    /// There is deliberately no `fetch-adopt-store` method here. Fetching takes seconds and must
    /// happen outside the runtime lock, while adopting and storing must happen inside it against
    /// the table the runtime is serving at that moment -- a combined call can only take a snapshot
    /// of the previous table, and any price checked while it was downloading is then lost from
    /// memory and disk alike. `start_collection_prices` composes the two steps around the lock.
    pub fn store_table(&self, table: &PriceTable) -> Result<(), PriceDumpError> {
        self.store(table)
    }

    fn store(&self, table: &PriceTable) -> Result<(), PriceDumpError> {
        fs::create_dir_all(&self.directory).map_err(|_| PriceDumpError::CacheWrite)?;
        let bytes = serde_json::to_vec(table).map_err(|_| PriceDumpError::CacheWrite)?;
        AtomicFile::new(self.path(), OverwriteBehavior::AllowOverwrite)
            .write(|file| file.write_all(&bytes).and_then(|_| file.sync_all()))
            .map_err(|_| PriceDumpError::CacheWrite)
    }
}
