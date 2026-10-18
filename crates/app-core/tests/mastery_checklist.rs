use std::sync::Arc;

use app_core::{AppCore, CollectionItemView, MasteryCategoryTotal};
use local_store::SnapshotMeta;
use warframe_acquisition::CatalogIndex;
use warframe_domain::{CatalogItem, Category, InventoryEntry, InventorySnapshot, ItemId};

const EXCALIBUR: &str = "/Lotus/Powersuits/Excalibur/Excalibur";
const BRATON: &str = "/Lotus/Weapons/Tenno/Rifle/Braton";
const LATO: &str = "/Lotus/Weapons/Tenno/Pistol/Lato";
const SKANA: &str = "/Lotus/Weapons/Tenno/Melee/LongSword/LongSword";
const FURIS: &str = "/Lotus/Weapons/Tenno/Pistol/Furis";
const SERRATION: &str = "/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod";

/// One frame and four weapons the catalogue calls masterable, and a mod it does not.
const CATALOG: &[u8] = br#"[
  {"uniqueName":"/Lotus/Powersuits/Excalibur/Excalibur","name":"Excalibur","type":"Warframe","category":"Warframes","masterable":true},
  {"uniqueName":"/Lotus/Weapons/Tenno/Rifle/Braton","name":"Braton","type":"Rifle","category":"Primary","masterable":true,"imageName":"Braton.png"},
  {"uniqueName":"/Lotus/Weapons/Tenno/Pistol/Lato","name":"Lato","type":"Pistol","category":"Secondary","masterable":true},
  {"uniqueName":"/Lotus/Weapons/Tenno/Melee/LongSword/LongSword","name":"Skana","type":"Melee","category":"Melee","masterable":true},
  {"uniqueName":"/Lotus/Weapons/Tenno/Pistol/Furis","name":"Furis","type":"Pistol","category":"Secondary","masterable":true},
  {"uniqueName":"/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod","name":"Serration","type":"Rifle Mod","category":"Mods","masterable":false,"fusionLimit":10}
]"#;

fn catalog() -> Arc<CatalogIndex> {
    Arc::new(CatalogIndex::from_wfcd_json(CATALOG).expect("catalog parses"))
}

fn item(id: &str, name: &str, category: Category, quantity: u32) -> InventoryEntry {
    InventoryEntry::new(
        CatalogItem::new(ItemId::new(id).expect("valid id"), name, category).expect("valid item"),
        quantity,
    )
}

fn synced_core(entries: Vec<InventoryEntry>) -> AppCore {
    let mut core = AppCore::in_memory().expect("in-memory core");
    core.apply_inventory_snapshot(
        InventorySnapshot::coherent(entries).expect("coherent snapshot"),
        SnapshotMeta::fake("synced").expect("meta"),
    )
    .expect("snapshot applies");
    core
}

fn by_id<'a>(items: &'a [CollectionItemView], id: &str) -> &'a CollectionItemView {
    items
        .iter()
        .find(|item| item.id() == id)
        .unwrap_or_else(|| panic!("{id} is listed"))
}

fn total(category: Category, mastered: usize, total: usize) -> MasteryCategoryTotal {
    MasteryCategoryTotal {
        category,
        mastered,
        total,
    }
}

fn ids(items: &[CollectionItemView]) -> Vec<&str> {
    items.iter().map(CollectionItemView::id).collect()
}

/// The checklist is what is left to master: equipment held but unranked, and equipment never
/// held. Mastered items are progress, held or sold, and the collection is the store alone again.
#[test]
fn the_checklist_lists_what_the_account_has_never_mastered() {
    let mut core = synced_core(vec![
        item(BRATON, "Braton", Category::Weapon, 1).with_mastered(true),
        // Mastered, then sold: progress the profile keeps, not a row for the list.
        item(LATO, "Lato", Category::Weapon, 0).with_mastered(true),
        item(SKANA, "Skana", Category::Weapon, 1),
        item(SERRATION, "Serration", Category::Mod, 12),
    ]);
    core.set_mastery_catalog(catalog());
    core.set_reads_inventory(true);

    let view = core.current_view().expect("view builds");

    let mastery = view.mastery();
    assert!(mastery.available());
    assert_eq!(
        ids(mastery.items()),
        [EXCALIBUR, SKANA, FURIS],
        "never held, held but unranked, never held; sorted by id"
    );
    assert!(mastery.items().iter().all(|item| !item.mastered()));
    assert_eq!(
        by_id(mastery.items(), SKANA).quantity(),
        1,
        "a held item keeps its quantity"
    );
    assert_eq!(by_id(mastery.items(), EXCALIBUR).quantity(), 0);
    assert_eq!(by_id(mastery.items(), FURIS).platinum(), None);
    assert_eq!(
        mastery.totals().to_vec(),
        vec![
            total(Category::Frame, 0, 1),
            total(Category::Weapon, 2, 4),
            total(Category::Companion, 0, 0),
            total(Category::Vehicle, 0, 0),
        ],
        "Braton and the sold Lato are mastered; totals are the catalogue's masterable set"
    );

    assert_eq!(
        ids(view.collection().items()),
        [SERRATION, SKANA, LATO, BRATON],
        "the collection is the store alone: no catalogue rows"
    );
    assert_eq!(view.collection().total_entries(), 4);
}

/// Sold before it was ever mastered: still to do, listed once at quantity 0 rather than twice
/// (the store's row and a catalogue row), and counted once in the category total.
#[test]
fn a_sold_unmastered_item_is_still_on_the_checklist_once() {
    let mut core = synced_core(vec![item(SKANA, "Skana", Category::Weapon, 0)]);
    core.set_mastery_catalog(catalog());
    core.set_reads_inventory(true);

    let mastery = core.current_view().expect("view builds").mastery().clone();

    assert_eq!(
        ids(mastery.items()),
        [EXCALIBUR, SKANA, FURIS, LATO, BRATON]
    );
    assert_eq!(by_id(mastery.items(), SKANA).quantity(), 0);
    assert_eq!(
        mastery.totals()[1],
        total(Category::Weapon, 0, 4),
        "four weapons in the catalogue, Skana among them exactly once"
    );
}

/// Before any inventory has been read, "never mastered" would mean "unknown".
#[test]
fn the_checklist_is_unavailable_before_an_inventory_has_been_read() {
    let mut core = AppCore::in_memory().expect("in-memory core");
    core.set_mastery_catalog(catalog());
    core.set_reads_inventory(true);

    let view = core.current_view().expect("view builds");

    assert!(!view.mastery().available());
    assert!(view.mastery().items().is_empty());
    assert!(view.mastery().totals().is_empty());
}

/// Companion and Overlay show a saved snapshot; a snapshot does not get to say what the account
/// has never mastered. The collection still shows what the snapshot holds.
#[test]
fn the_checklist_is_unavailable_without_inventory_access() {
    let mut core = synced_core(vec![
        item(BRATON, "Braton", Category::Weapon, 1).with_mastered(true),
    ]);
    core.set_mastery_catalog(catalog());

    let view = core.current_view().expect("view builds");

    assert!(!view.mastery().available());
    assert!(view.mastery().items().is_empty());
    assert!(view.mastery().totals().is_empty());
    assert_eq!(ids(view.collection().items()), [BRATON]);
}
