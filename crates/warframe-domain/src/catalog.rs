use std::fmt;

use serde::{Deserialize, Serialize};
use thiserror::Error;

#[derive(Debug, Error, PartialEq)]
pub enum DomainError {
    #[error("item ID must not be blank")]
    InvalidItemId,
    #[error("name must not be blank")]
    InvalidName,
    #[error("confidence must be finite and between 0.0 and 1.0")]
    InvalidConfidence,
    #[error("snapshot contains duplicate item ID: {0}")]
    DuplicateItemId(ItemId),
}

#[derive(Clone, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct ItemId(String);

impl ItemId {
    pub fn new(value: impl Into<String>) -> Result<Self, DomainError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(DomainError::InvalidItemId);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ItemId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Category {
    Frame,
    Weapon,
    Companion,
    PrimePart,
    Relic,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CatalogItem {
    pub id: ItemId,
    pub name: String,
    pub category: Category,
}

impl CatalogItem {
    pub fn new(
        id: ItemId,
        name: impl Into<String>,
        category: Category,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DomainError::InvalidName);
        }
        Ok(Self { id, name, category })
    }
}
