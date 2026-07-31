use serde::{Serialize, de::DeserializeOwned};
use serde_json::json;
use warframe_domain::{
    CatalogItem, Category, Collection, InventoryEntry, InventorySnapshot, ItemId, RewardAdvisor,
    RewardCandidate,
};

fn item(id: &str, name: &str) -> CatalogItem {
    CatalogItem::new(ItemId::new(id).unwrap(), name, Category::Weapon).unwrap()
}

fn assert_serializable<T: Serialize + DeserializeOwned>() {}

fn assert_output_serializable<T: Serialize>() {}

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

    assert_eq!(view.cards()[0].name, "Forma");
    assert_eq!(view.cards()[1].name, "Lex");
    assert_eq!(view.best_value_index(), Some(1));
    assert_eq!(view.best_value_name(), Some("Lex"));
    assert_output_serializable::<warframe_domain::RewardView>();
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
    assert_eq!(view.best_ducat_index(), None);
}

/// The case that ranking on platinum alone hides. A cheap common can carry more ducats than the
/// card the market values highest, and a player saving for Baro wants that one -- so the two
/// answers have to be separately visible, not collapsed into a tiebreak.
#[test]
fn the_ducat_winner_is_reported_even_when_another_card_is_worth_more_platinum() {
    let view = RewardAdvisor::advise(vec![
        RewardCandidate::new("Pricey Prime Blueprint", 45, 15, 0, false, 1.0).unwrap(),
        RewardCandidate::new("Cheap Prime Barrel", 6, 100, 0, false, 1.0).unwrap(),
    ]);

    assert_eq!(view.best_value_name(), Some("Pricey Prime Blueprint"));
    assert_eq!(view.best_ducat_index(), Some(1));
}

/// Forma carries no ducats at all. A screen of nothing but Forma must not crown one of them for a
/// currency none of them are worth.
#[test]
fn no_ducat_winner_when_nothing_on_offer_is_worth_ducats() {
    let view = RewardAdvisor::advise(vec![
        RewardCandidate::new("Forma Blueprint", 12, 0, 0, false, 1.0).unwrap(),
        RewardCandidate::new("2X Forma Blueprint", 20, 0, 0, false, 1.0).unwrap(),
    ]);

    assert_eq!(view.best_value_index(), Some(1));
    assert_eq!(view.best_ducat_index(), None);
}

#[test]
fn category_wire_shape_is_stable_snake_case() {
    let cases = [
        (Category::Frame, "frame"),
        (Category::Weapon, "weapon"),
        (Category::Companion, "companion"),
        (Category::PrimePart, "prime_part"),
        (Category::Relic, "relic"),
        (Category::Resource, "resource"),
        (Category::Blueprint, "blueprint"),
        (Category::Vehicle, "vehicle"),
        (Category::Mod, "mod"),
        (Category::Arcane, "arcane"),
    ];

    for (category, wire) in cases {
        assert_eq!(serde_json::to_value(category).unwrap(), json!(wire));
        assert_eq!(
            serde_json::from_value::<Category>(json!(wire)).unwrap(),
            category
        );
    }
}

#[test]
fn valid_domain_values_round_trip_through_json() {
    let id = ItemId::new("lex").unwrap();
    let catalog_item = item("lex", "Lex");
    let snapshot = InventorySnapshot::coherent(vec![
        InventoryEntry::new(catalog_item.clone(), 2).with_mastered(true),
    ])
    .unwrap();
    let reward = RewardCandidate::new("Lex Prime Receiver", 8, 25, 1, true, 0.99).unwrap();

    let id_wire = serde_json::to_value(&id).unwrap();
    let item_wire = serde_json::to_value(&catalog_item).unwrap();
    let snapshot_wire = serde_json::to_value(&snapshot).unwrap();
    let reward_wire = serde_json::to_value(&reward).unwrap();

    assert_eq!(id_wire, json!("lex"));
    assert_eq!(
        item_wire,
        json!({"id": "lex", "name": "Lex", "category": "weapon"})
    );
    assert_eq!(
        snapshot_wire,
        json!({
            "entries": [{
                "item": {"id": "lex", "name": "Lex", "category": "weapon"},
                "quantity": 2,
                "mastered": true
            }]
        })
    );
    assert_eq!(
        reward_wire,
        json!({
            "name": "Lex Prime Receiver",
            "platinum": 8,
            "ducats": 25,
            "owned": 1,
            "mastery_relevant": true,
            "confidence": 0.99_f32
        })
    );

    let id_round_trip: ItemId = serde_json::from_value(id_wire).unwrap();
    let item_round_trip: CatalogItem = serde_json::from_value(item_wire).unwrap();
    let snapshot_round_trip: InventorySnapshot = serde_json::from_value(snapshot_wire).unwrap();
    let reward_round_trip: RewardCandidate = serde_json::from_value(reward_wire).unwrap();

    assert_eq!(id_round_trip, id);
    assert_eq!(item_round_trip, catalog_item);
    assert_eq!(snapshot_round_trip, snapshot);
    assert_eq!(reward_round_trip, reward);
}

