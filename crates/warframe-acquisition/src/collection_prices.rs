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

/// Refinement tiers the catalog appends to a relic's name.
///
/// warframe.market lists one relic per slug and separates the tiers by `subtype`, and they are not
/// one price: measured 2026-07-30 over 80 relics, a radiant sells for a median 1.46x its intact
/// tier and up to 17x (`Requiem II`, 1p intact against 17p radiant). Pricing all four at the one
/// listing quoted every radiant relic at what the cheapest intact copy was going for.
const REFINEMENTS: [&str; 4] = [" Intact", " Exceptional", " Flawless", " Radiant"];

#[derive(Deserialize)]
struct DumpRecord {
    order_type: String,
    #[serde(default)]
    median: Option<f64>,
    /// The rank the record quotes, where the listing has ranks at all.
    #[serde(default)]
    mod_rank: Option<u32>,
    /// Which variant of the listing this record quotes: a relic's refinement, a fish's size, an
    /// Ayatan's socket count. Absent on listings that have only one.
    #[serde(default)]
    subtype: Option<String>,
    /// Trades behind a `closed` record; listings behind a `sell` one. Read as `f64` rather than an
    /// integer because a malformed dump is rejected whole, and one fractional volume must not cost
    /// the collection every other price in the file.
    #[serde(default)]
    volume: Option<f64>,
}

/// How many completed trades a `closed` record needs before it is read as a price at all.
///
/// A closed record standing on one or two trades is one player's odd deal, not a price: on the
/// 2026-07-30 dump `Pressure Point` closed at 50p against a 1p ask on a single trade. Three is
/// enough to be a median of something. It is not enough on its own -- `Vitality` closed at 115p
/// against a 1p ask on four -- which is why the ask bounds it rather than losing to it.
const MIN_CLOSED_VOLUME: f64 = 3.0;

/// How many of one listing warframe.market completed on the day this dump covers.
///
/// `subtype` selects a relic's refinement tier; `None` counts every record, which is what a listing
/// whose variant the inventory cannot name is sold into. Ranks are always counted together: a stack
/// is sold at whatever rank it is at, and the market's appetite is for the mod.
///
/// `MIN_CLOSED_VOLUME` deliberately does not apply. That floor asks whether a *median* is a price,
/// and one trade is a poor median but a perfectly real trade. Nothing else here reads a `closed`
/// record for anything but its count.
fn daily_trade_count(records: &[DumpRecord], subtype: Option<&str>) -> Option<f64> {
    let daily: f64 = records
        .iter()
        .filter(|record| record.order_type == "closed")
        .filter(|record| subtype.is_none() || record.subtype.as_deref() == subtype)
        .filter_map(|record| record.volume)
        .filter(|volume| volume.is_finite() && *volume > 0.0)
        .sum();
    (daily > 0.0).then_some(daily)
}

/// The window a holding is valued over: what the market takes in a month, matching `CARRY_DAYS`.
///
/// Long enough that a slow listing still registers, short enough to be a plan rather than a
/// retirement. Nothing about a longer window is more honest -- the figure already assumes the
/// player personally makes every trade in the game for these items.
const MONTH_DAYS: u32 = 30;

/// The floor on how much of the running trade rate one day's dump may be, once `MONTH_DAYS` of them
/// have been averaged.
///
/// Until then each dump gets an equal share, which is a plain mean of every day seen; after, the
/// oldest fade. This is the cheap way to keep a month's mean: a month of dumps is several thousand
/// daily counts per item, and the cache is a file the app rewrites on every checked price, so
/// storing them would cost more than the figure they support is worth. Weighting today at a flat
/// thirtieth from the first day instead leaves the very first dump 40% of the estimate a month
/// later -- measured, it read `Quickdraw`, which the game trades twice a month, as fifteen.
const DAY_WEIGHT: f64 = 1.0 / MONTH_DAYS as f64;

