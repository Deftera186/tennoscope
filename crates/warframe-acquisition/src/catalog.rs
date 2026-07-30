use std::collections::{BTreeMap, btree_map::Entry};

use serde::Deserialize;
use thiserror::Error;
use warframe_domain::Category;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CatalogMetadata {
    name: String,
    category: Option<Category>,
    masterable: bool,
    max_rank: u32,
    image_name: Option<String>,
    ducats: u32,
}

impl CatalogMetadata {
    pub fn name(&self) -> &str {
        &self.name
    }

    pub const fn category(&self) -> Option<Category> {
        self.category
    }

    pub const fn masterable(&self) -> bool {
        self.masterable
    }

    pub const fn max_rank(&self) -> u32 {
        self.max_rank
    }

    pub fn image_name(&self) -> Option<&str> {
        self.image_name.as_deref()
    }

    pub const fn ducats(&self) -> u32 {
        self.ducats
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum CatalogError {
    #[error("catalog JSON was invalid")]
    InvalidJson,
    #[error("catalog contained invalid or conflicting metadata")]
    InvalidMetadata,
}

#[derive(Clone, Debug, Default)]
pub struct CatalogIndex {
    items: BTreeMap<String, CatalogMetadata>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewardCatalogEntry {
    pub name: String,
    pub ducats: u32,
}

/// Does this catalog entry name the item the reward screen is offering?
///
/// The reward screen, the relic drop tables and warframe.market all name a Warframe part by the
/// blueprint the player picks up -- "Lavos Prime Chassis Blueprint". WFCD's item catalog names the
/// component that blueprint builds, "Lavos Prime Chassis", because the suffix lives in the parent's
/// component list rather than in the component's own name. Nothing else separates the two spellings.
///
/// Matching on the exact string priced 153 of the 596 names a relic can drop -- every Warframe
/// part -- at zero ducats and reported them as not owned, while weapon parts, whose two spellings
/// agree, were right. The trim only ever runs after an exact match fails, so items that really are
/// named "... Blueprint" in both vocabularies (every prime's own blueprint, Forma) still match
/// themselves first.
pub fn reward_name_matches(catalog_name: &str, reward_name: &str) -> bool {
    catalog_name == reward_name || reward_name.strip_suffix(" Blueprint") == Some(catalog_name)
}

impl CatalogIndex {
    pub fn from_wfcd_json(bytes: &[u8]) -> Result<Self, CatalogError> {
        let raw: Vec<WfcdItem> =
            serde_json::from_slice(bytes).map_err(|_| CatalogError::InvalidJson)?;
        let mut index = Self::default();

        for item in &raw {
            // WFCD's aggregate also contains star-chart node records whose IDs are not
            // inventory paths. They are useful to other consumers, but not to this index.
            if !valid_unique_name(&item.unique_name) {
                continue;
            }
            let category = classify_item(item);
            index.insert(
                &item.unique_name,
                CatalogMetadata {
                    name: validated_name(&item.name)?,
                    category,
                    masterable: item.masterable && category.is_some_and(is_equipment),
                    max_rank: catalog_max_rank(&item.name),
                    image_name: validated_image_name(item.image_name.as_deref())?,
                    ducats: item.ducats.unwrap_or(0),
                },
                false,
            )?;
        }

        for parent in &raw {
            if !is_prime_parent(&parent.name) {
                continue;
            }
            for component in &parent.components {
                if !component.tradable
                    || (component.ducats.is_none() && component.prime_selling_price.is_none())
                {
                    continue;
                }
                let component_name = validated_name(&component.name)?;
                let name = if component_name.contains("Prime") {
                    component_name
                } else {
                    format!("{} {component_name}", validated_name(&parent.name)?)
                };
                index.insert(
                    &component.unique_name,
                    CatalogMetadata {
                        name,
                        category: Some(Category::PrimePart),
                        masterable: false,
                        max_rank: 0,
                        image_name: validated_image_name(component.image_name.as_deref())?,
                        ducats: component.ducats.unwrap_or(0),
                    },
                    true,
                )?;
            }
        }
        Ok(index)
    }

    pub fn resolve(&self, unique_name: &str) -> Option<&CatalogMetadata> {
        self.items.get(unique_name)
    }

    pub fn reward_entries(&self) -> Vec<RewardCatalogEntry> {
        self.items
            .values()
            .filter(|metadata| {
                metadata.category == Some(Category::PrimePart) || metadata.name == "Forma Blueprint"
            })
            .map(|metadata| RewardCatalogEntry {
                name: metadata.name.clone(),
                ducats: metadata.ducats,
            })
            .collect()
    }

    pub fn paths_for_name(&self, name: &str) -> Vec<String> {
        self.items
            .iter()
            .filter(|(_, metadata)| metadata.name == name)
            .map(|(path, _)| path.clone())
            .collect()
    }

    fn insert(
        &mut self,
        unique_name: &str,
        metadata: CatalogMetadata,
        richer_component_context: bool,
    ) -> Result<(), CatalogError> {
        if !valid_unique_name(unique_name) {
            return Err(CatalogError::InvalidMetadata);
        }
        match self.items.entry(unique_name.to_owned()) {
            Entry::Vacant(entry) => {
                entry.insert(metadata);
            }
            Entry::Occupied(entry) if entry.get() == &metadata => {}
            Entry::Occupied(mut entry)
                if richer_component_context
                    && entry.get().category != Some(Category::PrimePart) =>
            {
                entry.insert(metadata);
            }
            Entry::Occupied(_) => return Err(CatalogError::InvalidMetadata),
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WfcdItem {
    unique_name: String,
    name: String,
    #[serde(rename = "type", default)]
    item_type: String,
    #[serde(default)]
    category: String,
    #[serde(default)]
    masterable: bool,
    #[serde(default)]
    components: Vec<WfcdComponent>,
    #[serde(default)]
    image_name: Option<String>,
    #[serde(default)]
    ducats: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WfcdComponent {
    unique_name: String,
    name: String,
    #[serde(default)]
    tradable: bool,
    #[serde(default)]
    ducats: Option<u32>,
    #[serde(default)]
    prime_selling_price: Option<u32>,
    #[serde(default)]
    image_name: Option<String>,
}

fn classify_item(item: &WfcdItem) -> Option<Category> {
    let category = item.category.to_ascii_lowercase();
    let item_type = item.item_type.to_ascii_lowercase();
    let path = item.unique_name.as_str();
    if item_type == "k-drive component" {
        Some(Category::Vehicle)
    } else if category.contains("warframe")
        || matches!(item_type.as_str(), "warframe" | "archwing" | "necramech")
    {
        Some(Category::Frame)
    } else if category.contains("companion")
        || category == "pets"
        || matches!(
            item_type.as_str(),
            "sentinel" | "kubrow" | "kavat" | "moa" | "pets"
        )
        || path.contains("/Types/Game/CatbrowPet/")
        || path.contains("/Types/Friendly/Catbrow")
        || path.contains("/Types/Friendly/Pets/")
    {
        Some(Category::Companion)
    } else if matches!(
        category.as_str(),
        "primary" | "secondary" | "melee" | "archwing" | "arch-gun" | "arch-melee"
    ) || matches!(
        item_type.as_str(),
        "rifle"
            | "shotgun"
            | "bow"
            | "pistol"
            | "melee"
            | "archgun"
            | "archmelee"
            | "arch-gun"
            | "arch-melee"
            | "kitgun component"
            | "zaw component"
    ) {
        Some(Category::Weapon)
    } else if category.contains("relic") || item_type == "relic" {
        Some(Category::Relic)
    } else if category.contains("resource") || item_type == "resource" {
        Some(Category::Resource)
    } else if category.contains("blueprint") || item_type == "blueprint" {
        Some(Category::Blueprint)
    } else {
        None
    }
}

fn is_equipment(category: Category) -> bool {
    matches!(
        category,
        Category::Frame | Category::Weapon | Category::Companion | Category::Vehicle
    )
}

fn catalog_max_rank(name: &str) -> u32 {
    if name == "Paracesis"
        || matches!(name, "Voidrig" | "Bonewidow")
        || name.starts_with("Kuva ")
        || name.starts_with("Tenet ")
        || name.starts_with("Coda ")
    {
        40
    } else {
        30
    }
}

fn is_prime_parent(name: &str) -> bool {
    name.ends_with(" Prime") || name.contains(" Prime ")
}

fn validated_name(name: &str) -> Result<String, CatalogError> {
    // The Archon shards carry the game's own inline icon tag in their name --
    // "<Shard_red_simple> Crimson Archon Shard". Only the game's own text renderer draws that tag;
    // everywhere else it survives as literal angle brackets.
    let trimmed = name
        .strip_prefix('<')
        .and_then(|rest| rest.split_once('>'))
        .map_or(name, |(_, rest)| rest)
        .trim();
    if trimmed.is_empty() || trimmed.len() > 256 {
        return Err(CatalogError::InvalidMetadata);
    }
    Ok(trimmed.to_owned())
}

fn validated_image_name(image_name: Option<&str>) -> Result<Option<String>, CatalogError> {
    Ok(image_name.and_then(|value| {
        let value = value.trim();
        (!value.is_empty()
            && value.len() <= 256
            && !value.contains('/')
            && !value.contains('\\')
            && value.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b' ')
            }))
        .then(|| base_shard_art(value))
    }))
}

/// WFCD publishes a Tauforged shard's art as the glow layer alone -- the halo the game composites
/// over the plain crystal -- so on its own it renders as a coloured smudge. Dropping the `Mythic`
/// and `Glow` tokens names the base shard art, the same crystal without the halo, which every
/// colour ships. The two spellings WFCD uses, `ArchonShardBorealMythicGlow` and
/// `ArchonShardMythicGreenGlow`, both reduce correctly.
fn base_shard_art(image_name: &str) -> String {
    if image_name.starts_with("ArchonShard")
        && image_name.ends_with("Glow.png")
        && image_name.contains("Mythic")
    {
        return image_name.replace("Mythic", "").replace("Glow", "");
    }
    image_name.to_owned()
}

fn valid_unique_name(path: &str) -> bool {
    path.starts_with("/Lotus/") && path.len() <= 512 && !path.ends_with('/')
}
