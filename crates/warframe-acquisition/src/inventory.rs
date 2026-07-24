use std::{collections::BTreeMap, io::Read, time::Duration};

use reqwest::{blocking::Client, redirect::Policy};
use serde::Deserialize;
use url::Url;
use warframe_domain::{CatalogItem, Category, InventoryEntry, InventorySnapshot, ItemId};

use crate::{
    AcquisitionError, CatalogIndex, InventoryAuthorization, InventoryTransport, SnapshotDecoder,
};

pub const INVENTORY_ENDPOINT: &str = "https://mobile.warframe.com/api/inventory.php";
pub const MAX_INVENTORY_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const TOTAL_TIMEOUT: Duration = Duration::from_secs(20);

/// Blocking HTTPS adapter for the one pinned Warframe inventory endpoint.
///
/// Redirects are rejected so credentials in the query cannot be forwarded to
/// another origin. The returned bytes exist only in the caller's memory.
pub struct InventoryHttpTransport {
    client: Client,
}

impl InventoryHttpTransport {
    pub fn new() -> Result<Self, AcquisitionError> {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TOTAL_TIMEOUT)
            .redirect(Policy::none())
            .build()
            .map_err(|_| AcquisitionError::InventoryRequestFailed)?;
        Ok(Self { client })
    }

    pub const fn connect_timeout(&self) -> Duration {
        CONNECT_TIMEOUT
    }

    pub const fn total_timeout(&self) -> Duration {
        TOTAL_TIMEOUT
    }

    pub const fn response_limit(&self) -> usize {
        MAX_INVENTORY_RESPONSE_BYTES
    }

    pub const fn follows_redirects(&self) -> bool {
        false
    }
}

impl InventoryTransport for InventoryHttpTransport {
    fn fetch(&self, authorization: &InventoryAuthorization) -> Result<Vec<u8>, AcquisitionError> {
        let mut url =
            Url::parse(INVENTORY_ENDPOINT).map_err(|_| AcquisitionError::InventoryRequestFailed)?;
        if url.scheme() != "https" || url.host_str() != Some("mobile.warframe.com") {
            return Err(AcquisitionError::InventoryRequestFailed);
        }
        url.query_pairs_mut()
            .append_pair("accountId", authorization.account_id())
            .append_pair("nonce", authorization.nonce());

        let response = self
            .client
            .get(url)
            .send()
            .map_err(|_| AcquisitionError::InventoryRequestFailed)?;
        if !response.status().is_success() {
            return Err(AcquisitionError::InventoryRequestFailed);
        }

        read_bounded(response, MAX_INVENTORY_RESPONSE_BYTES)
    }
}

fn read_bounded(reader: impl Read, limit: usize) -> Result<Vec<u8>, AcquisitionError> {
    let limit_u64 = u64::try_from(limit).map_err(|_| AcquisitionError::InventoryRequestFailed)?;
    let mut bytes = Vec::new();
    reader
        .take(limit_u64.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| AcquisitionError::InventoryRequestFailed)?;
    if bytes.len() > limit {
        return Err(AcquisitionError::InventoryResponseTooLarge);
    }
    Ok(bytes)
}

#[derive(Default)]
pub struct InventoryJsonDecoder<'a> {
    catalog: Option<&'a CatalogIndex>,
}

