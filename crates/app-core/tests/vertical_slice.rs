use app_core::{AppCore, HealthState};
use local_store::SnapshotMeta;
use serde_json::json;
use tempfile::tempdir;
use warframe_acquisition::CatalogIndex;
use warframe_domain::{
    CatalogItem, Category, InventoryEntry, InventorySnapshot, ItemId, RewardCandidate,
};

fn entry(id: &str, name: &str, category: Category, quantity: u32) -> InventoryEntry {
    InventoryEntry::new(
        CatalogItem::new(ItemId::new(id).unwrap(), name, category).unwrap(),
        quantity,
    )
}

fn illustrated_entry(
    id: &str,
    name: &str,
    category: Category,
    quantity: u32,
    image_name: &str,
) -> InventoryEntry {
    InventoryEntry::new(
        CatalogItem::new(ItemId::new(id).unwrap(), name, category)
            .unwrap()
            .with_image_name(image_name)
            .unwrap(),
        quantity,
    )
}

#[test]
fn canonical_artwork_reaches_the_serialized_collection_view() {
    let mut core = AppCore::in_memory().unwrap();
    let snapshot = InventorySnapshot::coherent(vec![illustrated_entry(
        "braton",
        "Braton",
        Category::Weapon,
        1,
        "Braton.png",
    )])
    .unwrap();

    let view = core
        .apply_inventory_snapshot(snapshot, SnapshotMeta::fake("art-build").unwrap())
        .unwrap();

    assert_eq!(
        serde_json::to_value(&view.collection().items()[0]).unwrap()["image_url"],
        json!("https://raw.githubusercontent.com/WFCD/warframe-items/master/data/img/Braton.png")
    );
}

#[test]
fn cached_snapshot_can_be_enriched_without_becoming_fresh() {
    let mut core = AppCore::in_memory().unwrap();
    let meta = SnapshotMeta::new(
        "2026-07-25T08:09:10Z".into(),
        "build-42".into(),
        "warframe-memory".into(),
    )
    .unwrap();
    core.apply_inventory_snapshot(
        InventorySnapshot::coherent(vec![entry(
            "/Lotus/Types/Items/MiscItems/Alertium",
            "Alertium",
            Category::Resource,
            7,
        )])
        .unwrap(),
        meta,
    )
    .unwrap();
    let catalog = CatalogIndex::from_wfcd_json(
        br#"[{"uniqueName":"/Lotus/Types/Items/MiscItems/Alertium","name":"Nitain Extract","type":"Misc","category":"Misc","imageName":"Alertium.png"}]"#,
    )
    .unwrap();

    let view = core.enrich_collection_from_catalog(&catalog).unwrap();

    let item = &view.collection().items()[0];
    assert_eq!(item.name(), "Nitain Extract");
    assert_eq!(item.quantity(), 7);
    assert!(item.image_url().unwrap().ends_with("/Alertium.png"));
    assert_eq!(
        view.collection().snapshot().unwrap().observed_at(),
        "2026-07-25T08:09:10Z"
    );
}

#[test]
fn snapshot_freshness_reaches_the_serialized_collection_view() {
    let mut core = AppCore::in_memory().unwrap();
    let meta = SnapshotMeta::new(
        "2026-07-25T08:09:10Z".into(),
        "build-42".into(),
        "warframe-memory".into(),
    )
    .unwrap();
    let view = core
        .apply_inventory_snapshot(InventorySnapshot::coherent(vec![]).unwrap(), meta)
        .unwrap();

    assert_eq!(
        serde_json::to_value(&view).unwrap()["collection"]["snapshot"],
        json!({
            "observed_at": "2026-07-25T08:09:10Z",
            "game_build": "build-42",
            "source": "warframe-memory"
        })
    );
}

