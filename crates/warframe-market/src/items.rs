//! warframe.market's item table, which is the only thing that can join an order to the collection.
//!
//! An order names an `itemId`, an opaque market identifier that appears nowhere in the game. The
//! collection is keyed by `/Lotus/` path. `GET /v2/items` publishes `gameRef`, which is exactly
//! that path, and is the whole reason reconciliation can be done at all rather than by matching
//! display names and hoping.
//!
//! Measured 2026-08-01: 1.61 MB, 3,837 entries, one request. 3,802 carry a `gameRef`; the 35 that
//! do not are fusion cores, void keys and similar retired items, and an order for one of those is
//! reported as unverifiable rather than guessed at.

use std::collections::HashMap;

use serde::Deserialize;

use crate::{API_V2, MarketError, MarketRequest, MarketTransport, Method};

/// The cap on the item payload. The live response is 1.61 MB; this is generous against that so a
/// grown table does not silently stop every reconciliation, while still bounding what one
/// unexpected response can allocate.
pub const MAX_ITEMS_BYTES: usize = 8 * 1024 * 1024;

#[derive(Deserialize)]
struct ItemsResponse {
    data: Vec<ItemRecord>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ItemRecord {
    id: String,
    /// The `/Lotus/` path. Absent for retired items that no longer exist in the game.
    #[serde(default)]
    game_ref: Option<String>,
    /// Present when one market item stands for several collection rows -- a relic publishes one
    /// entry with four refinements, and the collection holds each refinement separately.
    #[serde(default)]
    subtypes: Option<Vec<String>>,
    /// The four fields that make a listing's shape contextual. Each is present exactly when the
    /// item supports that dimension, and the POST that omits the matching field is rejected.
    #[serde(default)]
    max_rank: Option<u32>,
    #[serde(default)]
    max_amber_stars: Option<u32>,
    #[serde(default)]
    max_cyan_stars: Option<u32>,
    #[serde(default)]
    bulk_tradable: bool,
    #[serde(default)]
    i18n: HashMap<String, ItemNames>,
}

#[derive(Deserialize)]
struct ItemNames {
    #[serde(default)]
    name: String,
}

/// The listing one collection row becomes on warframe.market.
///
/// `item_id` is what the POST addresses. The other three fields are the contextual ones the API
/// demands exactly when the item supports the dimension and forbids otherwise: the `rank` a mod
/// or arcane is listed at, the `subtype` a relic's refinement is listed under, and the `perTrade`
/// size a bulk-tradable must declare. `None` throughout is a plain listing -- price and quantity
/// and nothing else.
///
/// A `per_trade` of one is the only size this application chooses: it asks nothing of the player
/// and misstates nothing, where a larger size would commit copies to batches the player was never
/// asked about. Batch sizes are the market site's own edit.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Listing<'a> {
    pub item_id: &'a str,
    pub rank: Option<u32>,
    pub subtype: Option<&'a str>,
    pub per_trade: Option<u32>,
}

/// What an `itemId` means, for as many items as warframe.market publishes.
#[derive(Clone, Debug, Default)]
pub struct MarketItems {
    entries: HashMap<String, ItemEntry>,
}

#[derive(Clone, Debug)]
struct ItemEntry {
    catalog_path: Option<String>,
    name: Option<String>,
    /// Whether this item's path can stand for one collection row. See `path_is_comparable`.
    comparable: bool,
    /// The fusion ceiling a ranked listing is measured against. `Some` for every mod and arcane.
    max_rank: Option<u32>,
    /// The subtypes the entry publishes, empty when it has none. A relic's are its four
    /// refinements; anything else here is a variant split the path alone cannot resolve.
    subtypes: Vec<String>,
    /// Whether a listing must declare socketed star counts no collection row knows.
    star_counted: bool,
    /// Whether a listing must declare a per-trade size.
    bulk_tradable: bool,
}

impl MarketItems {
    pub fn from_response(body: &[u8]) -> Result<Self, MarketError> {
        let response =
            serde_json::from_slice::<ItemsResponse>(body).map_err(|_| MarketError::Malformed)?;
        let entries = response
            .data
            .into_iter()
            .map(|record| {
                let name = record
                    .i18n
                    .get("en")
                    .map(|names| names.name.clone())
                    .filter(|name| !name.trim().is_empty());
                // Read before `game_ref` is moved out below.
                let star_counted =
                    record.max_amber_stars.is_some() || record.max_cyan_stars.is_some();
                let catalog_path = record.game_ref.filter(|path| path.starts_with("/Lotus/"));
                let comparable = path_is_comparable(
                    catalog_path.as_deref(),
                    name.as_deref(),
                    record.subtypes.as_deref(),
                );
                let subtypes = record.subtypes.unwrap_or_default();
                (
                    record.id,
                    ItemEntry {
                        catalog_path,
                        name,
                        comparable,
                        max_rank: record.max_rank,
                        subtypes,
                        star_counted,
                        bulk_tradable: record.bulk_tradable,
                    },
                )
            })
            .collect();
        Ok(Self { entries })
    }

