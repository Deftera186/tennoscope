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
    /// Where a ranked copy records the rank it was fused to, as JSON inside a JSON string.
    #[serde(default)]
    upgrade_fingerprint: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawXpEntry {
    item_type: String,
    #[serde(rename = "XP")]
    xp: u64,
}

/// One inventory response.
///
/// Every section is defaulted *and* holds raw rows rather than decoded ones. `inventory.php` omits
/// a section entirely when the account holds nothing in it -- no Necramech means no `MechSuits` --
/// and it also emits rows the game's own client refuses: a Steam Deck report carried
/// `Inventory has NULL item` in Warframe's `EE.log` against the very response we were reading, and
/// one `"ItemType": null` failed the entire account's snapshot. The client skips such a row and
/// carries on, so we do too; each row is converted in `add_*` where a failure costs that row only.
///
/// The sync marker stays required because it is what distinguishes an inventory response from any
/// other JSON the endpoint could return; a payload that decodes to no holdings at all is refused in
/// `decode` rather than here.
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawInventory {
    last_inventory_sync: serde_json::Value,
    #[serde(default, deserialize_with = "rows")]
    suits: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "rows")]
    long_guns: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "rows")]
    pistols: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "rows")]
    melee: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "rows")]
    sentinels: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "rows")]
    misc_items: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "rows")]
    recipes: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "rows")]
    pending_recipes: Vec<serde_json::Value>,
    #[serde(rename = "XPInfo", default, deserialize_with = "rows")]
    xp_info: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "rows")]
    space_suits: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "rows")]
    space_melee: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "rows")]
    space_guns: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "rows")]
    sentinel_weapons: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "rows")]
    kubrow_pets: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "rows")]
    operator_amps: Vec<serde_json::Value>,
    #[serde(default, deserialize_with = "rows")]
    mech_suits: Vec<serde_json::Value>,
    /// Unranked mods and arcanes, stacked by type. The largest holding in the response by value.
    #[serde(default, deserialize_with = "rows")]
    raw_upgrades: Vec<serde_json::Value>,
    /// Mods and arcanes carrying a rank, plus rivens: one row per copy, no `ItemCount`.
    #[serde(default, deserialize_with = "rows")]
    upgrades: Vec<serde_json::Value>,
    /// Ayatan sculptures and stars.
    #[serde(default, deserialize_with = "rows")]
    fusion_treasures: Vec<serde_json::Value>,
    /// Built Railjack armaments.
    #[serde(default, deserialize_with = "rows")]
    crew_ship_weapons: Vec<serde_json::Value>,
}

/// A section, tolerating the two shapes that are not a list of rows.
///
/// `null` is not the same as absent to serde -- `#[serde(default)]` covers a missing key and
/// nothing else -- and the endpoint emits both. Anything that is neither a list nor null is a
/// section we did not understand, and reads as empty rather than failing its nineteen siblings.
fn rows<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Vec<serde_json::Value>, D::Error> {
    Ok(match serde_json::Value::deserialize(deserializer)? {
        serde_json::Value::Array(rows) => rows,
        _ => Vec::new(),
    })
}

/// What a decode had to throw away, so a failed read can be explained without the payload.
///
/// Counts and paths only: the response is one account's holdings, and the credential that fetched
/// it never reaches here at all.
#[derive(Default)]
struct DecodeReport {
    rows_seen: usize,
    rows_skipped: usize,
    first_skip: Option<String>,
}

impl DecodeReport {
    fn skip(&mut self, reason: &str, row: &serde_json::Value) {
        self.rows_skipped += 1;
        if self.first_skip.is_none() {
            self.first_skip = Some(format!("{reason} ({})", row_label(row)));
        }
    }

    fn summary(&self) -> String {
        let first = self.first_skip.as_deref().unwrap_or("none");
        format!(
            "rows_seen={} rows_skipped={} first_skip={first}",
            self.rows_seen, self.rows_skipped
        )
    }
}

/// A skipped row named by its `ItemType` alone.
///
/// The rest of a row can carry a riven's rolled stats or a trade's counterparty, so only the item
/// path -- which is game content, identical for every account that owns one -- is ever quoted.
fn row_label(row: &serde_json::Value) -> String {
    match row.get("ItemType").and_then(serde_json::Value::as_str) {
        Some(path) if path.len() <= 128 => format!("ItemType={path}"),
        Some(_) => "ItemType=<overlong>".to_owned(),
        None => "ItemType absent".to_owned(),
    }
}

