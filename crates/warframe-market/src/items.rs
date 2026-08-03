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
    /// Whether a listing for it can be published with price and quantity alone. See
    /// `is_plainly_listable`.
    plainly_listable: bool,
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
                let plain_shape = is_plainly_shaped(&record);
                let catalog_path = record.game_ref.filter(|path| path.starts_with("/Lotus/"));
                let comparable = path_is_comparable(
                    catalog_path.as_deref(),
                    name.as_deref(),
                    record.subtypes.as_deref(),
                );
                let plainly_listable = comparable && plain_shape;
                (
                    record.id,
                    ItemEntry {
                        catalog_path,
                        name,
                        comparable,
                        plainly_listable,
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

    /// The market's id for a collection path, for the one direction selling needs.
    ///
    /// Answers only for items a listing can be published for with price and quantity alone. That
    /// is a narrower question than `comparable`, which asks whether an owned count can be read off
    /// the collection -- and the two were conflated until a rare mod refused to list.
    ///
    /// ponytail: linear scan over a few thousand entries, run once per sell. A reverse map if a
    /// caller ever needs this in a loop.
    pub fn market_id_for_path(&self, catalog_path: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(_, entry)| {
                entry.plainly_listable && entry.catalog_path.as_deref() == Some(catalog_path)
            })
            .map(|(id, _)| id.as_str())
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

/// Whether an item is shaped so a listing needs price and quantity alone.
///
/// `POST /v2/order` takes contextual fields that are required exactly when the item supports the
/// dimension and forbidden otherwise, and a 400 comes back either way. `maxRank` means the body
/// must carry a `rank`; `maxAmberStars` and `maxCyanStars` mean an Ayatan sculpture wants its star
/// counts; `bulkTradable` means `perTrade`. The sell form asks for price and quantity and nothing
/// else, so it can only publish for items that need nothing else.
///
/// This was `comparable` until a rare mod refused to list. Measured against the live table, 1,487
/// of the 2,721 comparable items carry a `maxRank` -- every mod and arcane in the game -- so the
/// screen was offering a Sell button on more than half of what it could actually publish, and the
/// failure arrived as a flat "could not publish" with no reason attached. Comparability answers
/// whether an owned count can be read off the collection, which is a different question and stays
/// as it is: those mods are still reconciled, still flagged, still removable.
///
/// The upgrade path is the sell form growing a rank field, which would recover the largest group
/// by far. Left undone here because the fix owed today is that the button stops lying.
fn is_plainly_shaped(record: &ItemRecord) -> bool {
    record.max_rank.is_none()
        && record.max_amber_stars.is_none()
        && record.max_cyan_stars.is_none()
        && !record.bulk_tradable
}
