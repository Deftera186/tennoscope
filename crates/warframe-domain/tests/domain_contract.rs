use serde::{Serialize, de::DeserializeOwned};
use warframe_domain::{
    CatalogItem, Category, Collection, InventoryEntry, InventorySnapshot, ItemId, RewardAdvisor,
    RewardCandidate,
};

fn item(id: &str, name: &str) -> CatalogItem {
    CatalogItem::new(ItemId::new(id).unwrap(), name, Category::Weapon).unwrap()
}

fn assert_serializable<T: Serialize + DeserializeOwned>() {}

#[test]
fn catalog_contract_validates_identifiers_and_names() {
    assert!(ItemId::new("").is_err());
    assert!(ItemId::new("   ").is_err());
    assert!(CatalogItem::new(ItemId::new("lex").unwrap(), "\t", Category::Weapon).is_err());

    let id = ItemId::new("lex").unwrap();
    assert_eq!(id.as_str(), "lex");
    assert_eq!(id.to_string(), "lex");
    assert_serializable::<ItemId>();
    assert_serializable::<Category>();
}

#[test]
fn coherent_snapshot_replacement_removes_absent_entries_and_reduces_quantity() {
    let lex = item("lex", "Lex");
    let forma = item("forma", "Forma");
    let lex_id = lex.id.clone();
    let forma_id = forma.id.clone();
    let mut collection = Collection::default();

    collection.replace(
        InventorySnapshot::coherent(vec![
            InventoryEntry::new(lex, 3).with_mastered(true),
            InventoryEntry::new(forma.clone(), 4),
        ])
        .unwrap(),
    );
    collection.replace(InventorySnapshot::coherent(vec![InventoryEntry::new(forma, 1)]).unwrap());

    assert_eq!(collection.quantity(&lex_id), 0);
    assert_eq!(collection.quantity(&forma_id), 1);
    assert_eq!(collection.entries().count(), 1);
}

#[test]
fn coherent_snapshot_rejects_duplicate_item_ids() {
    let first = item("lex", "Lex");
    let duplicate = item("lex", "Lex Blueprint");

    assert!(
        InventorySnapshot::coherent(vec![
            InventoryEntry::new(first, 1),
            InventoryEntry::new(duplicate, 2),
        ])
        .is_err()
    );
}

#[test]
fn reward_candidates_validate_names_and_confidence() {
    assert!(RewardCandidate::new(" ", 1, 1, 0, false, 0.9).is_err());
    for confidence in [f32::NAN, f32::INFINITY, -0.01, 1.01] {
        assert!(RewardCandidate::new("Lex", 1, 1, 0, false, confidence).is_err());
    }
}

#[test]
fn uncertain_high_value_reward_is_excluded_from_best_value() {
    let forma = RewardCandidate::new("Forma", 20, 0, 0, false, 0.40).unwrap();
    let lex = RewardCandidate::new("Lex", 8, 25, 0, true, 0.99).unwrap();

    let view = RewardAdvisor::advise(vec![forma, lex]);

    assert_eq!(view.cards[0].name, "Forma");
    assert_eq!(view.cards[1].name, "Lex");
    assert_eq!(view.best_value_index(), Some(1));
    assert_eq!(view.best_value_name(), Some("Lex"));
    assert_serializable::<warframe_domain::RewardView>();
}

#[test]
fn reward_ties_use_ducats_then_preserve_input_order() {
    let low_ducats = RewardCandidate::new("A", 10, 15, 0, false, 0.8).unwrap();
    let first_high = RewardCandidate::new("B", 10, 45, 0, false, 0.8).unwrap();
    let second_high = RewardCandidate::new("C", 10, 45, 0, false, 1.0).unwrap();

    let view = RewardAdvisor::advise(vec![low_ducats, first_high, second_high]);

    assert_eq!(view.best_value_index(), Some(1));
    assert_eq!(view.best_value_name(), Some("B"));
}

#[test]
fn all_uncertain_rewards_have_no_best_value() {
    let view = RewardAdvisor::advise(vec![
        RewardCandidate::new("A", 100, 100, 0, false, 0.79).unwrap(),
        RewardCandidate::new("B", 1, 1, 0, true, 0.0).unwrap(),
    ]);

    assert_eq!(view.best_value_index(), None);
    assert_eq!(view.best_value_name(), None);
}