/// Every `(subtype, rank)` the listing quotes, and what one unit of each costs.
///
/// A group's price is the lowest of the two measurements the dump carries for it. Neither survives
/// being trusted alone. `sell` -- what sellers ask -- quotes a bulk listing's whole *lot*, and the
/// dump mirrors warframe.market unmodified, so a six-pack enters the day's median at six times what
/// one item costs: measured 2026-07-30 on `lith_t11_relic` intact, 30p asked against the 4.5p it
/// traded at, where the online sellers' own per-unit asks were 4.67-5.00p. `closed` -- what trades
/// actually completed at -- is per unit and carries no such fault, but it is a thin sample on most
/// items, and a thin sample runs the other way: `Vitality` unranked closed at 115p on four trades
/// against an ask of 1p backed by 3,186 listings.
///
/// Taking the lower answers both. A lot-inflated ask always loses to the trade, and a freak trade
/// always loses to the ask. On the 2026-07-30 dump, 15 of the 3,179 groups quoting both had a
/// closed median above their own ask and every one of them was reading high.
///
/// `trust_asks` is false for relics, whose ask is the broken measurement this whole function exists
/// to route around: a relic with no trustworthy `closed` record must read as unpriced rather than
/// at six times its worth. Everything else keeps the ask, because for the 1,617 items whose closed
/// record is absent or thin it is the only number there is.
///
/// Floored at 1p, as the live path's per-unit division already is. No median in the 2026-07-30 dump
/// rounds to zero, but one below 0.5p would, and "0p" reads as free rather than as cheap.
fn quotes(records: &[DumpRecord], trust_asks: bool) -> HashMap<(Option<String>, Option<u32>), u32> {
    let mut groups: HashMap<(Option<String>, Option<u32>), f64> = HashMap::new();
    for record in records {
        let Some(median) = record
            .median
            .filter(|median| median.is_finite() && *median >= 0.0)
        else {
            continue;
        };
        let quoted = match record.order_type.as_str() {
            "closed" => record.volume.unwrap_or(0.0) >= MIN_CLOSED_VOLUME,
            "sell" => trust_asks,
            _ => false,
        };
        if !quoted {
            continue;
        }
        groups
            .entry((record.subtype.clone(), record.mod_rank))
            .and_modify(|seen| *seen = seen.min(median))
            .or_insert(median);
    }
    groups
        .into_iter()
        .map(|(group, median)| (group, (median.round() as u32).max(1)))
        .collect()
}

/// The price-table key for one of a relic's subtypes: `Axi A1 Relic` for intact, and
/// `Axi A1 Relic (Radiant)` for the refined tiers, which is what `market_name` resolves to.
///
/// `None` for a subtype `REFINEMENTS` does not name, because a key invented here is one no name
/// rule would ever ask for again.
fn relic_tier_name(name: &str, subtype: Option<&str>) -> Option<String> {
    let Some(subtype) = subtype else {
        return Some(name.to_owned());
    };
    let tier = REFINEMENTS
        .iter()
        .map(|suffix| suffix.trim_start())
        .find(|tier| tier.eq_ignore_ascii_case(subtype))?;
    Some(match tier {
        "Intact" => name.to_owned(),
        other => format!("{name} ({other})"),
    })
}

/// A dump key is a relic listing if it ends in this suffix, e.g. `Axi A1 Relic`.
const RELIC_SUFFIX: &str = " Relic";

/// What to show for a stack of copies that carry a rank.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RankedPrice {
    /// The figure the collection's total is built from. A floor, never a guess, whenever `ceiling`
    /// is also set.
    pub platinum: Option<u32>,
    /// The maxed quote, present only when these copies sit between the two ranks the market prices.
    pub ceiling: Option<u32>,
}

/// The intact listing behind a refined relic's market name: `Axi A1 Relic (Radiant)` to
/// `Axi A1 Relic`, and `None` for anything that is not a refined relic.
///
/// The parenthetical is only read off a name already ending in ` Relic`, so it cannot mistake a
/// market name that carries brackets of its own -- `Rifle Riven Mod (Veiled)` -- for a refinement.
pub fn relic_base(market_name: &str) -> Option<&str> {
    let (base, refinement) = market_name.split_once(" (")?;
    (base.ends_with(RELIC_SUFFIX) && refinement.ends_with(')')).then_some(base)
}