    pub fn fetch(transport: &dyn MarketTransport) -> Result<Self, MarketError> {
        let response = transport.send(MarketRequest {
            method: Method::Get,
            url: format!("{API_V2}/items"),
            // Public. Sending the credential would spend the account's identity on a request that
            // does not need it, and on the one request large enough to be worth caching publicly.
            token: None,
            body: None,
        })?;
        match response.status {
            200..=299 if response.body.len() <= MAX_ITEMS_BYTES => {
                Self::from_response(&response.body)
            }
            200..=299 => Err(MarketError::Malformed),
            429 => Err(MarketError::RateLimited),
            _ => Err(MarketError::Unreachable),
        }
    }

    /// The collection's own identity for this order's item, when the market publishes one.
    ///
    /// `None` is a real answer and not a failure: it is what makes an order unverifiable rather
    /// than wrongly flagged.
    pub fn catalog_path(&self, item_id: &str) -> Option<&str> {
        self.entries.get(item_id)?.catalog_path.as_deref()
    }

    /// Whether an owned quantity for this item can be read off the collection at all.
    ///
    /// `false` is not an error and not a missing entry: it says the market's identity for this
    /// item and the collection's identity for a row are not the same kind of thing, so no
    /// comparison between them means anything. The caller owes such an order `Unverifiable`.
    pub fn comparable(&self, item_id: &str) -> bool {
        self.entries
            .get(item_id)
            .is_some_and(|entry| entry.comparable)
    }

    /// The listing a collection row would publish, or `None` when there is none this application
    /// can name honestly.
    ///
    /// `id` is the row's whole key -- a bare path for the unranked stack, `path#rank` for a ranked
    /// copy, a tier-suffixed path for a relic refinement -- because the row, not the sell form, is
    /// what names the copy for sale. `at_max` says the rank in a suffixed id is the card's
    /// ceiling.
    ///
    /// Three kinds of answer come back. A plain item resolves to price and quantity alone. A
    /// ranked item -- every mod and arcane -- resolves with the rank the row names, because
    /// warframe.market quotes a card at rank 0 and at its ceiling only: the unranked stack lists
    /// at 0, a maxed copy at its ceiling, and a copy held part-way up has no rank the API would
    /// accept, so it resolves to nothing rather than to a listing that would be refused. A relic
    /// refinement resolves through the metal tier on its path to the subtype the market expects,
    /// plus the per-trade size every bulk-tradable must declare.
    ///
    /// Still refused, on purpose: an Ayatan sculpture, whose socketed star counts no collection
    /// row knows; and the 19 mods published under `regular`/`atragraph` subtypes with a single
    /// path between them, where the path cannot say which variant is held.
    ///
    /// ponytail: linear scans over a few thousand entries, run once per row when a view is built
    /// and once per sell. A reverse map if a caller ever needs this in a tighter loop.
    pub fn listing_for(&self, id: &str, at_max: bool) -> Option<Listing<'_>> {
        let (path, row_rank) = match id.split_once('#') {
            None => (id, None),
            // A suffix that is not a number names a row the inventory never writes; it is
            // nothing's listing, not the unranked stack's.
            Some((path, suffix)) => (path, Some(suffix.parse::<u32>().ok()?)),
        };
        if let Some((item_id, entry)) = self.entry_for_path(path) {
            return entry.listing(item_id, row_rank, at_max);
        }
        let (base, tier) = refinement_of(path)?;
        let (item_id, entry) = self.entry_for_path(base)?;
        // The subtype is read from the entry's own vocabulary rather than the tier mapping, so
        // the answer can only be a word the market itself publishes -- and a relic-shaped entry
        // whose subtypes are not refinements answers nothing, same as on the exact-match path.
        let subtype = entry
            .subtypes
            .iter()
            .find(|published| published.as_str() == tier)?;
        Some(Listing {
            item_id,
            rank: None,
            subtype: Some(subtype.as_str()),
            per_trade: entry.bulk_tradable.then_some(1),
        })
    }

    /// The entry whose `gameRef` is exactly this path, if the market publishes one.
    fn entry_for_path(&self, path: &str) -> Option<(&str, &ItemEntry)> {
        self.entries
            .iter()
            .find(|(_, entry)| entry.catalog_path.as_deref() == Some(path))
            .map(|(id, entry)| (id.as_str(), entry))
    }

    pub fn name(&self, item_id: &str) -> Option<&str> {
        self.entries.get(item_id)?.name.as_deref()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl ItemEntry {
    /// The listing for a row whose path is this entry's own `gameRef`.
    fn listing<'a>(
        &self,
        item_id: &'a str,
        row_rank: Option<u32>,
        at_max: bool,
    ) -> Option<Listing<'a>> {
        // Star counts are known only to the sculpture itself. Publishing a guess would sell an
        // empty one the player may not be holding.
        if self.star_counted {
            return None;
        }
        // A subtyped entry reached by its own path is a variant split -- the atragraph mods --
        // and the path cannot say which variant the row holds. A relic never reaches here: the
        // collection holds only tier-suffixed paths, never the base one this would match.
        if !self.subtypes.is_empty() {
            return None;
        }
        let rank = match (self.max_rank.is_some(), row_rank, at_max) {
            (false, _, _) => None,
            // The unranked stack lists at rank zero, one of the two ranks the market quotes.
            (true, None, _) => Some(0),
            // A maxed copy lists at its ceiling, the other quoted rank.
            (true, Some(rank), true) => Some(rank),
            // A copy held part-way up is quoted at neither end; there is no listing to publish.
            (true, Some(_), false) => return None,
        };
        Some(Listing {
            item_id,
            rank,
            subtype: None,
            per_trade: self.bulk_tradable.then_some(1),
        })
    }
}

