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
                (
                    record.id,
                    ItemEntry {
                        catalog_path: record.game_ref.filter(|path| path.starts_with("/Lotus/")),
                        name,
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
