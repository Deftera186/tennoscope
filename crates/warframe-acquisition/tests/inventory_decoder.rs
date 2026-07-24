use warframe_acquisition::{AcquisitionError, InventoryJsonDecoder, SnapshotDecoder};
use warframe_domain::Category;

fn complete_payload() -> Vec<u8> {
    br#"{
      "LastInventorySync":{"$date":{"$numberLong":"1753392000000"}},
      "Suits":[{"ItemType":"/Lotus/Powersuits/Excalibur/Excalibur"}],
      "LongGuns":[
        {"ItemType":"/Lotus/Weapons/Tenno/Rifle/Braton"},
        {"ItemType":"/Lotus/Weapons/Tenno/Rifle/Braton"}
      ],
      "Pistols":[],"Melee":[],
      "Sentinels":[{"ItemType":"/Lotus/Types/Sentinels/SentinelTypes/Carrier"}],
      "MiscItems":[
        {"ItemType":"/Lotus/Types/Items/MiscItems/ArgonCrystal","ItemCount":4},
        {"ItemType":"/Lotus/Types/Items/MiscItems/ZeroMarker","ItemCount":0},
        {"ItemType":"/Lotus/Types/Projections/LithA1Bronze","ItemCount":3}
      ],
      "Recipes":[{"ItemType":"/Lotus/Types/Recipes/Weapons/LexPrimeBlueprint","ItemCount":2}],
      "PendingRecipes":[{"ItemType":"/Lotus/Types/Recipes/Weapons/LexPrimeBlueprint","ItemCount":1}],
      "SentinelWeapons":[{"ItemType":"/Lotus/Types/Sentinels/SentinelWeapons/BurstLaser"}],
      "SpaceSuits":[],"SpaceMelee":[],"SpaceGuns":[],
      "KubrowPets":[],"OperatorAmps":[],"MechSuits":[],
      "XPInfo":[
        {"ItemType":"/Lotus/Powersuits/Excalibur/Excalibur","XP":900000},
        {"ItemType":"/Lotus/Types/Sentinels/SentinelWeapons/BurstLaser","XP":450000},
        {"ItemType":"/Lotus/Weapons/Tenno/Pistol/Lato","XP":450000}
      ]
    }"#.to_vec()
}

#[test]
fn complete_payload_becomes_one_coherent_aggregated_snapshot() {
    let snapshot = InventoryJsonDecoder.decode(&complete_payload()).unwrap();
    let entries = snapshot.entries();

    assert_eq!(entries.len(), 8);
    let by_id = |id: &str| {
        entries
            .iter()
            .find(|entry| entry.item.id.as_str() == id)
            .unwrap()
    };

    let excalibur = by_id("/Lotus/Powersuits/Excalibur/Excalibur");
    assert_eq!(excalibur.item.name, "Excalibur");
    assert_eq!(excalibur.item.category, Category::Frame);
    assert_eq!(excalibur.quantity, 1);
    assert!(excalibur.mastered);

    let braton = by_id("/Lotus/Weapons/Tenno/Rifle/Braton");
    assert_eq!(braton.quantity, 2);
    assert_eq!(braton.item.name, "Braton");
    assert_eq!(braton.item.category, Category::Weapon);

    let recipe = by_id("/Lotus/Types/Recipes/Weapons/LexPrimeBlueprint");
    assert_eq!(recipe.quantity, 1);
    assert_eq!(recipe.item.name, "Lex Prime Blueprint");
    assert_eq!(recipe.item.category, Category::Blueprint);

    let resource = by_id("/Lotus/Types/Items/MiscItems/ArgonCrystal");
    assert_eq!(resource.item.category, Category::Resource);

    let relic = by_id("/Lotus/Types/Projections/LithA1Bronze");
    assert_eq!(relic.item.category, Category::Relic);

    let lato = by_id("/Lotus/Weapons/Tenno/Pistol/Lato");
    assert_eq!(lato.quantity, 0);
    assert!(lato.mastered);

    let sentinel_weapon = by_id("/Lotus/Types/Sentinels/SentinelWeapons/BurstLaser");
    assert_eq!(sentinel_weapon.item.category, Category::Weapon);
    assert!(sentinel_weapon.mastered);
    assert!(
        entries
            .iter()
            .all(|entry| !entry.item.id.as_str().ends_with("ZeroMarker"))
    );
}

#[test]
fn mastery_uses_category_specific_rank_thirty_thresholds() {
    let mut payload: serde_json::Value = serde_json::from_slice(&complete_payload()).unwrap();
    payload["LongGuns"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!({"ItemType":"/Lotus/Gear/SectionClassifiedWeapon"}));
    payload["XPInfo"] = serde_json::json!([
        {"ItemType":"/Lotus/Weapons/Tenno/Rifle/AtThreshold","XP":450000},
        {"ItemType":"/Lotus/Weapons/Tenno/Rifle/BelowThreshold","XP":449999},
        {"ItemType":"/Lotus/Powersuits/AtThreshold/AtThreshold","XP":900000},
        {"ItemType":"/Lotus/Powersuits/BelowThreshold/BelowThreshold","XP":899999},
        {"ItemType":"/Lotus/Types/Sentinels/SentinelTypes/Carrier","XP":900000},
        {"ItemType":"/Lotus/Gear/SectionClassifiedWeapon","XP":450000}
    ]);
    let snapshot = InventoryJsonDecoder
        .decode(&serde_json::to_vec(&payload).unwrap())
        .unwrap();
    let mastered = |suffix: &str| {
        snapshot
            .entries()
            .iter()
            .find(|entry| entry.item.id.as_str().ends_with(suffix))
            .unwrap()
            .mastered
    };

    assert!(mastered("Rifle/AtThreshold"));
    assert!(!mastered("Rifle/BelowThreshold"));
    assert!(mastered("AtThreshold/AtThreshold"));
    assert!(!mastered("BelowThreshold/BelowThreshold"));
    assert!(mastered("SentinelTypes/Carrier"));
    assert!(mastered("Gear/SectionClassifiedWeapon"));
}