/// One row, or the reason it cannot be one.
fn parse_entry(row: serde_json::Value, report: &mut DecodeReport) -> Option<RawEntry> {
    report.rows_seen += 1;
    match serde_json::from_value::<RawEntry>(row.clone()) {
        Ok(entry) if validate_item_type(&entry.item_type) => Some(entry),
        Ok(_) => {
            report.skip("item path is not canonical", &row);
            None
        }
        Err(_) => {
            report.skip("row does not have an item type", &row);
            None
        }
    }
}

struct AccumulatedEntry {
    /// The catalogue path these copies came from, which is the map key for everything except a
    /// ranked mod -- that one is keyed per rank, and still has to look its artwork and name up here.
    path: String,
    name: Option<String>,
    category: Category,
    quantity: i64,
    mastered: bool,
    masterable: bool,
    max_rank: Option<u32>,
    rank: Option<u32>,
    fusion_limit: Option<u32>,
    image_name: Option<String>,
}

impl SnapshotDecoder for InventoryJsonDecoder<'_> {
    fn decode(&self, response: &[u8]) -> Result<InventorySnapshot, AcquisitionError> {
        let raw: RawInventory =
            serde_json::from_slice(response).map_err(|_| AcquisitionError::SnapshotInvalid)?;
        if raw.last_inventory_sync.is_null() {
            return Err(AcquisitionError::SnapshotInvalid);
        }
        let mut entries = BTreeMap::<String, AccumulatedEntry>::new();
        let report = &mut DecodeReport::default();

        add_unique_section(
            &mut entries,
            raw.suits,
            Category::Frame,
            self.catalog,
            report,
        );
        add_unique_section(
            &mut entries,
            raw.long_guns,
            Category::Weapon,
            self.catalog,
            report,
        );
        add_unique_section(
            &mut entries,
            raw.pistols,
            Category::Weapon,
            self.catalog,
            report,
        );
        add_unique_section(
            &mut entries,
            raw.melee,
            Category::Weapon,
            self.catalog,
            report,
        );
        add_unique_section(
            &mut entries,
            raw.sentinels,
            Category::Companion,
            self.catalog,
            report,
        );
        add_misc_section(&mut entries, raw.misc_items, self.catalog, report);
        add_stackable_section(
            &mut entries,
            raw.recipes,
            Category::Blueprint,
            1,
            self.catalog,
            report,
        );
        add_stackable_section(
            &mut entries,
            raw.pending_recipes,
            Category::Blueprint,
            -1,
            self.catalog,
            report,
        );
        add_unique_section(
            &mut entries,
            raw.space_suits,
            Category::Frame,
            self.catalog,
            report,
        );
        add_unique_section(
            &mut entries,
            raw.space_melee,
            Category::Weapon,
            self.catalog,
            report,
        );
        add_unique_section(
            &mut entries,
            raw.space_guns,
            Category::Weapon,
            self.catalog,
            report,
        );
        add_unique_section(
            &mut entries,
            raw.sentinel_weapons,
            Category::Weapon,
            self.catalog,
            report,
        );
        add_unique_section(
            &mut entries,
            raw.kubrow_pets,
            Category::Companion,
            self.catalog,
            report,
        );
        add_unique_section(
            &mut entries,
            raw.operator_amps,
            Category::Weapon,
            self.catalog,
            report,
        );
        add_unique_section(
            &mut entries,
            raw.mech_suits,
            Category::Frame,
            self.catalog,
            report,
        );
        add_upgrade_section(&mut entries, raw.raw_upgrades, self.catalog, report);
        // One row per copy and no `ItemCount`. Copies that carry a rank get a row per rank rather
        // than joining the unranked stack, because the market prices the ranks separately.
        add_upgrade_section(&mut entries, raw.upgrades, self.catalog, report);
        add_stackable_section(
            &mut entries,
            raw.fusion_treasures,
            Category::Resource,
            1,
            self.catalog,
            report,
        );
        add_unique_section(
            &mut entries,
            raw.crew_ship_weapons,
            Category::Weapon,
            self.catalog,
            report,
        );

        for row in raw.xp_info {
            report.rows_seen += 1;
            let Ok(xp) = serde_json::from_value::<RawXpEntry>(row.clone()) else {
                report.skip("row does not have an item type and XP", &row);
                continue;
            };
            if !validate_item_type(&xp.item_type) {
                report.skip("item path is not canonical", &row);
                continue;
            }
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
            .filter_map(|(path, accumulated)| {
                build_entry(path, accumulated).or_else(|| {
                    report.rows_skipped += 1;
                    None
                })
            })
            .collect::<Vec<_>>();
        // Every section being optional is what lets a young account read at all, but it also means
        // a response whose shape we stopped understanding would decode quietly to nothing. No
        // logged-in account owns nothing, so an empty snapshot is a failed read, not a poor one.
        if domain_entries.is_empty() {
            trace_decode(&format!("[decode] rejected empty: {}", report.summary()));
            return Err(AcquisitionError::SnapshotInvalid);
        }
        if report.rows_skipped != 0 {
            trace_decode(&format!("[decode] tolerated: {}", report.summary()));
        }
        InventorySnapshot::coherent(domain_entries).map_err(|_| AcquisitionError::SnapshotInvalid)
    }
}

