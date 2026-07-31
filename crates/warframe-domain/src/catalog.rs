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

    /// The catalogue path behind this id: the whole id, unless it names a rank.
    ///
    /// A mod or arcane held at several ranks is several holdings -- the market prices rank 0 and
    /// the ceiling separately -- so each gets a row keyed `<path>#<rank>`. The catalogue knows
    /// only the path, and anything asking it about one of those rows has to ask about this or be
    /// told the item does not exist.
    pub fn catalog_path(&self) -> &str {
        self.0.split('#').next().unwrap_or(&self.0)
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
    Resource,
    Blueprint,
    Vehicle,
    Mod,
    Arcane,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct CatalogItem {
    pub id: ItemId,
    pub name: String,
    pub category: Category,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image_name: Option<String>,
}

#[derive(Deserialize)]
struct CatalogItemDto {
    id: ItemId,
    name: String,
    category: Category,
    #[serde(default)]
    image_name: Option<String>,
}

impl<'de> Deserialize<'de> for CatalogItem {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let dto = CatalogItemDto::deserialize(deserializer)?;
        let item = Self::new(dto.id, dto.name, dto.category).map_err(D::Error::custom)?;
        match dto.image_name {
            Some(image_name) => item.with_image_name(image_name).map_err(D::Error::custom),
            None => Ok(item),
        }
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
        Ok(Self {
            id,
            name,
            category,
            image_name: None,
        })
    }

    pub fn with_image_name(mut self, image_name: impl Into<String>) -> Result<Self, DomainError> {
        let image_name = image_name.into();
        if image_name.trim().is_empty()
            || image_name.len() > 256
            || image_name.contains('/')
            || image_name.contains('\\')
            || !image_name.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b' ')
            })
        {
            return Err(DomainError::InvalidName);
        }
        self.image_name = Some(image_name);
        Ok(self)
    }
}