#[test]
fn known_rank_forty_items_require_the_rank_forty_threshold() {
    let mut payload: serde_json::Value = serde_json::from_slice(&complete_payload()).unwrap();
    payload["XPInfo"] = serde_json::json!([
        {"ItemType":"/Lotus/Weapons/Tenno/Melee/Paracesis","XP":799999}
    ]);
    let below = InventoryJsonDecoder
        .decode(&serde_json::to_vec(&payload).unwrap())
        .unwrap();
    assert!(
        !below
            .entries()
            .iter()
            .find(|entry| entry.item.id.as_str().ends_with("Paracesis"))
            .unwrap()
            .mastered
    );

    payload["XPInfo"][0]["XP"] = serde_json::json!(800000);
    let complete = InventoryJsonDecoder
        .decode(&serde_json::to_vec(&payload).unwrap())
        .unwrap();
    assert!(
        complete
            .entries()
            .iter()
            .find(|entry| entry.item.id.as_str().ends_with("Paracesis"))
            .unwrap()
            .mastered
    );
}

#[test]
fn incomplete_truncated_or_structurally_wrong_payload_is_rejected() {
    let decoder = InventoryJsonDecoder;
    let truncated = &complete_payload()[..complete_payload().len() - 3];
    assert_eq!(
        decoder.decode(truncated),
        Err(AcquisitionError::SnapshotInvalid)
    );

    let missing_section = br#"{"Suits":[],"LongGuns":[],"Pistols":[],"Melee":[],"Sentinels":[],"MiscItems":[],"Recipes":[],"PendingRecipes":[]}"#;
    assert_eq!(
        decoder.decode(missing_section),
        Err(AcquisitionError::SnapshotInvalid)
    );

    let wrong_shape = br#"{"Suits":{},"LongGuns":[],"Pistols":[],"Melee":[],"Sentinels":[],"MiscItems":[],"Recipes":[],"PendingRecipes":[],"XPInfo":[]}"#;
    assert_eq!(
        decoder.decode(wrong_shape),
        Err(AcquisitionError::SnapshotInvalid)
    );
}

#[test]
fn every_authoritative_section_and_sync_marker_is_required() {
    let required = [
        "LastInventorySync",
        "Suits",
        "LongGuns",
        "Pistols",
        "Melee",
        "Sentinels",
        "MiscItems",
        "Recipes",
        "PendingRecipes",
        "XPInfo",
        "SpaceSuits",
        "SpaceMelee",
        "SpaceGuns",
        "SentinelWeapons",
        "KubrowPets",
        "OperatorAmps",
        "MechSuits",
    ];
    for field in required {
        let mut payload: serde_json::Value = serde_json::from_slice(&complete_payload()).unwrap();
        payload.as_object_mut().unwrap().remove(field);
        assert_eq!(
            InventoryJsonDecoder.decode(&serde_json::to_vec(&payload).unwrap()),
            Err(AcquisitionError::SnapshotInvalid),
            "omitting {field} must reject the whole snapshot"
        );
    }
}

#[test]
fn malformed_entries_or_unsafe_counts_reject_the_whole_snapshot() {
    let decoder = InventoryJsonDecoder;
    let malformed_path = String::from_utf8(complete_payload()).unwrap().replace(
        "/Lotus/Weapons/Tenno/Rifle/Braton",
        "not-a-canonical-item-path",
    );
    assert_eq!(
        decoder.decode(malformed_path.as_bytes()),
        Err(AcquisitionError::SnapshotInvalid)
    );

    let negative_count = String::from_utf8(complete_payload())
        .unwrap()
        .replace("\"ItemCount\":4", "\"ItemCount\":-1");
    assert_eq!(
        decoder.decode(negative_count.as_bytes()),
        Err(AcquisitionError::SnapshotInvalid)
    );
}

#[test]
fn stable_ids_preserve_paths_and_labels_split_acronyms_and_digits() {
    let payload = String::from_utf8(complete_payload()).unwrap().replace(
        "/Lotus/Weapons/Tenno/Rifle/Braton",
        "/Lotus/Weapons/Tenno/Rifle/SomaPrimeMK2",
    );
    let snapshot = InventoryJsonDecoder.decode(payload.as_bytes()).unwrap();
    let entry = snapshot
        .entries()
        .iter()
        .find(|entry| entry.item.id.as_str().ends_with("SomaPrimeMK2"))
        .unwrap();
    assert_eq!(
        entry.item.id.as_str(),
        "/Lotus/Weapons/Tenno/Rifle/SomaPrimeMK2"
    );
    assert_eq!(entry.item.name, "Soma Prime MK 2");
}