/// Every priceable item, keyed by the dump's own English name.
///
/// Three price maps, in order of freshness. `prices` is today's dump. `carried_relic_prices` is
/// what earlier dumps said about relics today's is silent on. `checked_prices` is what
/// warframe.market answered when the player asked it directly about the page in front of them, and
/// it wins over both, because a price checked minutes ago beats the middle of a day-old file. A
/// fourth collection, `checked_unpriced`, remembers the names the market answered about with
/// nothing for sale, so that answer is not mistaken for an unasked question.
///
/// A relic is priced from its `closed` records only. warframe.market's `sell` statistics -- which
/// the dump mirrors unmodified -- quote a bulk listing's whole lot, and sellers list relics six at a
/// time, so the ask reads at six times what one relic costs: measured 2026-07-30 on
/// `lith_t11_relic` intact, 30p asked against 4.5p traded and 4.67-5.00p per unit across the four
/// online sellers. `closed` is per unit and needs no divisor, but only 163 of 772 relics carry one
/// on a given day -- which is what `carried_relic_prices` and `adopt` are for. A relic's dump key
/// lives in `relic_names` whether or not it was priced, because the live path builds its
/// warframe.market slug from that name.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct PriceTable {
    prices: HashMap<String, u32>,
    #[serde(default)]
    relic_names: std::collections::HashSet<String>,
    #[serde(default)]
    checked_prices: HashMap<String, u32>,
    /// Names warframe.market answered about with no online seller.
    ///
    /// "Nobody is selling this" is an answer, not a failed request, and without somewhere to put it
    /// a second refresh of the same page re-asks about every item nobody is selling. It keeps the
    /// company of `checked_prices` rather than living on its own clock: both are things the market
    /// told us, both belong to the dump they were told against, and both are dropped together when
    /// a genuinely newer dump lands.
    #[serde(default)]
    checked_unpriced: std::collections::HashSet<String>,
    /// What a fully-ranked copy sells for, for the names that quote a rank at all.
    ///
    /// A second map rather than a second key format, because this is the same item: the filters,
    /// the live path and `market_name` all key on the one listing name, and only the price differs.
    #[serde(default)]
    max_rank_prices: HashMap<String, u32>,
    /// Relic prices kept from the dumps that produced them, with that dump's date.
    ///
    /// A relic is priced from `closed` records alone, and only 163 of 772 relics carry one on any
    /// given day -- so a table built from one dump prices 44% of a real collection's relics and
    /// leaves the rest blank. The dumps do not disagree; they are sparse. Union them and the same
    /// collection reaches 76% at three days, 86% at seven and 96% at twenty-eight, for no extra
    /// request at all: the app already downloads a dump a day and was throwing the previous one's
    /// relic prices away.
    ///
    /// Kept apart from `prices` rather than folded into it so the two can be told apart -- today's
    /// file said this, an older file said that -- and so `CARRY_DAYS` has a date to enforce.
    #[serde(default)]
    carried_relic_prices: HashMap<String, (u32, String)>,
    /// Completed trades a day, averaged over the dumps seen so far, with how many of them that is
    /// and the last one that saw a trade. `monthly_trades` reads it out as a month's worth.
    ///
    /// The price says what one copy is worth. This says how many copies anyone wanted, which is the
    /// other half of what a holding is actually worth: this account owns 182 `Quickdraw` and the
    /// entire game traded two of them in twenty-eight days. No stack is worth more than the market
    /// takes, however correct its unit price.
    ///
    /// Keyed exactly like `prices`, so a relic's tiers count separately. Averaged rather than read
    /// off today's file, because a day's dump is a sparse sample and both plainer readings of it are
    /// biased. Today's count alone understated a real account's sellable total by about a quarter and
    /// swung it by a tenth with whichever listings happened to trade that morning: `Intruder`
    /// completed 159 trades over twenty-eight days and carries no `closed` record at all on the 30th.
    /// Carrying the last count *seen* overstates just as hard, because it conditions on a day where a
    /// trade happened -- it read `Quickdraw` as thirty a month.
    #[serde(default)]
    trade_rate: HashMap<String, (f64, u32, String)>,
    dump_date: String,
    /// Which version of the parse produced `prices`. Absent, and so zero, in every table written
    /// before this field existed.
    #[serde(default)]
    schema: u32,
}

