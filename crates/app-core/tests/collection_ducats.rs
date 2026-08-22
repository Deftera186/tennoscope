use std::sync::Arc;

use app_core::AppCore;
use local_store::SnapshotMeta;
use warframe_acquisition::{CatalogIndex, DucatTable};
use warframe_domain::{CatalogItem, Category, InventoryEntry, InventorySnapshot, ItemId};

const CATALOG: &[u8] = br#"[
  {"uniqueName":"/Lotus/Weapons/Tenno/Bows/PrimeHuntingBow","name":"Paris Prime","type":"Bow","category":"Primary",
   "components":[{"uniqueName":"/Lotus/Types/Recipes/Weapons/WeaponParts/PrimeBowString","name":"String","tradable":true,"ducats":15,"primeSellingPrice":15}]},
  {"uniqueName":"/Lotus/Weapons/Tenno/Rifle/Braton","name":"Braton","type":"Rifle","category":"Primary","masterable":true,"components":[]}
]"#;

fn item(id: &str, name: &str, category: Category, quantity: u32) -> InventoryEntry {
    InventoryEntry::new(
        CatalogItem::new(ItemId::new(id).expect("valid id"), name, category).expect("valid item"),
        quantity,
    )
}

fn core_with_items(entries: Vec<InventoryEntry>) -> AppCore {
    let mut core = AppCore::in_memory().expect("in-memory core");
    core.apply_inventory_snapshot(
        InventorySnapshot::coherent(entries).expect("coherent snapshot"),
        SnapshotMeta::fake("build").expect("meta"),
    )
    .expect("snapshot applies");
    core
}

fn ducats_from_catalog() -> Arc<DucatTable> {
    Arc::new(
        CatalogIndex::from_wfcd_json(CATALOG)
            .expect("catalog parses")
            .ducat_table(),
    )
}

fn named(core: &AppCore, name: &str) -> app_core::CollectionItemView {
    core.current_view()
        .expect("view builds")
        .collection()
        .items()
        .iter()
        .find(|item| item.name() == name)
        .cloned()
        .expect("item is in the collection")
}

#[test]
fn a_prime_part_carries_its_ducats_into_the_view() {
    let mut core = core_with_items(vec![
        item(
            "/Lotus/Types/Recipes/Weapons/WeaponParts/PrimeBowString",
            "Paris Prime String",
            Category::PrimePart,
            3,
        ),
        item(
            "/Lotus/Weapons/Tenno/Rifle/Braton",
            "Braton",
            Category::Weapon,
            1,
        ),
    ]);
    core.set_collection_ducats(ducats_from_catalog());

    assert_eq!(
        named(&core, "Paris Prime String").ducats(),
        Some(15),
        "joined by catalog path, as enrichment already joins names and artwork"
    );
    assert_eq!(
        named(&core, "Braton").ducats(),
        None,
        "plain equipment holds no ducats"
    );
}

/// Platinum is hidden on a quantity-0 row because it describes a sale, and a sale needs a copy.
/// Ducats describe the item: the number on a missing part is exactly what tells the player which
/// relic reward to take, so it stays.
#[test]
fn a_prime_part_the_player_does_not_own_still_carries_its_ducats() {
    let mut core = core_with_items(vec![item(
        "/Lotus/Types/Recipes/Weapons/WeaponParts/PrimeBowString",
        "Paris Prime String",
        Category::PrimePart,
        0,
    )]);
    core.set_collection_ducats(ducats_from_catalog());

    let missing = named(&core, "Paris Prime String");
    assert_eq!(missing.platinum(), None, "unowned, so unsellable");
    assert_eq!(missing.ducats(), Some(15), "but Baro's price for it stands");
}

/// Before the catalogue loads, and for anything it does not list, the view says nothing rather
/// than zero -- zero ducats would read as "worthless" for a part that is merely not yet described.
#[test]
fn a_view_built_before_the_catalog_loads_has_no_ducats() {
    let core = core_with_items(vec![item(
        "/Lotus/Types/Recipes/Weapons/WeaponParts/PrimeBowString",
        "Paris Prime String",
        Category::PrimePart,
        3,
    )]);

    assert_eq!(named(&core, "Paris Prime String").ducats(), None);
}
