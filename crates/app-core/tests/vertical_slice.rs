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

    assert_eq!(wire["health"]["game_reader"]["state"], json!("ready"));
    assert_eq!(
        wire["collection"]["items"][1]["category"],
        json!("prime_part")
    );
    assert_eq!(wire["collection"]["total_entries"], json!(5));
    assert_eq!(wire["collection"]["items"].as_array().unwrap().len(), 5);
    assert!(wire["reward"].get("best_value_name").is_none());
}
