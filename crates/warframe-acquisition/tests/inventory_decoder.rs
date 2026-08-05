use warframe_acquisition::{AcquisitionError, CatalogIndex, InventoryJsonDecoder, SnapshotDecoder};
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
    let snapshot = InventoryJsonDecoder::default()
        .decode(&complete_payload())
        .unwrap();
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
    assert!(!excalibur.mastered);

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
    assert!(!lato.mastered);

    let sentinel_weapon = by_id("/Lotus/Types/Sentinels/SentinelWeapons/BurstLaser");
    assert_eq!(sentinel_weapon.item.category, Category::Weapon);
    assert!(!sentinel_weapon.mastered);
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
    let catalog = mastery_catalog();
    let snapshot = InventoryJsonDecoder::with_catalog(&catalog)
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
    let catalog = mastery_catalog();
    let below = InventoryJsonDecoder::with_catalog(&catalog)
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
    let complete = InventoryJsonDecoder::with_catalog(&catalog)
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
    let decoder = InventoryJsonDecoder::default();
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
fn mods_arcanes_sculptures_and_armaments_are_tracked_and_each_rank_is_its_own_row() {
    let mut payload: serde_json::Value = serde_json::from_slice(&complete_payload()).unwrap();
    payload["RawUpgrades"] = serde_json::json!([
        {"ItemType":"/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod","ItemCount":3,
         "LastAdded":{"$oid":"6889f4a4a1b2c3d4e5f60718"}},
        {"ItemType":"/Lotus/Upgrades/CosmeticEnhancers/Melee/ArcaneAvenger","ItemCount":2},
        {"ItemType":"/Lotus/Powersuits/Trinity/LinkAugmentCard","ItemCount":1}
    ]);
    payload["Upgrades"] = serde_json::json!([
        {"ItemId":{"$oid":"6889f4a4a1b2c3d4e5f60719"},
         "ItemType":"/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod",
         "UpgradeFingerprint":"{\"lvl\":5}"},
        {"ItemId":{"$oid":"6889f4a4a1b2c3d4e5f6071c"},
         "ItemType":"/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod",
         "UpgradeFingerprint":"{\"lvl\":5}"},
        {"ItemId":{"$oid":"6889f4a4a1b2c3d4e5f6071d"},
         "ItemType":"/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod",
         "UpgradeFingerprint":"{\"lvl\":10}"},
        {"ItemId":{"$oid":"6889f4a4a1b2c3d4e5f6071a"},
         "ItemType":"/Lotus/Upgrades/Mods/Randomized/LotusRifleRandomModRare",
         "UpgradeFingerprint":"{\"compat\":\"/Lotus/Weapons/Tenno/Rifle/Braton\"}"}
    ]);
    payload["FusionTreasures"] = serde_json::json!([{"ItemType":"/Lotus/Types/Items/FusionTreasures/OroFusexF",
                            "ItemCount":2,"Sockets":1}]);
    payload["CrewShipWeapons"] = serde_json::json!([{"ItemId":{"$oid":"6889f4a4a1b2c3d4e5f6071b"},
                            "ItemType":"/Lotus/Weapons/CrewShip/Turret/Zetki/ZetkiPhoton",
                            "UpgradeFingerprint":"{}","UpgradeType":"","UpgradeVer":101}]);

    let snapshot = InventoryJsonDecoder::default()
        .decode(&serde_json::to_vec(&payload).unwrap())
        .unwrap();
    let by_id = |id: &str| {
        snapshot
            .entries()
            .iter()
            .find(|entry| entry.item.id.as_str() == id)
            .unwrap_or_else(|| panic!("no entry for {id}"))
            .clone()
    };

    // Serration is held at three ranks, so it is three holdings worth three different prices --
    // 3p, and whatever a rank-5 and a rank-10 copy fetch. Summed onto one row, the only price the
    // row could honestly show is the unranked one, and the ranked copies would be given away.
    let unranked = by_id("/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod");
    assert_eq!(unranked.quantity, 3);
    assert_eq!(unranked.rank, None);
    assert_eq!(unranked.item.category, Category::Mod);
    assert!(!unranked.mastered);

    let rank_five = by_id("/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod#5");
    assert_eq!(
        rank_five.quantity, 2,
        "two copies at the same rank share a row"
    );
    assert_eq!(rank_five.rank, Some(5));
    assert_eq!(rank_five.item.name, "Weapon Damage Amount Mod");
    assert_eq!(rank_five.item.category, Category::Mod);

    assert_eq!(
        by_id("/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod#10").rank,
        Some(10)
    );

    // A riven's fingerprint carries rolled stats and no `lvl`, which is rank 0, not a parse failure.
    let riven = by_id("/Lotus/Upgrades/Mods/Randomized/LotusRifleRandomModRare");
    assert_eq!(riven.quantity, 1);
    assert_eq!(riven.rank, None);

    assert_eq!(
        by_id("/Lotus/Upgrades/CosmeticEnhancers/Melee/ArcaneAvenger")
            .item
            .category,
        Category::Arcane
    );

    // An augment mod lives at its Warframe's own path. Categorised by that path it would read as a
    // Frame the player has never owned.
    let augment = by_id("/Lotus/Powersuits/Trinity/LinkAugmentCard");
    assert_eq!(augment.item.category, Category::Mod);
    assert_eq!(augment.quantity, 1);

    let sculpture = by_id("/Lotus/Types/Items/FusionTreasures/OroFusexF");
    assert_eq!(sculpture.quantity, 2);
    assert_eq!(sculpture.item.category, Category::Resource);

    assert_eq!(
        by_id("/Lotus/Weapons/CrewShip/Turret/Zetki/ZetkiPhoton")
            .item
            .category,
        Category::Weapon
    );
}