#[test]
fn in_memory_starts_empty_and_reports_honest_health() {
    let core = AppCore::in_memory().unwrap();

    let view = core.current_view().unwrap();

    assert_eq!(view.collection().total_entries(), 0);
    assert!(view.collection().items().is_empty());
    assert!(view.reward().cards().is_empty());
    assert_eq!(view.reward().best_value_index(), None);
    assert_eq!(view.health().database().state(), HealthState::Ready);
    assert_eq!(view.health().game_reader().state(), HealthState::Idle);
    assert_eq!(view.health().capture().state(), HealthState::Idle);
    assert_eq!(view.health().catalog().state(), HealthState::Idle);
    assert_eq!(view.health().market().state(), HealthState::Idle);
}

#[test]
fn discovered_game_process_is_reported_before_inventory_refresh_finishes() {
    let mut core = AppCore::in_memory().unwrap();

    let view = core.record_game_process_ready().unwrap();

    assert_eq!(view.health().game_reader().state(), HealthState::Ready);
    assert_eq!(
        view.health().game_reader().message(),
        "Warframe process connected"
    );
}

#[test]
fn fake_session_is_deterministic_and_uses_domain_ranking() {
    let mut core = AppCore::in_memory().unwrap();

    let view = core.load_fake_session().unwrap();

    assert_eq!(view.collection().total_entries(), 5);
    assert_eq!(view.collection().items().len(), 5);
    assert_eq!(
        view.collection()
            .items()
            .iter()
            .map(|item| item.id())
            .collect::<Vec<_>>(),
        vec![
            "braton",
            "lex-prime-receiver",
            "lith-a1",
            "rhino",
            "saryn-prime-chassis"
        ]
    );
    assert!(view.collection().items().iter().any(|item| item.mastered()));
    assert_eq!(view.health().game_reader().state(), HealthState::Ready);
    assert_eq!(view.reward().cards().len(), 4);
    assert_eq!(view.reward().best_value_name(), Some("Forma Blueprint"));
    let lex = view
        .reward()
        .cards()
        .iter()
        .find(|card| card.name == "Lex Prime Receiver")
        .unwrap();
    assert!(lex.mastery_relevant);
    assert!(view.reward().cards().iter().any(|card| {
        card.platinum > 12 && card.confidence < 0.80 && card.name != "Forma Blueprint"
    }));
}

#[test]
fn a_second_snapshot_authoritatively_replaces_collection() {
    let mut core = AppCore::in_memory().unwrap();
    core.load_fake_session().unwrap();
    let snapshot =
        InventorySnapshot::coherent(vec![entry("braton", "Braton", Category::Weapon, 1)]).unwrap();

    let view = core
        .apply_inventory_snapshot(snapshot, SnapshotMeta::fake("second-build").unwrap())
        .unwrap();

    assert_eq!(view.collection().total_entries(), 1);
    assert_eq!(view.collection().items()[0].id(), "braton");
    assert_eq!(view.collection().items()[0].quantity(), 1);
    assert!(view.reward().cards().is_empty());
    assert_eq!(view.reward().best_value_index(), None);
}

