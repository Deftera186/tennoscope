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