/// The ceiling decides which of the market's two quotes a copy is owed, so it has to come from the
/// catalogue -- and a riven's published ceiling is a sentinel that has to be refused.
#[test]
fn ranked_copies_carry_the_ceiling_the_catalogue_can_vouch_for() {
    let catalog = CatalogIndex::from_wfcd_json(
        br#"[
          {"uniqueName":"/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod","name":"Serration","type":"Rifle Mod","category":"Mods","masterable":false,"fusionLimit":10},
          {"uniqueName":"/Lotus/Upgrades/Mods/Randomized/LotusRifleRandomModRare","name":"Rifle Riven Mod","type":"Rifle Mod","category":"Mods","masterable":false,"fusionLimit":515}
        ]"#,
    )
    .unwrap();
    let mut payload: serde_json::Value = serde_json::from_slice(&complete_payload()).unwrap();
    payload["Upgrades"] = serde_json::json!([
        {"ItemType":"/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod","UpgradeFingerprint":"{\"lvl\":10}"},
        {"ItemType":"/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod","UpgradeFingerprint":"{\"lvl\":8}"},
        {"ItemType":"/Lotus/Upgrades/Mods/Randomized/LotusRifleRandomModRare","UpgradeFingerprint":"{\"lvl\":3}"}
    ]);
    let snapshot = InventoryJsonDecoder::with_catalog(&catalog)
        .decode(&serde_json::to_vec(&payload).unwrap())
        .unwrap();
    let by_id = |id: &str| {
        snapshot
            .entries()
            .iter()
            .find(|entry| entry.item.id.as_str() == id)
            .unwrap_or_else(|| panic!("no entry for {id}"))
    };

    let maxed = by_id("/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod#10");
    assert_eq!(maxed.max_rank, Some(10));
    assert_eq!(maxed.at_max_rank(), Some(true));
    assert_eq!(maxed.item.name, "Serration");

    let partial = by_id("/Lotus/Upgrades/Mods/Rifle/WeaponDamageAmountMod#8");
    assert_eq!(partial.at_max_rank(), Some(false));

    // 515 is not a rank. Believed, a maxed rank-3 riven can never reach its ceiling and is quoted
    // forever as a card somebody abandoned part-way up.
    let riven = by_id("/Lotus/Upgrades/Mods/Randomized/LotusRifleRandomModRare#3");
    assert_eq!(riven.max_rank, None);
    assert_eq!(riven.at_max_rank(), None);
}