/// One accumulated holding as a domain entry, or nothing if it cannot become one.
fn build_entry(path: String, accumulated: AccumulatedEntry) -> Option<InventoryEntry> {
    let name = match accumulated.name {
        Some(name) => name,
        None => display_label(&accumulated.path)?,
    };
    let item = CatalogItem::new(ItemId::new(path).ok()?, name, accumulated.category).ok()?;
    let item = match accumulated.image_name {
        Some(image_name) => item.with_image_name(image_name).ok()?,
        None => item,
    };
    let entry = InventoryEntry::new(item, u32::try_from(accumulated.quantity).ok()?)
        .with_mastered(accumulated.mastered);
    Some(match accumulated.rank {
        Some(rank) => entry.with_rank(rank, accumulated.fusion_limit),
        None => entry,
    })
}

/// One line about what a decode had to throw away.
///
/// A failed read reaches the player as "Inventory snapshot was invalid" and reaches us as nothing
/// at all: the serde error was discarded at the parse, so the one report that mattered could not
/// be answered from the app's own output. This is counts and item paths only -- see `row_label`.
fn trace_decode(line: &str) {
    #[cfg(debug_assertions)]
    crate::append_debug_line(line);
    #[cfg(not(debug_assertions))]
    let _ = line;
}

fn add_misc_section(
    output: &mut BTreeMap<String, AccumulatedEntry>,
    rows: Vec<serde_json::Value>,
    catalog: Option<&CatalogIndex>,
    report: &mut DecodeReport,
) {
    for row in rows {
        let Some(item) = parse_entry(row, report) else {
            continue;
        };
        let category = if item.item_type.contains("/Projections/") {
            Category::Relic
        } else {
            Category::Resource
        };
        add_stackable_item(output, item, category, 1, catalog, report);
    }
}

/// Mods, arcanes and rivens, whether they carry a rank or not.
///
/// Both sections mix the two kinds, and only the path tells them apart -- WFCD files an arcane
/// under `/Upgrades/CosmeticEnhancers/`. The fallback is what an unresolved path gets: 12 of this
/// account's 1,011 rows are newer than the cached catalog, and "a mod" is a better guess for them
/// than "a resource".
fn add_upgrade_section(
    output: &mut BTreeMap<String, AccumulatedEntry>,
    rows: Vec<serde_json::Value>,
    catalog: Option<&CatalogIndex>,
    report: &mut DecodeReport,
) {
    for row in rows {
        let Some(item) = parse_entry(row, report) else {
            continue;
        };
        let category = if item.item_type.contains("/CosmeticEnhancers/") {
            Category::Arcane
        } else {
            Category::Mod
        };
        match fused_rank(item.upgrade_fingerprint.as_deref()) {
            0 => add_stackable_item(output, item, category, 1, catalog, report),
            rank => add_ranked_copy(output, item.item_type, category, rank, catalog),
        }
    }
}

/// The rank a copy was fused to, from the fingerprint the game stores it in.
///
/// The fingerprint is a JSON document inside a JSON string, and an unranked copy omits `lvl`
/// entirely rather than writing zero. A riven's fingerprint carries its rolled stats in the same
/// field, so this has to read one key rather than assume a shape.
fn fused_rank(fingerprint: Option<&str>) -> u32 {
    fingerprint
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
        .and_then(|parsed| parsed.get("lvl")?.as_u64())
        .and_then(|rank| u32::try_from(rank).ok())
        .unwrap_or(0)
}

