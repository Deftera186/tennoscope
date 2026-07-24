use warframe_acquisition::{AcquisitionError, InventoryJsonDecoder, SnapshotDecoder};
use warframe_domain::Category;

fn complete_payload() -> Vec<u8> {
    br#"{
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
      "XPInfo":[
        {"ItemType":"/Lotus/Powersuits/Excalibur/Excalibur","XP":900000},
        {"ItemType":"/Lotus/Types/Sentinels/SentinelWeapons/BurstLaser","XP":900000},
        {"ItemType":"/Lotus/Weapons/Tenno/Pistol/Lato","XP":900000}
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