impl<'a> InventoryJsonDecoder<'a> {
    pub const fn with_catalog(catalog: &'a CatalogIndex) -> Self {
        Self {
            catalog: Some(catalog),
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawEntry {
    item_type: String,
    #[serde(default)]
    item_count: Option<i64>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawXpEntry {
    item_type: String,
    #[serde(rename = "XP")]
    xp: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawInventory {
    last_inventory_sync: serde_json::Value,
    suits: Vec<RawEntry>,
    long_guns: Vec<RawEntry>,
    pistols: Vec<RawEntry>,
    melee: Vec<RawEntry>,
    sentinels: Vec<RawEntry>,
    misc_items: Vec<RawEntry>,
    recipes: Vec<RawEntry>,
    pending_recipes: Vec<RawEntry>,
    #[serde(rename = "XPInfo")]
    xp_info: Vec<RawXpEntry>,
    space_suits: Vec<RawEntry>,
    space_melee: Vec<RawEntry>,
    space_guns: Vec<RawEntry>,
    sentinel_weapons: Vec<RawEntry>,
    kubrow_pets: Vec<RawEntry>,
    operator_amps: Vec<RawEntry>,
    mech_suits: Vec<RawEntry>,
}

struct AccumulatedEntry {
    name: Option<String>,
    category: Category,
    quantity: i64,
    mastered: bool,
    masterable: bool,
    max_rank: Option<u32>,
}

impl SnapshotDecoder for InventoryJsonDecoder<'_> {
    fn decode(&self, response: &[u8]) -> Result<InventorySnapshot, AcquisitionError> {
        let raw: RawInventory =
            serde_json::from_slice(response).map_err(|_| AcquisitionError::SnapshotInvalid)?;
        if raw.last_inventory_sync.is_null() {
            return Err(AcquisitionError::SnapshotInvalid);
        }
        let mut entries = BTreeMap::<String, AccumulatedEntry>::new();

        add_unique_section(&mut entries, raw.suits, Category::Frame, self.catalog)?;
        add_unique_section(&mut entries, raw.long_guns, Category::Weapon, self.catalog)?;
        add_unique_section(&mut entries, raw.pistols, Category::Weapon, self.catalog)?;
        add_unique_section(&mut entries, raw.melee, Category::Weapon, self.catalog)?;
        add_unique_section(
            &mut entries,
            raw.sentinels,
            Category::Companion,
            self.catalog,
        )?;
        add_misc_section(&mut entries, raw.misc_items, self.catalog)?;
        add_stackable_section(
            &mut entries,
            raw.recipes,
            Category::Blueprint,
            1,
            self.catalog,
        )?;
        add_stackable_section(
            &mut entries,
            raw.pending_recipes,
            Category::Blueprint,
            -1,
            self.catalog,
        )?;
        add_unique_section(&mut entries, raw.space_suits, Category::Frame, self.catalog)?;
        add_unique_section(
            &mut entries,
            raw.space_melee,
            Category::Weapon,
            self.catalog,
        )?;
        add_unique_section(&mut entries, raw.space_guns, Category::Weapon, self.catalog)?;
        add_unique_section(
            &mut entries,
            raw.sentinel_weapons,
            Category::Weapon,
            self.catalog,
        )?;
        add_unique_section(
            &mut entries,
            raw.kubrow_pets,
            Category::Companion,
            self.catalog,
        )?;
        add_unique_section(
            &mut entries,
            raw.operator_amps,
            Category::Weapon,
            self.catalog,
        )?;
        add_unique_section(&mut entries, raw.mech_suits, Category::Frame, self.catalog)?;

        for xp in raw.xp_info {
            validate_item_type(&xp.item_type)?;
            let path = xp.item_type;
            let fallback = category_from_path(&path);
            let entry = entries
                .entry(path.clone())
                .or_insert_with(|| accumulated_entry(&path, fallback, self.catalog));
            if entry.masterable
                && entry
                    .max_rank
                    .and_then(|rank| mastery_threshold(entry.category, rank))
                    .is_some_and(|threshold| xp.xp >= threshold)
            {
                entry.mastered = true;
            }
        }

        let domain_entries = entries
            .into_iter()
            .filter(|(_, accumulated)| accumulated.quantity >= 0)
            .map(|(path, accumulated)| {
                let name = accumulated.name.unwrap_or(display_label(&path)?);
                let id = ItemId::new(path).map_err(|_| AcquisitionError::SnapshotInvalid)?;
                let item = CatalogItem::new(id, name, accumulated.category)
                    .map_err(|_| AcquisitionError::SnapshotInvalid)?;
                let quantity = u32::try_from(accumulated.quantity)
                    .map_err(|_| AcquisitionError::SnapshotInvalid)?;
                Ok(InventoryEntry::new(item, quantity).with_mastered(accumulated.mastered))
            })
            .collect::<Result<Vec<_>, AcquisitionError>>()?;
        InventorySnapshot::coherent(domain_entries).map_err(|_| AcquisitionError::SnapshotInvalid)
    }
}

fn add_misc_section(
    output: &mut BTreeMap<String, AccumulatedEntry>,
    items: Vec<RawEntry>,
    catalog: Option<&CatalogIndex>,
) -> Result<(), AcquisitionError> {
    for item in items {
        let category = if item.item_type.contains("/Projections/") {
            Category::Relic
        } else {
            Category::Resource
        };
        add_stackable_item(output, item, category, 1, catalog)?;
    }
    Ok(())
}

fn add_unique_section(
    output: &mut BTreeMap<String, AccumulatedEntry>,
    items: Vec<RawEntry>,
    category: Category,
    catalog: Option<&CatalogIndex>,
) -> Result<(), AcquisitionError> {
    for item in items {
        validate_item_type(&item.item_type)?;
        let path = item.item_type;
        let resolved = accumulated_entry(&path, category, catalog);
        let entry = output.entry(path).or_insert(resolved);
        entry.quantity = entry
            .quantity
            .checked_add(1)
            .ok_or(AcquisitionError::SnapshotInvalid)?;
    }
    Ok(())
}

fn add_stackable_section(
    output: &mut BTreeMap<String, AccumulatedEntry>,
    items: Vec<RawEntry>,
    category: Category,
    direction: i64,
    catalog: Option<&CatalogIndex>,
) -> Result<(), AcquisitionError> {
    for item in items {
        add_stackable_item(output, item, category, direction, catalog)?;
    }
    Ok(())
}

fn add_stackable_item(
    output: &mut BTreeMap<String, AccumulatedEntry>,
    item: RawEntry,
    category: Category,
    direction: i64,
    catalog: Option<&CatalogIndex>,
) -> Result<(), AcquisitionError> {
    validate_item_type(&item.item_type)?;
    let quantity = item.item_count.unwrap_or(1);
    if quantity < 0 {
        return Err(AcquisitionError::SnapshotInvalid);
    }
    if quantity == 0 {
        return Ok(());
    }
    let path = item.item_type;
    let resolved = accumulated_entry(&path, category, catalog);
    let entry = output.entry(path).or_insert(resolved);
    let delta = quantity
        .checked_mul(direction)
        .ok_or(AcquisitionError::SnapshotInvalid)?;
    entry.quantity = entry
        .quantity
        .checked_add(delta)
        .ok_or(AcquisitionError::SnapshotInvalid)?;
    Ok(())
}

fn validate_item_type(path: &str) -> Result<(), AcquisitionError> {
    if path.starts_with("/Lotus/")
        && path.len() <= 512
        && path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-'))
        && path
            .rsplit('/')
            .next()
            .is_some_and(|segment| !segment.is_empty())
    {
        Ok(())
    } else {
        Err(AcquisitionError::SnapshotInvalid)
    }
}

fn category_from_path(path: &str) -> Category {
    if path.contains("/Projections/") {
        Category::Relic
    } else if path.contains("/Powersuits/") || path.contains("/MechSuits/") {
        Category::Frame
    } else if path.contains("/Weapons/")
        || path.contains("/SentinelWeapons/")
        || path.contains("/OperatorAmp")
    {
        Category::Weapon
    } else if path.contains("/Sentinel") || path.contains("/Kubrow") || path.contains("/Pets/") {
        Category::Companion
    } else if path.contains("/Recipes/") {
        Category::Blueprint
    } else {
        Category::Resource
    }
}

fn mastery_threshold(category: Category, max_rank: u32) -> Option<u64> {
    let max_rank = u64::from(max_rank);
    let affinity_per_rank_squared = match category {
        Category::Frame | Category::Companion | Category::Vehicle => 1_000_u64,
        Category::Weapon => 500_u64,
        Category::PrimePart | Category::Relic | Category::Resource | Category::Blueprint => {
            return None;
        }
    };
    affinity_per_rank_squared
        .checked_mul(max_rank)?
        .checked_mul(max_rank)
}

fn accumulated_entry(
    path: &str,
    fallback_category: Category,
    catalog: Option<&CatalogIndex>,
) -> AccumulatedEntry {
    let metadata = catalog.and_then(|catalog| catalog.resolve(path));
    AccumulatedEntry {
        name: metadata.map(|metadata| metadata.name().to_owned()),
        category: metadata.map_or(fallback_category, |metadata| metadata.category()),
        quantity: 0,
        mastered: false,
        masterable: metadata.is_some_and(|metadata| metadata.masterable()),
        max_rank: metadata.map(|metadata| metadata.max_rank()),
    }
}

fn display_label(path: &str) -> Result<String, AcquisitionError> {
    let segment = path
        .rsplit('/')
        .next()
        .ok_or(AcquisitionError::SnapshotInvalid)?;
    let chars: Vec<char> = segment.chars().collect();
    let mut label = String::with_capacity(segment.len() + 8);
    for (index, &current) in chars.iter().enumerate() {
        let previous = index.checked_sub(1).and_then(|i| chars.get(i)).copied();
        let next = chars.get(index + 1).copied();
        let boundary = previous.is_some_and(|previous| {
            (previous.is_ascii_lowercase() && current.is_ascii_uppercase())
                || (previous.is_ascii_alphabetic() && current.is_ascii_digit())
                || (previous.is_ascii_digit() && current.is_ascii_alphabetic())
                || (previous.is_ascii_uppercase()
                    && current.is_ascii_uppercase()
                    && next.is_some_and(|next| next.is_ascii_lowercase()))
                || previous == '_'
                || previous == '-'
        });
        if boundary && !label.ends_with(' ') {
            label.push(' ');
        }
        if !matches!(current, '_' | '-') {
            label.push(current);
        }
    }
    let label = label.trim().to_owned();
    (!label.is_empty())
        .then_some(label)
        .ok_or(AcquisitionError::SnapshotInvalid)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::read_bounded;
    use crate::AcquisitionError;

    #[test]
    fn bounded_reader_accepts_complete_body_at_limit() {
        assert_eq!(read_bounded(Cursor::new(b"1234"), 4).unwrap(), b"1234");
    }

    #[test]
    fn bounded_reader_rejects_body_one_byte_over_limit() {
        assert_eq!(
            read_bounded(Cursor::new(b"12345"), 4),
            Err(AcquisitionError::InventoryResponseTooLarge)
        );
    }
}