#[test]
fn a_live_snapshot_replaces_fake_reader_health_metadata() {
    let mut core = AppCore::in_memory().unwrap();
    let fake = core.load_fake_session().unwrap();
    assert_eq!(fake.health().game_reader().state(), HealthState::Ready);
    assert_eq!(
        fake.health().game_reader().message(),
        "Deterministic fake inventory loaded"
    );
    assert_eq!(
        fake.health().game_reader().last_success(),
        Some("2000-01-01T00:00:00Z")
    );
    assert_eq!(fake.health().capture().state(), HealthState::Degraded);
    assert_eq!(
        fake.health().capture().message(),
        "Fake session; capture not connected"
    );
    assert_eq!(fake.health().capture().last_success(), None);
    assert_eq!(fake.health().catalog().state(), HealthState::Degraded);
    assert_eq!(
        fake.health().catalog().message(),
        "Fake session; live catalog not connected"
    );
    assert_eq!(fake.health().catalog().last_success(), None);
    assert_eq!(fake.health().market().state(), HealthState::Degraded);
    assert_eq!(
        fake.health().market().message(),
        "Fake session; live market not connected"
    );
    assert_eq!(fake.health().market().last_success(), None);
    assert_eq!(fake.health().database().state(), HealthState::Ready);
    assert_eq!(
        fake.health().database().message(),
        "SQLite database available"
    );
    assert_eq!(fake.health().database().last_success(), None);
    let snapshot =
        InventorySnapshot::coherent(vec![entry("braton", "Braton", Category::Weapon, 1)]).unwrap();
    let meta = SnapshotMeta::new(
        "2026-07-24T09:30:00Z".to_owned(),
        "live-build".to_owned(),
        "game-log".to_owned(),
    )
    .unwrap();

    let view = core.apply_inventory_snapshot(snapshot, meta).unwrap();

    assert_eq!(view.health().game_reader().state(), HealthState::Ready);
    assert_eq!(
        view.health().game_reader().last_success(),
        Some("2026-07-24T09:30:00Z")
    );
    assert!(view.health().game_reader().message().contains("game-log"));
    assert!(!view.health().game_reader().message().contains("fake"));
    assert_eq!(view.health().capture().state(), HealthState::Idle);
    assert_eq!(
        view.health().capture().message(),
        "OCR reward observer idle; no reward screen yet"
    );
    assert_eq!(view.health().capture().last_success(), None);
    assert_eq!(view.health().catalog().state(), HealthState::Idle);
    assert_eq!(
        view.health().catalog().message(),
        "Item catalog has not loaded yet"
    );
    assert_eq!(view.health().catalog().last_success(), None);
    assert_eq!(view.health().market().state(), HealthState::Idle);
    assert_eq!(
        view.health().market().message(),
        "warframe.market pricing idle; nothing to price yet"
    );
    assert_eq!(view.health().market().last_success(), None);
    assert_eq!(view.health().database().state(), HealthState::Ready);
    assert_eq!(
        view.health().database().message(),
        "SQLite database available"
    );
    assert_eq!(view.health().database().last_success(), None);
}

#[test]
fn reward_application_preserves_source_order_and_domain_tie_breaking() {
    let mut core = AppCore::in_memory().unwrap();
    let rewards = vec![
        RewardCandidate::new("Low Ducats", 10, 15, 0, false, 0.8).unwrap(),
        RewardCandidate::new("First High", 10, 45, 0, false, 0.8).unwrap(),
        RewardCandidate::new("Uncertain", 100, 100, 0, false, 0.79).unwrap(),
        RewardCandidate::new("Second High", 10, 45, 0, false, 1.0).unwrap(),
    ];

    let view = core.apply_reward_candidates(rewards).unwrap();

    assert_eq!(view.reward().best_value_name(), Some("First High"));
    assert_eq!(view.reward().cards()[2].name, "Uncertain");
}

#[test]
fn reward_observer_health_exposes_only_source_and_bounded_timing() {
    let mut core = AppCore::in_memory().unwrap();

    let view = core
        .record_capture_source_ready("memory", 89, "2026-07-25T00:00:00Z")
        .unwrap();

    assert_eq!(view.health().capture().state(), HealthState::Ready);
    assert_eq!(
        view.health().capture().message(),
        "Memory reward observer ready (89 ms)"
    );
    let json = serde_json::to_string(&view).unwrap();
    assert!(!json.contains("0x"));
    assert!(!json.contains("/Lotus/"));
}

#[test]
fn file_backed_collection_persists_but_rewards_are_ephemeral() {
    let directory = tempdir().unwrap();
    let path = directory.path().join("helper.sqlite");
    {
        let mut core = AppCore::open(&path).unwrap();
        core.load_fake_session().unwrap();
    }

    let reopened = AppCore::open(&path).unwrap();
    let view = reopened.current_view().unwrap();

    assert_eq!(view.collection().total_entries(), 5);
    assert!(view.reward().cards().is_empty());
    assert_eq!(view.reward().best_value_index(), None);
}

