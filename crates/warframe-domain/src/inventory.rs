use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::{CatalogItem, DomainError, ItemId};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct InventoryEntry {
    pub item: CatalogItem,
    pub quantity: u32,
    pub mastered: bool,
}

impl InventoryEntry {
    pub fn new(item: CatalogItem, quantity: u32) -> Self {
        Self {
            item,
            quantity,
            mastered: false,
        }
    }

    pub fn with_mastered(mut self, mastered: bool) -> Self {
        self.mastered = mastered;
        self
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

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct Collection {
    entries: BTreeMap<ItemId, InventoryEntry>,
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
