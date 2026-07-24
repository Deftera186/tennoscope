use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::error::DomainError;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
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

impl<'de> Deserialize<'de> for ItemId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Self::new(String::deserialize(deserializer)?).map_err(D::Error::custom)
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CatalogItem {
    pub id: ItemId,
    pub name: String,
    pub category: Category,
}

#[derive(Deserialize)]
struct CatalogItemDto {
    id: ItemId,
    name: String,
    category: Category,
}

impl<'de> Deserialize<'de> for CatalogItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let dto = CatalogItemDto::deserialize(deserializer)?;
        Self::new(dto.id, dto.name, dto.category).map_err(D::Error::custom)
    }
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
