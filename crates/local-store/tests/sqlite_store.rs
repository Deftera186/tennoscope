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
fn catalog_metadata_update_preserves_quantity_and_snapshot_audit() {
    let mut store = SqliteStore::in_memory().unwrap();
    store
        .replace_collection(
            &snapshot(vec![entry(
                "alertium",
                "Alertium",
                Category::Resource,
                7,
                false,
            )]),
            &SnapshotMeta::fake("before").unwrap(),
        )
        .unwrap();
    let enriched = snapshot(vec![InventoryEntry::new(
        CatalogItem::new(
            ItemId::new("alertium").unwrap(),
            "Nitain Extract",
            Category::Resource,
        )
        .unwrap()
        .with_image_name("Alertium.png")
        .unwrap(),
        999,
    )]);

    store.update_collection_metadata(&enriched).unwrap();

    let collection = store.load_collection().unwrap();
    let item = collection.entries().next().unwrap();
    assert_eq!(item.quantity, 7);
    assert_eq!(item.item.name, "Nitain Extract");
    assert_eq!(item.item.image_name.as_deref(), Some("Alertium.png"));
    assert_eq!(store.audit_count().unwrap(), 1);
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
        ("bad_baby", "Bad Baby", Category::Vehicle),
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

#[test]
fn latest_snapshot_metadata_round_trips_and_empty_store_has_none() {
    let mut store = SqliteStore::in_memory().unwrap();
    assert_eq!(store.latest_snapshot_meta().unwrap(), None);
    let meta = SnapshotMeta::new(
        "2026-07-25T08:09:10Z".into(),
        "build-42".into(),
        "warframe-memory".into(),
    )
    .unwrap();
    store.replace_collection(&snapshot(vec![]), &meta).unwrap();

    assert_eq!(store.latest_snapshot_meta().unwrap(), Some(meta));
}

/// A rank has to survive the database or the row split means nothing: both rows come back
/// unranked, resolve to the same market listing, and are handed the same price -- which is exactly
/// what a maxed `Arcane Reaper` showing an unranked one's 15p looked like.
#[test]
fn a_ranked_row_keeps_its_rank_and_ceiling_across_a_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.sqlite3");
    let mut store = SqliteStore::open(&path).unwrap();
    store
        .replace_collection(
            &snapshot(vec![
                entry("serration", "Serration", Category::Mod, 3, false),
                entry("serration#10", "Serration", Category::Mod, 1, false).with_rank(10, Some(10)),
                // A riven's ceiling is unpublished, and an absent one must stay absent rather than
                // come back as zero, which would read as a maxed card.
                entry("riven#3", "Rifle Riven Mod", Category::Mod, 1, false).with_rank(3, None),
            ]),
            &SnapshotMeta::fake("build").unwrap(),
        )
        .unwrap();
    drop(store);

    let reopened = SqliteStore::open(&path).unwrap();
    let collection = reopened.load_collection().unwrap();
    let ranks: Vec<_> = collection
        .entries()
        .map(|entry| {
            (
                entry.item.id.as_str().to_owned(),
                entry.rank,
                entry.max_rank,
            )
        })
        .collect();

    assert_eq!(
        ranks,
        vec![
            ("riven#3".to_owned(), Some(3), None),
            ("serration".to_owned(), None, None),
            ("serration#10".to_owned(), Some(10), Some(10)),
        ]
    );
}