/// The four sections above were added to the decoder after the others, so an account holding
/// nothing in one of them must still sync. Only the sections that were always authoritative are
/// allowed to reject a snapshot by their absence.
#[test]
fn a_section_the_account_has_nothing_in_is_omitted_not_broken() {
    // `inventory.php` leaves a section out entirely when the account holds nothing in it: no
    // Necramech means no `MechSuits`, no Amp means no `OperatorAmps`. Requiring every section
    // turned "this player has not reached Deimos yet" into a whole failed read.
    let optional = [
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
    for field in optional {
        let mut payload: serde_json::Value = serde_json::from_slice(&complete_payload()).unwrap();
        payload.as_object_mut().unwrap().remove(field);
        let snapshot = InventoryJsonDecoder::default()
            .decode(&serde_json::to_vec(&payload).unwrap())
            .unwrap_or_else(|error| panic!("omitting {field} must still decode, got {error:?}"));
        assert!(
            !snapshot.entries().is_empty(),
            "omitting {field} must keep every other section"
        );
    }

    // The one section this account genuinely has nothing in.
    let no_mechs: serde_json::Value = serde_json::from_slice(&complete_payload()).unwrap();
    let with_mechs = InventoryJsonDecoder::default()
        .decode(&serde_json::to_vec(&no_mechs).unwrap())
        .unwrap();
    let mut stripped = no_mechs;
    stripped.as_object_mut().unwrap().remove("MechSuits");
    assert_eq!(
        InventoryJsonDecoder::default()
            .decode(&serde_json::to_vec(&stripped).unwrap())
            .unwrap()
            .entries(),
        with_mechs.entries(),
        "an omitted empty section decodes the same as an explicitly empty one"
    );
}

#[test]
fn the_sync_marker_is_required_and_a_snapshot_holding_nothing_is_not_believed() {
    let mut payload: serde_json::Value = serde_json::from_slice(&complete_payload()).unwrap();
    payload.as_object_mut().unwrap().remove("LastInventorySync");
    assert_eq!(
        InventoryJsonDecoder::default().decode(&serde_json::to_vec(&payload).unwrap()),
        Err(AcquisitionError::SnapshotInvalid),
        "without the sync marker this is not an inventory response"
    );

    // Sections are optional one at a time, not all at once: no logged-in account owns nothing, so
    // a response that decodes to an empty collection is a response that was not understood.
    assert_eq!(
        InventoryJsonDecoder::default()
            .decode(br#"{"LastInventorySync":{"$date":{"$numberLong":"1753392000000"}}}"#),
        Err(AcquisitionError::SnapshotInvalid),
        "a snapshot with no holdings at all must not read as an empty collection"
    );
}

#[test]
fn malformed_entries_or_unsafe_counts_reject_the_whole_snapshot() {
    let decoder = InventoryJsonDecoder::default();
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
    let snapshot = InventoryJsonDecoder::default()
        .decode(payload.as_bytes())
        .unwrap();
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

fn mastery_catalog() -> CatalogIndex {
    CatalogIndex::from_wfcd_json(
        br#"[
          {"uniqueName":"/Lotus/Weapons/Tenno/Rifle/AtThreshold","name":"At Threshold","type":"Rifle","category":"Primary","masterable":true},
          {"uniqueName":"/Lotus/Weapons/Tenno/Rifle/BelowThreshold","name":"Below Threshold","type":"Rifle","category":"Primary","masterable":true},
          {"uniqueName":"/Lotus/Powersuits/AtThreshold/AtThreshold","name":"At Threshold","type":"Warframe","category":"Warframes","masterable":true},
          {"uniqueName":"/Lotus/Powersuits/BelowThreshold/BelowThreshold","name":"Below Threshold","type":"Warframe","category":"Warframes","masterable":true},
          {"uniqueName":"/Lotus/Types/Sentinels/SentinelTypes/Carrier","name":"Carrier","type":"Sentinel","category":"Companions","masterable":true},
          {"uniqueName":"/Lotus/Gear/SectionClassifiedWeapon","name":"Section Classified Weapon","type":"Rifle","category":"Primary","masterable":true},
          {"uniqueName":"/Lotus/Weapons/Tenno/Melee/Paracesis","name":"Paracesis","type":"Melee","category":"Melee","masterable":true}
        ]"#,
    )
    .unwrap()
}