#[test]
fn serialized_view_has_stable_wire_values_and_consistent_derived_fields() {
    let mut core = AppCore::in_memory().unwrap();
    let view = core.load_fake_session().unwrap();

    let wire = serde_json::to_value(&view).unwrap();

    assert_eq!(
        wire,
        json!({
            "collection": {
                "items": [
                    {"id": "braton", "name": "Braton", "category": "weapon", "quantity": 3, "mastered": true, "live": false, "priceable": false},
                    {"id": "lex-prime-receiver", "name": "Lex Prime Receiver", "category": "prime_part", "quantity": 1, "mastered": false, "live": false, "priceable": false},
                    {"id": "lith-a1", "name": "Lith A1 Relic", "category": "relic", "quantity": 7, "mastered": false, "live": false, "priceable": false},
                    {"id": "rhino", "name": "Rhino", "category": "frame", "quantity": 1, "mastered": true, "live": false, "priceable": false},
                    {"id": "saryn-prime-chassis", "name": "Saryn Prime Chassis", "category": "prime_part", "quantity": 2, "mastered": false, "live": false, "priceable": false}
                ],
                "total_entries": 5,
                "snapshot": {
                    "observed_at": "2000-01-01T00:00:00Z",
                    "game_build": "fake-build",
                    "source": "test-fixture"
                },
                "pricing": null
            },
            "reward": {
                "cards": [
                    {"name": "Forma Blueprint", "platinum": 12, "ducats": 25, "owned": 0, "mastery_relevant": false, "confidence": 1.0},
                    {"name": "Lex Prime Receiver", "platinum": 8, "ducats": 15, "owned": 0, "mastery_relevant": true, "confidence": 1.0},
                    {"name": "Rare Prime Set", "platinum": 30, "ducats": 100, "owned": 0, "mastery_relevant": false, "confidence": 0.79_f32},
                    {"name": "Paris Prime String", "platinum": 6, "ducats": 45, "owned": 1, "mastery_relevant": false, "confidence": 1.0}
                ],
                "best_value_index": 0,
                "best_ducat_index": 3
            },
            "health": {
                "acquisition_stages": [],
                "game_reader": {"state": "ready", "message": "Deterministic fake inventory loaded", "last_success": "2000-01-01T00:00:00Z"},
                "log_monitor": {"state": "idle", "message": "Waiting for Warframe", "last_success": null},
                "capture": {"state": "degraded", "message": "Fake session; capture not connected", "last_success": null},
                "catalog": {"state": "degraded", "message": "Fake session; live catalog not connected", "last_success": null},
                "market": {"state": "degraded", "message": "Fake session; live market not connected", "last_success": null},
                "collection_prices": {"state": "idle", "message": "Collection price dump has not loaded yet", "last_success": null},
                "database": {"state": "ready", "message": "SQLite database available", "last_success": null},
                "market_account": {"state": "idle", "message": "No warframe.market account linked", "last_success": null}
            },
            "market_account": {
                "link": "unlinked",
                "backing": null,
                "orders": [],
                "fetched_at": null,
                "listed_platinum": 0,
                "listable": [],
                "presence": { "status": null, "wanted": null, "auto": false },
                "flagged": 0
            }
        })
    );
}

#[test]
fn vehicle_category_reaches_the_ui_model_with_a_stable_wire_value() {
    let mut core = AppCore::in_memory().unwrap();
    let snapshot =
        InventorySnapshot::coherent(vec![entry("bad-baby", "Bad Baby", Category::Vehicle, 1)])
            .unwrap();
    let view = core
        .apply_inventory_snapshot(snapshot, SnapshotMeta::fake("vehicle-build").unwrap())
        .unwrap();

    assert_eq!(view.collection().items()[0].category(), Category::Vehicle);
    assert_eq!(
        serde_json::to_value(&view.collection().items()[0]).unwrap()["category"],
        json!("vehicle")
    );
}
