use std::sync::Arc;

use app_core::{AppCore, AppView, CollectionItemView};
use local_store::SnapshotMeta;
use warframe_acquisition::CatalogIndex;
use warframe_domain::{CatalogItem, Category, InventoryEntry, InventorySnapshot, ItemId};

const BRATON: &str = "/Lotus/Weapons/Tenno/Rifle/Braton";
const LATO: &str = "/Lotus/Weapons/Tenno/Pistol/Lato";
const SKANA: &str = "/Lotus/Weapons/Tenno/Melee/LongSword/LongSword";
const SERRATION: &str = "/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod";

/// Two weapons the account has records for, one it has never touched, and a mod that earns
/// no mastery. Lato has no artwork on purpose.
const CATALOG: &[u8] = br#"[
  {"uniqueName":"/Lotus/Weapons/Tenno/Rifle/Braton","name":"Braton","type":"Rifle","category":"Primary","masterable":true,"imageName":"Braton.png"},
  {"uniqueName":"/Lotus/Weapons/Tenno/Pistol/Lato","name":"Lato","type":"Pistol","category":"Secondary","masterable":true},
  {"uniqueName":"/Lotus/Weapons/Tenno/Melee/LongSword/LongSword","name":"Skana","type":"Melee","category":"Melee","masterable":true,"imageName":"Skana.png"},
  {"uniqueName":"/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod","name":"Serration","type":"Rifle Mod","category":"Mods","masterable":false,"fusionLimit":10}
]"#;

fn catalog() -> Arc<CatalogIndex> {
    Arc::new(CatalogIndex::from_wfcd_json(CATALOG).expect("catalog parses"))
}

fn item(id: &str, name: &str, quantity: u32) -> InventoryEntry {
    InventoryEntry::new(
        CatalogItem::new(ItemId::new(id).expect("valid id"), name, Category::Weapon)
            .expect("valid item"),
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

fn by_id<'a>(view: &'a AppView, id: &str) -> Option<&'a CollectionItemView> {
    view.collection()
        .items()
        .iter()
        .find(|item| item.id() == id)
}

#[test]
fn equipment_the_account_has_never_ranked_is_listed_as_missing_and_unmastered() {
    let mut core = synced_core(vec![
        item(BRATON, "Braton", 1).with_mastered(true),
        // Mastered, then sold: the decoder keeps it on record at zero.
        item(LATO, "Lato", 0).with_mastered(true),
    ]);
    core.set_mastery_catalog(catalog());
    core.set_reads_inventory(true);

    let view = core.current_view().expect("view builds");

    let skana = by_id(&view, SKANA)
        .expect("the catalogue's Skana is listed though the account never held one");
    assert_eq!(skana.name(), "Skana");
    assert_eq!(skana.category(), Category::Weapon);
    assert_eq!(skana.quantity(), 0);
    assert!(!skana.mastered());
    assert_eq!(
        skana.image_url(),
        Some("https://raw.githubusercontent.com/WFCD/warframe-items/master/data/img/Skana.png")
    );
    assert_eq!(skana.platinum(), None, "a quantity-0 row is not a sale");
    assert!(!skana.priceable());

    assert_eq!(
        view.collection()
            .items()
            .iter()
            .filter(|item| item.id() == LATO)
            .count(),
        1,
        "the account's own zero-quantity row is not doubled by the catalogue"
    );
    let lato = by_id(&view, LATO).expect("Lato stays listed");
    assert!(lato.mastered(), "a mastered-and-sold item keeps its mark");
    assert_eq!(
        lato.image_url(),
        None,
        "no artwork in the catalogue, none invented"
    );

    assert!(
        by_id(&view, SERRATION).is_none(),
        "a mod earns no mastery and is not padded in"
    );
    assert_eq!(
        view.collection().total_entries(),
        3,
        "Braton, Lato and the padded Skana"
    );
}

/// Before any inventory has been read, "missing" would mean "unknown", and the page would be the
/// catalogue wearing the account's name.
#[test]
fn nothing_is_padded_before_an_inventory_has_been_read() {
    let mut core = AppCore::in_memory().expect("in-memory core");
    core.set_mastery_catalog(catalog());
    core.set_reads_inventory(true);

    let view = core.current_view().expect("view builds");

    assert!(
        view.collection().items().is_empty(),
        "{:?}",
        view.collection().items()
    );
    assert_eq!(view.collection().total_entries(), 0);
}

/// Companion and Overlay access show a saved snapshot and call every row a saved record, with the
/// ownership filters hidden. Padding is a claim about the account as it stands, and only the mode
/// that reads the inventory gets to make it.
#[test]
fn nothing_is_padded_when_the_access_mode_cannot_read_the_inventory() {
    let mut core = synced_core(vec![item(BRATON, "Braton", 1).with_mastered(true)]);
    core.set_mastery_catalog(catalog());

    let view = core.current_view().expect("view builds");

    let ids: Vec<&str> = view
        .collection()
        .items()
        .iter()
        .map(|item| item.id())
        .collect();
    assert_eq!(
        ids,
        [BRATON],
        "a snapshot shown without inventory access is only itself"
    );
}