/// The base projection path and the market subtype a relic refinement path names, or `None` for
/// any path that is not one.
///
/// The game writes a relic's refinement as a metal tier on the end of the path; the market
/// publishes it as a lowercase subtype. These four pairs are the whole vocabulary.
fn refinement_of(path: &str) -> Option<(&str, &str)> {
    if !path.contains("/Projections/") {
        return None;
    }
    const TIERS: [(&str, &str); 4] = [
        ("Bronze", "intact"),
        ("Silver", "exceptional"),
        ("Gold", "flawless"),
        ("Platinum", "radiant"),
    ];
    TIERS
        .into_iter()
        .find_map(|(suffix, subtype)| path.strip_suffix(suffix).map(|base| (base, subtype)))
}

/// Whether a market item's `gameRef` names the same thing a collection row names.
///
/// It usually does, and where it does the join is exact. Two published shapes break it, and both
/// were measured against the live table rather than reasoned about:
///
/// A **relic** publishes one entry carrying the base projection path and separates its four
/// refinements by `subtype` -- `Axi A1 Relic` is `/Lotus/Types/Game/Projections/T4VoidProjectionE`
/// with `['intact', 'exceptional', 'flawless', 'radiant']`. The collection holds each refinement
/// as its own row, suffixed (`…T4VoidProjectionEBronze`). The base path is therefore a row the
/// collection never has, and asking whether it is held always answers no.
///
/// A **set** publishes the path of the *built* item -- `Braton Prime Set` is
/// `/Lotus/Weapons/Tenno/Rifle/BratonPrime`. What a player selling a set actually holds is the
/// four `/Lotus/Types/Recipes/…` parts. Again the path names a row the collection does not carry.
///
/// Left unhandled, both answer "owned: 0" and every relic and set listing -- between them most of
/// what a real account sells -- is flagged `Missing` with a button offering to take it down. That
/// is the false accusation the whole unverifiable state exists to prevent, so both are refused
/// here, at the point where the market's own vocabulary is still in view.
///
/// The subtype refusal is deliberately blunt. Measured against the live table, it catches 1,116 of
/// 3,837 items: the 772 relics and 230 sets it is for, plus fish sizes, Ayatan variants,
/// blueprint-against-crafted and veiled rivens, which are all separate collection rows too. The
/// one group it over-refuses is 19 mods whose subtypes are `regular` and `atragraph`, a cosmetic
/// variant of a single row. Judging those would be correct and is left undone on purpose: the cost
/// of refusing one is a row that says nothing, and the cost of a wrong narrowing here is a delete
/// button beside a listing the player still wants.
fn path_is_comparable(path: Option<&str>, name: Option<&str>, subtypes: Option<&[String]>) -> bool {
    let Some(path) = path else { return false };
    if subtypes.is_some_and(|subtypes| !subtypes.is_empty()) {
        return false;
    }
    // Checked as well as the subtype list: a relic entry that ever ships without one is still a
    // base projection path, and the collection still holds only suffixed refinements.
    if path.contains("/Projections/") {
        return false;
    }
    !name.is_some_and(|name| name.trim().ends_with(" Set"))
}
