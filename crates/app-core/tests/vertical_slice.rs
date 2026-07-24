use app_core::{AppCore, HealthState};
use local_store::SnapshotMeta;
use serde_json::json;
use tempfile::tempdir;
use warframe_domain::{
    CatalogItem, Category, InventoryEntry, InventorySnapshot, ItemId, RewardCandidate,
};

fn entry(id: &str, name: &str, category: Category, quantity: u32) -> InventoryEntry {
    InventoryEntry::new(
        CatalogItem::new(ItemId::new(id).unwrap(), name, category).unwrap(),
        quantity,
    )
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
    assert_eq!(view.health().game_reader().state(), HealthState::Degraded);
    assert_eq!(view.health().capture().state(), HealthState::Degraded);
    assert_eq!(view.health().catalog().state(), HealthState::Degraded);
    assert_eq!(view.health().market().state(), HealthState::Degraded);
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
    assert_eq!(view.health().capture().state(), HealthState::Degraded);
    assert_eq!(
        view.health().capture().message(),
        "Phase 1 capture not connected"
    );
    assert_eq!(view.health().capture().last_success(), None);
    assert_eq!(view.health().catalog().state(), HealthState::Degraded);
    assert_eq!(
        view.health().catalog().message(),
        "Phase 1 catalog not connected"
    );
    assert_eq!(view.health().catalog().last_success(), None);
    assert_eq!(view.health().market().state(), HealthState::Degraded);
    assert_eq!(
        view.health().market().message(),
        "Phase 1 market not connected"
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
                    {"id": "braton", "name": "Braton", "category": "weapon", "quantity": 3, "mastered": true},
                    {"id": "lex-prime-receiver", "name": "Lex Prime Receiver", "category": "prime_part", "quantity": 1, "mastered": false},
                    {"id": "lith-a1", "name": "Lith A1 Relic", "category": "relic", "quantity": 7, "mastered": false},
                    {"id": "rhino", "name": "Rhino", "category": "frame", "quantity": 1, "mastered": true},
                    {"id": "saryn-prime-chassis", "name": "Saryn Prime Chassis", "category": "prime_part", "quantity": 2, "mastered": false}
                ],
                "total_entries": 5
            },
            "reward": {
                "cards": [
                    {"name": "Forma Blueprint", "platinum": 12, "ducats": 25, "owned": 0, "mastery_relevant": false, "confidence": 1.0},
                    {"name": "Lex Prime Receiver", "platinum": 8, "ducats": 15, "owned": 0, "mastery_relevant": true, "confidence": 1.0},
                    {"name": "Rare Prime Set", "platinum": 30, "ducats": 100, "owned": 0, "mastery_relevant": false, "confidence": 0.79_f32},
                    {"name": "Paris Prime String", "platinum": 6, "ducats": 45, "owned": 1, "mastery_relevant": false, "confidence": 1.0}
                ],
                "best_value_index": 0
            },
            "health": {
                "game_reader": {"state": "ready", "message": "Deterministic fake inventory loaded", "last_success": "2000-01-01T00:00:00Z"},
                "capture": {"state": "degraded", "message": "Fake session; capture not connected", "last_success": null},
                "catalog": {"state": "degraded", "message": "Fake session; live catalog not connected", "last_success": null},
                "market": {"state": "degraded", "message": "Fake session; live market not connected", "last_success": null},
                "database": {"state": "ready", "message": "SQLite database available", "last_success": null}
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
