use local_store::{SnapshotMeta, SqliteStore};
use warframe_domain::{CatalogItem, Category, InventoryEntry, InventorySnapshot, ItemId};

fn entry(
    id: &str,
    name: &str,
    category: Category,
    quantity: u32,
    mastered: bool,
) -> InventoryEntry {
    InventoryEntry::new(
        CatalogItem::new(ItemId::new(id).unwrap(), name, category).unwrap(),
        quantity,
    )
    .with_mastered(mastered)
}

fn snapshot(entries: Vec<InventoryEntry>) -> InventorySnapshot {
    InventorySnapshot::coherent(entries).unwrap()
}

#[test]
fn authoritative_replacement_removes_absent_items_and_appends_audit_rows() {
    let mut store = SqliteStore::in_memory().unwrap();
    store
        .replace_collection(
            &snapshot(vec![
                entry("paris", "Paris", Category::Weapon, 3, true),
                entry("lex", "Lex", Category::Weapon, 2, false),
            ]),
            &SnapshotMeta::fake("build-1").unwrap(),
        )
        .unwrap();
    store
        .replace_collection(
            &snapshot(vec![entry("paris", "Paris", Category::Weapon, 1, true)]),
            &SnapshotMeta::fake("build-2").unwrap(),
        )
        .unwrap();

    let collection = store.load_collection().unwrap();
    assert_eq!(collection.quantity(&ItemId::new("paris").unwrap()), 1);
    assert_eq!(collection.quantity(&ItemId::new("lex").unwrap()), 0);
    assert_eq!(collection.entries().count(), 1);
    assert_eq!(store.audit_count().unwrap(), 2);
}

#[test]
fn all_categories_mastery_and_zero_quantity_round_trip() {
    let fixtures = [
        ("excalibur", "Excalibur", Category::Frame),
        ("paris", "Paris", Category::Weapon),
        ("carrier", "Carrier", Category::Companion),
        ("lex_prime_barrel", "Lex Prime Barrel", Category::PrimePart),
        ("lith_a1", "Lith A1", Category::Relic),
        ("argon", "Argon Crystal", Category::Resource),
        ("lex_blueprint", "Lex Blueprint", Category::Blueprint),
    ];
    let snapshot = snapshot(
        fixtures
            .into_iter()
            .enumerate()
            .map(|(index, (id, name, category))| {
                entry(
                    id,
                    name,
                    category,
                    u32::try_from(index).unwrap(),
                    index != 1,
                )
            })
            .collect(),
    );
    let mut store = SqliteStore::in_memory().unwrap();
    store
        .replace_collection(&snapshot, &SnapshotMeta::fake("build").unwrap())
        .unwrap();

    let collection = store.load_collection().unwrap();
    assert_eq!(collection.entries().count(), fixtures.len());
    for (index, (id, _, category)) in fixtures.into_iter().enumerate() {
        let loaded = collection
            .entries()
            .find(|entry| entry.item.id == ItemId::new(id).unwrap())
            .unwrap();
        assert_eq!(loaded.item.category, category);
        assert_eq!(loaded.quantity, u32::try_from(index).unwrap());
        assert_eq!(loaded.mastered, index != 1);
    }
}

#[test]
fn file_backed_store_persists_across_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("inventory.sqlite3");
    {
        let mut store = SqliteStore::open(&path).unwrap();
        store
            .replace_collection(
                &snapshot(vec![entry("lex", "Lex", Category::Weapon, 4, false)]),
                &SnapshotMeta::fake("build").unwrap(),
            )
            .unwrap();
    }

    let store = SqliteStore::open(&path).unwrap();
    assert_eq!(
        store
            .load_collection()
            .unwrap()
            .quantity(&ItemId::new("lex").unwrap()),
        4
    );
    assert_eq!(store.audit_count().unwrap(), 1);
}

#[test]
fn snapshot_metadata_rejects_blank_fields() {
    assert!(SnapshotMeta::new(" ".into(), "build".into(), "test".into()).is_err());
    assert!(SnapshotMeta::new("now".into(), "\t".into(), "test".into()).is_err());
    assert!(SnapshotMeta::new("now".into(), "build".into(), "\n".into()).is_err());
    assert!(SnapshotMeta::fake("  ").is_err());
}
