use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::{CatalogItem, DomainError, ItemId};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InventoryEntry {
    pub item: CatalogItem,
    pub quantity: u32,
    pub mastered: bool,
    /// The rank every copy on this entry carries, for the things that have one: mods, arcanes and
    /// rivens. `None` for everything else and for the unranked stack, which is the default state
    /// and the tier the market quotes by default.
    ///
    /// A rank is not cosmetic. `Serration` sells for 3p at rank 0 and 48p at rank 10, so copies at
    /// different ranks are different holdings and get an entry each -- which is why this is on the
    /// entry rather than on the catalog item, whose identity is the same card either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rank: Option<u32>,
    /// The highest rank this card can reach, where it is known. Absent for a riven, whose published
    /// limit is a sentinel rather than a rank.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_rank: Option<u32>,
}

impl InventoryEntry {
    pub fn new(item: CatalogItem, quantity: u32) -> Self {
        Self {
            item,
            quantity,
            mastered: false,
            rank: None,
            max_rank: None,
        }
    }

    pub fn with_mastered(mut self, mastered: bool) -> Self {
        self.mastered = mastered;
        self
    }

    pub fn with_rank(mut self, rank: u32, max_rank: Option<u32>) -> Self {
        self.rank = Some(rank);
        self.max_rank = max_rank;
        self
    }

    /// Whether these copies are fully ranked. `None` when the ceiling is unknown, which is not the
    /// same answer as "no" and must not be collapsed into one: an unknown ceiling means the market
    /// quote for a maxed copy cannot be claimed for this one.
    pub fn at_max_rank(&self) -> Option<bool> {
        Some(self.rank.unwrap_or(0) >= self.max_rank?)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct InventorySnapshot {
    entries: Vec<InventoryEntry>,
}

#[derive(Deserialize)]
struct InventorySnapshotDto {
    entries: Vec<InventoryEntry>,
}

impl<'de> Deserialize<'de> for InventorySnapshot {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let dto = InventorySnapshotDto::deserialize(deserializer)?;
        Self::coherent(dto.entries).map_err(D::Error::custom)
    }
}

impl InventorySnapshot {
    pub fn coherent(entries: Vec<InventoryEntry>) -> Result<Self, DomainError> {
        let mut ids = BTreeSet::new();
        for entry in &entries {
            if !ids.insert(entry.item.id.clone()) {
                return Err(DomainError::DuplicateItemId(entry.item.id.clone()));
            }
        }
        Ok(Self { entries })
    }

    pub fn entries(&self) -> &[InventoryEntry] {
        &self.entries
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Collection {
    entries: BTreeMap<ItemId, InventoryEntry>,
}

#[derive(Deserialize)]
struct CollectionDto {
    entries: BTreeMap<ItemId, InventoryEntry>,
}

impl<'de> Deserialize<'de> for Collection {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let dto = CollectionDto::deserialize(deserializer)?;
        let mut logical_ids = BTreeSet::new();
        for (key, entry) in &dto.entries {
            if key != &entry.item.id {
                return Err(D::Error::custom(format_args!(
                    "collection key {key} does not match item ID {}",
                    entry.item.id
                )));
            }
            if !logical_ids.insert(entry.item.id.clone()) {
                return Err(D::Error::custom(format_args!(
                    "collection contains duplicate item ID: {}",
                    entry.item.id
                )));
            }
        }
        Ok(Self {
            entries: dto.entries,
        })
    }
}

impl Collection {
    /// Replaces the complete collection; entries absent from the snapshot are removed.
    pub fn replace(&mut self, snapshot: InventorySnapshot) {
        self.entries = snapshot
            .entries
            .into_iter()
            .map(|entry| (entry.item.id.clone(), entry))
            .collect();
    }

    pub fn quantity(&self, id: &ItemId) -> u32 {
        self.entries.get(id).map_or(0, |entry| entry.quantity)
    }

    pub fn entries(&self) -> impl ExactSizeIterator<Item = &InventoryEntry> {
        self.entries.values()
    }
}