/// One fused copy, on a row of its own for its rank.
///
/// Ranks are not a detail of the same holding: `Serration` sells for 3p unranked and 48p at rank
/// 10, and the two are separate listings on warframe.market. Summing them onto one row forces one
/// price onto both, and the only price a merged row can honestly show is the lower.
fn add_ranked_copy(
    output: &mut BTreeMap<String, AccumulatedEntry>,
    path: String,
    category: Category,
    rank: u32,
    catalog: Option<&CatalogIndex>,
) {
    let mut resolved = accumulated_entry(&path, category, catalog);
    resolved.rank = Some(rank);
    // The key, not the path: the path is what the catalogue is asked about, and several rows share
    // it. `#` cannot occur in an item path, which `validate_item_type` has already established.
    let entry = output.entry(format!("{path}#{rank}")).or_insert(resolved);
    entry.quantity = entry.quantity.saturating_add(1);
}

fn add_unique_section(
    output: &mut BTreeMap<String, AccumulatedEntry>,
    rows: Vec<serde_json::Value>,
    category: Category,
    catalog: Option<&CatalogIndex>,
    report: &mut DecodeReport,
) {
    for row in rows {
        let Some(item) = parse_entry(row, report) else {
            continue;
        };
        let path = item.item_type;
        let resolved = accumulated_entry(&path, category, catalog);
        let entry = output.entry(path).or_insert(resolved);
        entry.quantity = entry.quantity.saturating_add(1);
    }
}

fn add_stackable_section(
    output: &mut BTreeMap<String, AccumulatedEntry>,
    rows: Vec<serde_json::Value>,
    category: Category,
    direction: i64,
    catalog: Option<&CatalogIndex>,
    report: &mut DecodeReport,
) {
    for row in rows {
        let Some(item) = parse_entry(row, report) else {
            continue;
        };
        add_stackable_item(output, item, category, direction, catalog, report);
    }
}

fn add_stackable_item(
    output: &mut BTreeMap<String, AccumulatedEntry>,
    item: RawEntry,
    category: Category,
    direction: i64,
    catalog: Option<&CatalogIndex>,
    report: &mut DecodeReport,
) {
    let quantity = item.item_count.unwrap_or(1);
    if quantity < 0 {
        report.skip(
            "negative count",
            &serde_json::json!({"ItemType": item.item_type}),
        );
        return;
    }
    if quantity == 0 {
        return;
    }
    let path = item.item_type;
    let resolved = accumulated_entry(&path, category, catalog);
    let entry = output.entry(path).or_insert(resolved);
    entry.quantity = entry
        .quantity
        .saturating_add(quantity.saturating_mul(direction));
}

/// Whether a path is one the game could have written.
///
/// A row failing this is a row we cannot key, name, or price, so it is dropped -- but the account
/// around it is untouched. Rejecting the snapshot over one such path is what a real read hit.
fn validate_item_type(path: &str) -> bool {
    path.starts_with("/Lotus/")
        && path.len() <= 512
        && path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'_' | b'-'))
        && path
            .rsplit('/')
            .next()
            .is_some_and(|segment| !segment.is_empty())
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
        Category::PrimePart
        | Category::Relic
        | Category::Resource
        | Category::Blueprint
        | Category::Mod
        | Category::Arcane => {
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
        path: path.to_owned(),
        name: metadata.map(|metadata| metadata.name().to_owned()),
        category: metadata
            .and_then(|metadata| metadata.category())
            .unwrap_or(fallback_category),
        quantity: 0,
        mastered: false,
        masterable: metadata.is_some_and(|metadata| metadata.masterable()),
        max_rank: metadata.map(|metadata| metadata.max_rank()),
        rank: None,
        fusion_limit: metadata.and_then(|metadata| metadata.fusion_limit()),
        image_name: metadata
            .and_then(|metadata| metadata.image_name())
            .map(str::to_owned),
    }
}

fn display_label(path: &str) -> Option<String> {
    let segment = path.rsplit('/').next()?;
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
    (!label.is_empty()).then_some(label)
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