/// Bump this whenever `from_dump_json` changes what a stored price means.
///
/// The cache stores the parsed table, not the download, and `dump_is_current` skips the download
/// while the stored date is today's or yesterday's. So a table keeps whatever the parse meant on
/// the day it was written, for as long as that date stays current -- and every checked price
/// rewrites the file, which makes the stale numbers look freshly saved.
///
/// The 2026-07-29 dump was parsed into this cache before subtypes were
/// priced separately, so it stored `Serration` at 48p and `Arcane Reaper` at 400p: the *maxed*
/// medians, under the plain listing name. The fix shipped, the cache did not move, and both ranks
/// of every mod went on showing the maxed price.
///
/// Bumped to 2 when `closed` became the preferred measurement: every stored price is now what the
/// item traded at rather than what was asked for it, which moved 1,442 of 3,059 non-relic items and
/// gave 142 relics a dump price they never had.
///
/// Bumped to 3 for `trade_rate`, which a table written before it existed does not carry at all --
/// and an absent rate reads as an untraded item, so the whole collection would be worth nothing
/// until the next dump. The same bump covers a `prices` map that no longer answers for an unranked
/// copy from a rank-only quote.
const CACHE_SCHEMA: u32 = 3;

impl PriceTable {
    pub fn from_dump_json(bytes: &[u8], dump_date: &str) -> Result<Self, PriceDumpError> {
        let raw: HashMap<String, Vec<DumpRecord>> =
            serde_json::from_slice(bytes).map_err(|_| PriceDumpError::Malformed)?;
        let mut prices = HashMap::new();
        let mut max_rank_prices = HashMap::new();
        let mut trade_rate = HashMap::new();
        let mut relic_names = std::collections::HashSet::new();
        for (name, records) in raw {
            let relic = name.ends_with(RELIC_SUFFIX);
            let groups = quotes(&records, !relic);
            if relic {
                // A relic is priced per refinement tier, under the same key `market_name` produces,
                // because the tiers are separate subtypes of one listing and are not one price.
                for ((subtype, _), price) in &groups {
                    if let Some(tier) = relic_tier_name(&name, subtype.as_deref()) {
                        if let Some(traded) = daily_trade_count(&records, subtype.as_deref()) {
                            trade_rate.insert(tier.clone(), (traded, 1, dump_date.to_owned()));
                        }
                        prices.insert(tier, *price);
                    }
                }
                relic_names.insert(name);
                continue;
            }
            // Every subtype at once, unlike the price: the inventory usually cannot say which
            // variant a stack is, so the trades that variant would be sold into cannot be singled
            // out either.
            if let Some(traded) = daily_trade_count(&records, None) {
                trade_rate.insert(name.clone(), (traded, 1, dump_date.to_owned()));
            }
            if let Some(maxed) = max_rank_price(&groups) {
                max_rank_prices.insert(name.clone(), maxed);
            }
            // The cheapest quoted variant, among the ones that speak for an unranked copy. The
            // subtype is often not knowable from the inventory -- a fish's size, an Ayatan's socket
            // count -- so the lowest is the least the player is certainly holding; taking whichever
            // record came first in the file valued a `Tromyzon` at its `magnificent` 10p when its
            // `basic` was 2p.
            //
            // A rank above 0 is excluded rather than merely outranked. Seven listings in the dump
            // are quoted at one rank and no other -- `Scan Matter` at rank 3 alone, and six more
            // between 80p and 300p -- and a minimum over every group handed that maxed quote to
            // every copy for want of anything else, pricing a 0/3 `Scan Matter` at 240p. Having no
            // price is the honest answer there: `max_rank_price` above still keeps the quote for
            // the copies that earned it, and the name still resolves, so the page refresh can go
            // and ask.
            if let Some(price) = groups
                .iter()
                .filter(|((_, rank), _)| rank.unwrap_or(0) == 0)
                .map(|(_, price)| *price)
                .min()
            {
                prices.insert(name, price);
            }
        }
        Ok(Self {
            prices,
            relic_names,
            checked_prices: HashMap::new(),
            checked_unpriced: std::collections::HashSet::new(),
            max_rank_prices,
            carried_relic_prices: HashMap::new(),
            trade_rate,
            dump_date: dump_date.to_owned(),
            schema: CACHE_SCHEMA,
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
    /// A relic carries its refinement, because warframe.market prices the tiers separately: an
    /// `Axi A1 Radiant` resolves to `Axi A1 Relic (Radiant)`. Intact keeps the bare listing name,
    /// which is both the tier the market means by default and the key every price checked before
    /// this distinction existed was stored under.
    pub fn market_name(&self, name: &str) -> Option<String> {
        if let Some(key) = self.resolve(name) {
            return Some(key.to_owned());
        }
        // A refined relic's own market name maps to itself. Both `price_for` and the view are
        // called with a market name as often as with the catalog's, and without this the name this
        // very function produced would fail to resolve on the way back in.
        if let Some(base) = relic_base(name)
            && self.relic_names.contains(base)
        {
            return Some(name.to_owned());
        }
        if let Some(base) = name.strip_suffix(" Blueprint")
            && let Some(key) = self.resolve(base)
        {
            return Some(key.to_owned());
        }
        let (base, refinement) = REFINEMENTS
            .iter()
            .find_map(|suffix| Some((name.strip_suffix(suffix)?, suffix.trim_start())))?;
        let relic = self.resolve(&format!("{base}{RELIC_SUFFIX}"))?;
        Some(match refinement {
            "Intact" => relic.to_owned(),
            other => format!("{relic} ({other})"),
        })
    }

    /// A dump key by its own name, whether or not it carries a dump price: a relic resolves here
    /// even though its price may have come from an earlier dump than this one.
    fn resolve(&self, name: &str) -> Option<&str> {
        if let Some((key, _)) = self.prices.get_key_value(name) {
            return Some(key);
        }
        // A listing quoted only above rank 0 has no entry in `prices` and is still a name this
        // table knows: a maxed copy has a real price to read, and an unranked one is priceable in
        // the sense that matters -- warframe.market can be asked about it.
        if let Some((key, _)) = self.max_rank_prices.get_key_value(name) {
            return Some(key);
        }
        self.relic_names.get(name).map(String::as_str)
    }

    /// The best price in hand: what warframe.market last answered directly, then today's dump, then
    /// the newest earlier dump that priced it.
    ///
    /// The order matters for every item, relics included. A live order book beats a day-old
    /// completed trade, so consulting the dump first would quietly discard the price the player
    /// just spent a request on. Today's dump beats an older one for the same reason, one day down.
    pub fn price_for(&self, name: &str) -> Option<u32> {
        let key = self.market_name(name)?;
        self.quoted(&key)
            // A refined relic falls back to the intact listing, which is the tier that is actually
            // traded: measured over 80 relics, nobody at all was selling `exceptional` or
            // `flawless`, and 61% of radiants had no online seller either. Without the fallback
            // those tiers would read as a dash forever, which is a worse answer than the intact
            // floor they already showed before the tiers were told apart. The intact tier's own
            // dump price counts here as well as its checked one, since intact is the tier with a
            // trustworthy `closed` record in 157 of the 172 relic records that have one at all.
            .or_else(|| self.quoted(relic_base(&key)?))
    }

    /// How many copies of `name` warframe.market completes in a month, if any.
    ///
    /// `None` and `Some(0)` mean the same thing on purpose: nothing in the last thirty days of
    /// dumps recorded anybody buying one. A stack the market does not touch is worth its unit price
    /// and nothing in total, which is the whole point of the figure -- `Scan Matter` is a 240p mod
    /// that has traded 0 times in twenty-eight days.
    pub fn monthly_trades(&self, name: &str) -> Option<u32> {
        let key = self.market_name(name)?;
        self.trade_rate
            .get(&key)
            .or_else(|| self.trade_rate.get(relic_base(&key)?))
            .map(|(rate, ..)| (rate * f64::from(MONTH_DAYS)).round() as u32)
    }

    /// One key's price, through every source in order of freshness.
    fn quoted(&self, key: &str) -> Option<u32> {
        self.checked_prices
            .get(key)
            .or_else(|| self.prices.get(key))
            .copied()
            .or_else(|| self.carried_relic_prices.get(key).map(|(price, _)| *price))
    }

    /// The price for copies at a given rank.
    ///
    /// A mod or arcane is worth what its rank is worth, and the market says so in two numbers per
    /// listing and no more: rank 0 and the ceiling. So there are three answers, not one. Unranked
    /// copies take the rank-0 median, which is the only price this table held before. Fully ranked
    /// copies take the ceiling's. A copy stopped somewhere in between has no quote anywhere -- the
    /// market simply does not trade half-ranked cards -- and the honest report of that is the pair
    /// it sits between, not either end passed off as the answer.
    pub fn ranked_price_for(
        &self,
        name: &str,
        rank: Option<u32>,
        at_max_rank: Option<bool>,
    ) -> RankedPrice {
        let unranked = self.price_for(name);
        if rank.unwrap_or(0) == 0 {
            return RankedPrice {
                platinum: unranked,
                ceiling: None,
            };
        }
        let maxed = self
            .market_name(name)
            .and_then(|key| self.max_rank_prices.get(&key).copied());
        match at_max_rank {
            // The ceiling is a real quote for these copies, so it is the price rather than a bound.
            Some(true) => RankedPrice {
                platinum: maxed.or(unranked),
                ceiling: None,
            },
            // Part-ranked, or a riven whose ceiling the catalogue does not honestly publish. Either
            // way the maxed quote is not this copy's, and claiming it would overstate the holding.
            _ => RankedPrice {
                platinum: unranked,
                ceiling: maxed,
            },
        }
    }

    /// Records what warframe.market answered for this item, keyed by the same market name
    /// `market_name` resolves to. Written by the page refresh.
    pub fn insert_checked(&mut self, market_name: &str, platinum: u32) {
        self.checked_unpriced.remove(market_name);
        self.checked_prices.insert(market_name.to_owned(), platinum);
    }

    /// Records that warframe.market was asked about this item and had nobody selling it.
    ///
    /// Only for `PriceLookup::NoSellers`. An unreachable endpoint must keep retrying -- recording
    /// an outage here would blacklist a relic until tomorrow's dump over a router that rebooted.
    pub fn mark_checked_unpriced(&mut self, market_name: &str) {
        if !self.checked_prices.contains_key(market_name) {
            self.checked_unpriced.insert(market_name.to_owned());
        }
    }

    /// Whether this item already carries a price checked against warframe.market. What the view
    /// reads to say a price was checked live rather than taken from the dump.
    ///
    /// Follows the same fallback `price_for` does, so a radiant relic showing its intact tier's
    /// *checked* price is not labelled as having come from the dump. Deliberately narrower than
    /// that fallback in one place: an intact tier's *dump* price borrowed by a radiant is a dump
    /// price, and says so.
    pub fn has_checked_price(&self, market_name: &str) -> bool {
        self.checked_prices.contains_key(market_name)
            || relic_base(market_name).is_some_and(|base| self.checked_prices.contains_key(base))
    }

    /// Whether warframe.market has already answered about this item at all, price or no price.
    ///
    /// Filtering on `has_checked_price` instead meant every item nobody happened to be selling
    /// failed the test forever, so a second pass over the same page re-spent the same requests to
    /// be told the same thing.
    ///
    /// Deliberately exact where `has_checked_price` falls back: a radiant relic borrowing the
    /// intact price has still never been asked about, and sharing the fallback here would report
    /// it as answered.
    pub fn has_been_checked(&self, market_name: &str) -> bool {
        self.checked_prices.contains_key(market_name) || self.checked_unpriced.contains(market_name)
    }

    /// Take from the table this one replaces everything that outlives a download.
    ///
    /// Two different lifetimes, because they answer to two different clocks.
    ///
    /// **Relic dump prices carry across dumps**, up to `CARRY_DAYS`. Each daily file prices only
    /// the 163-odd relics that happened to trade that day, so any one of them leaves most of a
    /// collection blank while the file before it had the answer. Unioning them takes a real
    /// collection's relic coverage from 44% to 96% at no cost -- the download already happens --
    /// and it is what lets the startup sweep go away entirely. The staleness is affordable because
    /// relics are cheap: a typical closed median is 4.2p and 90% are under 7p, so the median 20%
    /// drift across a month is 0.8p on an item.
    ///
    /// **Checked prices belong to their own dump's day** and are dropped when a genuinely newer
    /// one lands. They are the live path's answers, and the live path is now only ever the player
    /// asking about the page in front of them; re-asking is a click away and costs one request.
    /// The dumps lag -- on 2026-07-29 the newest published was dated the 27th -- so an ordinary
    /// launch re-downloads a file it already has and parses it into a table whose `checked_prices`
    /// is empty, which is the case this exists for.
    ///
    /// A price already in `self` wins throughout, because it is the newer of the two. The
    /// no-seller marks ride along with the checked prices on the same rule: they are answers from
    /// the same day, and dropping them while keeping the prices would put the page refresh back to
    /// re-asking about every relic nobody is selling.
    pub fn adopt(&mut self, previous: &PriceTable) {
        // Everything the last table could price a relic from, each under the dump that produced it
        // -- its own carried entries, plus its dump prices, which its dump date vouches for. The
        // two are disjoint: a name is only carried while no dump in hand prices it.
        let inherited = previous
            .carried_relic_prices
            .iter()
            .map(|(name, (platinum, from))| (name, *platinum, from.as_str()))
            .chain(
                previous
                    .prices
                    .iter()
                    .filter(|(name, _)| is_relic_key(name))
                    .map(|(name, platinum)| (name, *platinum, previous.dump_date.as_str())),
            );
        for (name, platinum, from) in inherited {
            if self.prices.contains_key(name) || self.expired(from) {
                continue;
            }
            let held = self
                .carried_relic_prices
                .entry(name.clone())
                .or_insert_with(|| (platinum, from.to_owned()));
            if held.1.as_str() < from {
                *held = (platinum, from.to_owned());
            }
            // The name comes with the price. `resolve` answers from `relic_names`, and a relic
            // carried from a dump this one does not list would otherwise hold a price that nothing
            // could look up.
            self.relic_names
                .insert(relic_base(name).unwrap_or(name).to_owned());
        }
        // The market's appetite is averaged over every dump seen, not read off this one: a day's
        // file is a sample of the market, and one sample of a listing that trades weekly is either
        // nothing or a week's worth. A listing with no record today contributes the zero it earned,
        // which is how a rate falls as well as rises.
        //
        // The date carried is the last dump that saw a trade, not today, so a listing nobody has
        // bought in `CARRY_DAYS` drops out entirely rather than decaying towards zero forever.
        for (name, (rate, days, seen)) in &previous.trade_rate {
            if self.expired(seen) {
                continue;
            }
            let today = self.trade_rate.get(name);
            let observed = today.map_or(0.0, |(rate, ..)| *rate);
            let seen = today.map_or(seen, |(_, _, seen)| seen).clone();
            let weight = (1.0 / f64::from(days + 1)).max(DAY_WEIGHT);
            self.trade_rate.insert(
                name.clone(),
                (
                    observed * weight + rate * (1.0 - weight),
                    (days + 1).min(MONTH_DAYS),
                    seen,
                ),
            );
        }
        if self.dump_date != previous.dump_date {
            return;
        }
        for (market_name, platinum) in &previous.checked_prices {
            self.checked_prices
                .entry(market_name.clone())
                .or_insert(*platinum);
        }
        for market_name in &previous.checked_unpriced {
            self.mark_checked_unpriced(market_name);
        }
    }

    /// Whether a price from `from` is too old to stand behind under this table's dump date.
    ///
    /// An unreadable date counts as expired. A price whose provenance cannot be stated is one the
    /// register cannot describe, and the honest report of that is a dash.
    fn expired(&self, from: &str) -> bool {
        match (epoch_day(from), epoch_day(&self.dump_date)) {
            (Some(from), Some(now)) => now - from > CARRY_DAYS,
            _ => true,
        }
    }

    pub fn dump_date(&self) -> &str {
        &self.dump_date
    }

    /// How many items this table can price, dump prices and checked prices together. What the
    /// collection price health row reports, so it grows as a page refresh lands rather than
    /// standing still at the dump's count. An item the dump prices and a live check has since
    /// improved is one item, not two.
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

/// What a fully-ranked copy is quoted at, or nothing if this listing has no ranks.
///
/// The highest quoted rank is the maxed one. warframe.market publishes exactly two per rankable
/// listing -- rank 0 and the ceiling -- measured across the 2026-07-29 dump, where 1,512 of the
/// 1,514 rankable names quote precisely two and the ceiling is 3, 5 or 10. Nothing is quoted in
/// between, which is why a half-ranked copy has no price of its own to show.
///
/// The rank is matched, never crossed, which is the whole reason `quotes` groups by it. `Serration`
/// on 2026-07-30 carries a `closed` record at rank 10 and none at rank 0: reading closed without
/// pairing it to its own rank would price every unranked copy at the maxed 20p, which is exactly
/// the fault `CACHE_SCHEMA` was bumped for the first time.
fn max_rank_price(groups: &HashMap<(Option<String>, Option<u32>), u32>) -> Option<u32> {
    groups
        .iter()
        .filter_map(|((_, rank), price)| Some(((*rank)?, *price)))
        .filter(|(rank, _)| *rank > 0)
        .max_by_key(|(rank, _)| *rank)
        .map(|(_, price)| price)
}

/// Whether a `prices` key belongs to a relic: the bare listing, or one of its refined tiers.
fn is_relic_key(name: &str) -> bool {
    name.ends_with(RELIC_SUFFIX) || relic_base(name).is_some()
}

/// How many days a relic price may be carried past the dump that produced it.
///
/// Long enough to be worth carrying and short enough to stay defensible. A real collection's relic
/// coverage is 86% at seven days and 96% at twenty-eight, so the window has to reach into weeks to
/// do its job at all; past a month the price stops being a stale reading and starts being a guess,
/// and a relic still uncarried at that point is one nobody has traded in a month.
const CARRY_DAYS: i64 = 30;

/// Days since the Unix epoch for a `YYYY-MM-DD` date. The inverse of `civil_date`, and the only
/// date arithmetic in here: dump dates compare as strings everywhere else.
fn epoch_day(date: &str) -> Option<i64> {
    let mut parts = date.split('-');
    let year: i64 = parts.next()?.parse().ok()?;
    let month: i64 = parts.next()?.parse().ok()?;
    let day: i64 = parts.next()?.parse().ok()?;
    if parts.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let shifted_month = if month > 2 { month - 3 } else { month + 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    Some(era * 146_097 + day_of_era - 719_468)
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

    /// The stored table, unless it was written by a parse that no longer means what this one does.
    ///
    /// A rejected cache costs one 3.9 MB download and the checked prices in it. Keeping
    /// it costs wrong prices until the publisher moves on, which is the worse of the two.
    pub fn load_cached(&self) -> Option<PriceTable> {
        let bytes = fs::read(self.path()).ok()?;
        let table: PriceTable = serde_json::from_slice(&bytes).ok()?;
        (table.schema == CACHE_SCHEMA && !table.is_empty()).then_some(table)
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
