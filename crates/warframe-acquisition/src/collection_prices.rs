//! Collection pricing from the daily warframe.market price dump.
//!
//! The overlay prices four cards live because a reward screen is a decision made in fifteen
//! seconds. A collection is a valuation of hundreds of items, which is a different question and
//! gets a different answer: one file a day, every price in it, no per-item requests at all.

use std::collections::HashMap;
use std::io::Read;
use std::time::Duration;

use reqwest::blocking::Client;
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

    /// warframe.market's own name for an item the catalog calls `name`.
    ///
    /// The catalog's name and the market's are usually the same string, and where they are not the
    /// difference is one of three known shapes rather than a fuzzy match. What comes back is what
    /// the live lookup builds its slug from, so this is the identity map as much as the price map.
    pub fn market_name(&self, name: &str) -> Option<&str> {
        if let Some((key, _)) = self.prices.get_key_value(name) {
            return Some(key);
        }
        if let Some((key, _)) = self.prices.get_key_value(&format!("{name} Blueprint")) {
            return Some(key);
        }
        if let Some(base) = name.strip_suffix(" Blueprint")
            && let Some((key, _)) = self.prices.get_key_value(base)
        {
            return Some(key);
        }
        REFINEMENTS
            .iter()
            .find_map(|suffix| name.strip_suffix(suffix))
            .and_then(|base| self.prices.get_key_value(&format!("{base} Relic")))
            .map(|(key, _)| key.as_str())
    }

    pub fn price_for(&self, name: &str) -> Option<u32> {
        self.prices.get(self.market_name(name)?).copied()
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