/// A database written before ranks existed must open and keep its rows. Refusing it would throw
/// away a collection over a column, and rebuilding it costs a game session.
#[test]
fn a_database_from_before_ranks_migrates_with_its_rows_intact() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("store.sqlite3");
    {
        let connection = rusqlite::Connection::open(&path).unwrap();
        connection
            .execute_batch(
                "CREATE TABLE inventory (
                    item_id TEXT PRIMARY KEY,
                    name TEXT NOT NULL,
                    category TEXT NOT NULL,
                    quantity INTEGER NOT NULL CHECK (quantity >= 0),
                    mastered INTEGER NOT NULL CHECK (mastered IN (0, 1)),
                    image_name TEXT
                 );
                 CREATE TABLE snapshot_audit (
                    id INTEGER PRIMARY KEY AUTOINCREMENT,
                    observed_at TEXT NOT NULL CHECK (length(trim(observed_at)) > 0),
                    game_build TEXT NOT NULL CHECK (length(trim(game_build)) > 0),
                    source TEXT NOT NULL CHECK (length(trim(source)) > 0),
                    item_count INTEGER NOT NULL CHECK (item_count >= 0)
                 );
                 INSERT INTO inventory (item_id, name, category, quantity, mastered, image_name)
                 VALUES ('paris', 'Paris', 'weapon', 2, 1, NULL);
                 PRAGMA user_version = 2;",
            )
            .unwrap();
    }

    let store = SqliteStore::open(&path).unwrap();
    let collection = store.load_collection().unwrap();
    let entries: Vec<_> = collection
        .entries()
        .map(|entry| (entry.item.name.clone(), entry.quantity, entry.rank))
        .collect();

    assert_eq!(entries, vec![("Paris".to_owned(), 2, None)]);
}

/// The credential survives a close and reopen, which is the whole reason it is stored rather than
/// held: a player links once, not once per launch.
#[test]
fn a_stored_market_credential_survives_a_reopen() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("store.sqlite3");
    {
        let mut store = SqliteStore::open(&path).expect("store opens");
        store
            .set_market_credential("fake-token-value")
            .expect("credential stores");
    }

    let store = SqliteStore::open(&path).expect("store reopens");

    assert_eq!(
        store.market_credential().expect("credential reads"),
        Some("fake-token-value".to_owned())
    );
}

#[test]
fn a_store_with_no_credential_holds_none() {
    let store = SqliteStore::in_memory().expect("store opens");

    assert_eq!(store.market_credential().expect("credential reads"), None);
}

/// Storing again replaces rather than accumulates. A renewed token arrives on every authenticated
/// call, so a table that appended would grow without bound and leave the reader picking.
#[test]
fn storing_a_credential_replaces_the_previous_one() {
    let mut store = SqliteStore::in_memory().expect("store opens");

    store.set_market_credential("first-value").expect("stores");
    store.set_market_credential("second-value").expect("stores");

    assert_eq!(
        store.market_credential().expect("credential reads"),
        Some("second-value".to_owned())
    );
}

/// Unlinking removes the credential rather than blanking it, so a player who unlinked has no
/// credential left in the file rather than an empty one.
#[test]
fn clearing_removes_the_credential() {
    let mut store = SqliteStore::in_memory().expect("store opens");
    store.set_market_credential("fake-token-value").expect("stores");

    store.clear_market_credential().expect("clears");

    assert_eq!(store.market_credential().expect("credential reads"), None);
}

/// A version 3 database opens and migrates rather than being rejected. Every existing installation
/// is at version 3, so a migration that failed here would lose a working collection.
#[test]
fn a_version_three_database_migrates_to_four() {
    let directory = tempfile::tempdir().expect("temporary directory");
    let path = directory.path().join("store.sqlite3");
    {
        let connection = rusqlite::Connection::open(&path).expect("connection opens");
        connection
            .execute_batch(include_str!("../src/schema_v3.sql"))
            .expect("v3 schema applies");
        connection
            .execute(
                "INSERT INTO inventory (item_id, name, category, quantity, mastered)
                 VALUES ('paris', 'Paris', 'weapon', 2, 1)",
                [],
            )
            .expect("existing inventory row inserts");
        connection
            .pragma_update(None, "user_version", 3)
            .expect("version set");
    }

    let store = SqliteStore::open(&path).expect("v3 database migrates");

    assert_eq!(store.market_credential().expect("credential reads"), None);
    assert_eq!(
        store
            .load_collection()
            .expect("collection loads")
            .quantity(&ItemId::new("paris").unwrap()),
        2
    );
}