#[test]
fn deserialization_rejects_invalid_catalog_values() {
    assert!(serde_json::from_value::<ItemId>(json!("  ")).is_err());
    assert!(
        serde_json::from_value::<CatalogItem>(json!({
            "id": "lex",
            "name": "\t",
            "category": "weapon"
        }))
        .is_err()
    );
}

#[test]
fn deserialization_rejects_duplicate_snapshot_entries() {
    let entry = json!({
        "item": {"id": "lex", "name": "Lex", "category": "weapon"},
        "quantity": 1,
        "mastered": false
    });
    assert!(
        serde_json::from_value::<InventorySnapshot>(json!({
            "entries": [entry.clone(), entry]
        }))
        .is_err()
    );
}

#[test]
fn deserialization_rejects_invalid_reward_candidates() {
    let candidate = |name: &str, confidence: f32| {
        json!({
            "name": name,
            "platinum": 8,
            "ducats": 25,
            "owned": 0,
            "mastery_relevant": true,
            "confidence": confidence
        })
    };

    assert!(serde_json::from_value::<RewardCandidate>(candidate(" ", 0.9)).is_err());
    assert!(serde_json::from_value::<RewardCandidate>(candidate("Lex", -0.01)).is_err());
    assert!(serde_json::from_value::<RewardCandidate>(candidate("Lex", 1.01)).is_err());
}

#[test]
fn reward_view_serializes_derived_selection_without_being_mutable() {
    let view = RewardAdvisor::advise(vec![
        RewardCandidate::new("Forma", 20, 0, 0, false, 0.4).unwrap(),
        RewardCandidate::new("Lex", 8, 25, 0, true, 0.99).unwrap(),
    ]);

    assert_eq!(
        serde_json::to_value(&view).unwrap(),
        json!({
            "cards": [
                {"name": "Forma", "platinum": 20, "ducats": 0, "owned": 0, "mastery_relevant": false, "confidence": 0.4_f32},
                {"name": "Lex", "platinum": 8, "ducats": 25, "owned": 0, "mastery_relevant": true, "confidence": 0.99_f32}
            ],
            "best_value_index": 1,
            "best_ducat_index": 1
        })
    );
}

#[test]
fn collection_json_round_trips_with_deterministic_item_id_keys() {
    let lex = item("lex", "Lex");
    let mut collection = Collection::default();
    collection.replace(
        InventorySnapshot::coherent(vec![InventoryEntry::new(lex, 2).with_mastered(true)]).unwrap(),
    );

    let wire = serde_json::to_value(&collection).unwrap();
    assert_eq!(
        wire,
        json!({
            "entries": {
                "lex": {
                    "item": {"id": "lex", "name": "Lex", "category": "weapon"},
                    "quantity": 2,
                    "mastered": true
                }
            }
        })
    );

    let round_trip: Collection = serde_json::from_value(wire).unwrap();
    assert_eq!(round_trip.quantity(&ItemId::new("lex").unwrap()), 2);
}

#[test]
fn collection_deserialization_rejects_mismatched_map_and_item_ids() {
    assert!(
        serde_json::from_value::<Collection>(json!({
            "entries": {
                "lex_prime": {
                    "item": {"id": "lex", "name": "Lex", "category": "weapon"},
                    "quantity": 1,
                    "mastered": false
                }
            }
        }))
        .is_err()
    );
}

#[test]
fn collection_deserialization_rejects_duplicate_logical_item_ids() {
    assert!(
        serde_json::from_value::<Collection>(json!({
            "entries": {
                "lex": {
                    "item": {"id": "lex", "name": "Lex", "category": "weapon"},
                    "quantity": 1,
                    "mastered": false
                },
                "lex_alias": {
                    "item": {"id": "lex", "name": "Lex", "category": "weapon"},
                    "quantity": 2,
                    "mastered": true
                }
            }
        }))
        .is_err()
    );
}
