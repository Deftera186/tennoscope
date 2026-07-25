use warframe_acquisition::{CatalogIndex, InventoryJsonDecoder, SnapshotDecoder};
use warframe_domain::Category;

fn wfcd_fixture() -> Vec<u8> {
    br#"[
      {
        "uniqueName":"/Lotus/Weapons/Tenno/Bows/PrimeHuntingBow",
        "name":"Paris Prime","type":"Bow","category":"Primary","masterable":true,"imageName":"ParisPrime.png",
        "components":[
          {"uniqueName":"/Lotus/Types/Recipes/Weapons/WeaponParts/PrimeBowString","name":"String","tradable":true,"ducats":15,"primeSellingPrice":15,"imageName":"PrimeBowString.png"},
          {"uniqueName":"/Lotus/Types/Items/MiscItems/Ferrite","name":"Ferrite","tradable":false}
        ]
      },
      {
        "uniqueName":"/Lotus/Types/Recipes/Kubrow/Collars/PrimeKubrowCollarA","name":"Kavasa Prime Kubrow Collar","type":"Companion","category":"Companions","masterable":false,
        "components":[
          {"uniqueName":"/Lotus/Types/Recipes/Kubrow/Collars/PrimeKubrowCollarABlueprint","name":"Blueprint","tradable":true,"ducats":45,"primeSellingPrice":45},
          {"uniqueName":"/Lotus/Types/Recipes/Kubrow/Collars/PrimeKubrowCollarABandComponent","name":"Kavasa Prime Band","tradable":true,"ducats":45,"primeSellingPrice":45}
        ]
      },
      {"uniqueName":"/Lotus/Types/Items/MiscItems/ArgonCrystal","name":"Argon Crystal","type":"Resource","category":"Resources","masterable":false,"components":[]},
      {"uniqueName":"/Lotus/Types/Items/MiscItems/Alertium","name":"Nitain Extract","type":"Misc","category":"Misc","masterable":false,"imageName":"Alertium.png","components":[]},
      {"uniqueName":"/Lotus/Types/Friendly/Pets/CatbrowPetPowerSuit","name":"Smeeta Kavat","type":"Kavat","category":"Companions","masterable":true,"components":[]},
      {"uniqueName":"/Lotus/Types/Game/CatbrowPet/MirrorCatbrowPetPowerSuit","name":"Adarza Kavat","type":"Pets","category":"Pets","masterable":true,"components":[]},
      {"uniqueName":"/Lotus/Weapons/Tenno/Archwing/Primary/NokkoArchGun/NokkoArchGun","name":"Arbucep","type":"Arch-Gun","category":"Arch-Gun","masterable":true,"components":[]},
      {"uniqueName":"/Lotus/Weapons/Tenno/Archwing/Melee/ArchSwordHook/ArchHookSwordWeapon","name":"Agkuza","type":"Arch-Melee","category":"Arch-Melee","masterable":true,"components":[]},
      {"uniqueName":"/Lotus/Weapons/SolarisUnited/Secondary/SUModularSecondarySet1/Barrel/SUModularSecondaryBarrelAPart","name":"Catchmoon","type":"Kitgun Component","category":"Misc","masterable":true,"components":[]},
      {"uniqueName":"/Lotus/Weapons/Ostron/Melee/ModularMelee01/Tip/TipOne","name":"Balla","type":"Zaw Component","category":"Melee","masterable":true,"components":[]},
      {"uniqueName":"/Lotus/Types/Vehicles/Hoverboard/HoverboardParts/PartComponents/HoverboardSolarisA/HoverboardSolarisADeck","name":"Bad Baby","type":"K-Drive Component","category":"Misc","masterable":true,"components":[]},
      {"uniqueName":"/Lotus/Weapons/Tenno/Rifle/Braton","name":"Braton","type":"Rifle","category":"Primary","masterable":true,"components":[]},
      {"uniqueName":"/Lotus/Weapons/Orokin/BallasSword/BallasSwordWeapon","name":"Paracesis","type":"Rifle","category":"Melee","masterable":true,"components":[]},
      {"uniqueName":"/Lotus/Weapons/Grineer/KuvaLich/LongGuns/KuvaChakkhurr","name":"Kuva Chakkhurr","type":"Rifle","category":"Primary","masterable":true,"components":[]}
    ]"#.to_vec()
}

#[test]
fn parses_canonical_items_and_prime_parent_components() {
    let catalog = CatalogIndex::from_wfcd_json(&wfcd_fixture()).unwrap();

    let prime = catalog
        .resolve("/Lotus/Types/Recipes/Weapons/WeaponParts/PrimeBowString")
        .unwrap();
    assert_eq!(prime.name(), "Paris Prime String");
    assert_eq!(prime.category(), Some(Category::PrimePart));
    assert!(!prime.masterable());
    assert_eq!(prime.image_name(), Some("PrimeBowString.png"));
    assert_eq!(prime.ducats(), 15);
    assert!(
        catalog
            .reward_entries()
            .iter()
            .any(|entry| entry.name == "Paris Prime String" && entry.ducats == 15)
    );

    let parent = catalog
        .resolve("/Lotus/Weapons/Tenno/Bows/PrimeHuntingBow")
        .unwrap();
    assert_eq!(parent.image_name(), Some("ParisPrime.png"));

    let qualified = catalog
        .resolve("/Lotus/Types/Recipes/Kubrow/Collars/PrimeKubrowCollarABandComponent")
        .unwrap();
    assert_eq!(qualified.name(), "Kavasa Prime Band");
    let generic = catalog
        .resolve("/Lotus/Types/Recipes/Kubrow/Collars/PrimeKubrowCollarABlueprint")
        .unwrap();
    assert_eq!(generic.name(), "Kavasa Prime Kubrow Collar Blueprint");

    let resource = catalog
        .resolve("/Lotus/Types/Items/MiscItems/ArgonCrystal")
        .unwrap();
    assert_eq!(resource.name(), "Argon Crystal");
    assert_eq!(resource.category(), Some(Category::Resource));

    let alertium = catalog
        .resolve("/Lotus/Types/Items/MiscItems/Alertium")
        .unwrap();
    assert_eq!(alertium.name(), "Nitain Extract");
    assert_eq!(alertium.category(), None);
    assert_eq!(alertium.image_name(), Some("Alertium.png"));

    let companion = catalog
        .resolve("/Lotus/Types/Friendly/Pets/CatbrowPetPowerSuit")
        .unwrap();
    assert_eq!(companion.name(), "Smeeta Kavat");
    assert_eq!(companion.category(), Some(Category::Companion));
    assert_eq!(companion.max_rank(), 30);

    for path in [
        "/Lotus/Types/Game/CatbrowPet/MirrorCatbrowPetPowerSuit",
        "/Lotus/Types/Friendly/Pets/CatbrowPetPowerSuit",
    ] {
        assert_eq!(
            catalog.resolve(path).unwrap().category(),
            Some(Category::Companion)
        );
    }
    for path in [
        "/Lotus/Weapons/Tenno/Archwing/Primary/NokkoArchGun/NokkoArchGun",
        "/Lotus/Weapons/Tenno/Archwing/Melee/ArchSwordHook/ArchHookSwordWeapon",
        "/Lotus/Weapons/SolarisUnited/Secondary/SUModularSecondarySet1/Barrel/SUModularSecondaryBarrelAPart",
        "/Lotus/Weapons/Ostron/Melee/ModularMelee01/Tip/TipOne",
    ] {
        let metadata = catalog.resolve(path).unwrap();
        assert_eq!(metadata.category(), Some(Category::Weapon));
        assert!(metadata.masterable());
    }
    let kdrive = catalog
        .resolve("/Lotus/Types/Vehicles/Hoverboard/HoverboardParts/PartComponents/HoverboardSolarisA/HoverboardSolarisADeck")
        .unwrap();
    assert_eq!(kdrive.category(), Some(Category::Vehicle));
    assert!(kdrive.masterable());

    assert_eq!(
        catalog
            .resolve("/Lotus/Weapons/Tenno/Rifle/Braton")
            .unwrap()
            .max_rank(),
        30
    );
    for path in [
        "/Lotus/Weapons/Orokin/BallasSword/BallasSwordWeapon",
        "/Lotus/Weapons/Grineer/KuvaLich/LongGuns/KuvaChakkhurr",
    ] {
        assert_eq!(catalog.resolve(path).unwrap().max_rank(), 40);
    }
}

#[test]
fn catalog_decoder_classifies_sold_xp_items_and_uses_catalog_rank() {
    let catalog = CatalogIndex::from_wfcd_json(&wfcd_fixture()).unwrap();
    let payload = br#"{
      "LastInventorySync":{"$date":{"$numberLong":"1753392000000"}},
      "Suits":[],"LongGuns":[],"Pistols":[],"Melee":[],"Sentinels":[],
      "MiscItems":[{"ItemType":"/Lotus/Types/Recipes/Weapons/WeaponParts/PrimeBowString","ItemCount":1}],
      "Recipes":[],"PendingRecipes":[],"SpaceSuits":[],"SpaceMelee":[],"SpaceGuns":[],
      "SentinelWeapons":[],"KubrowPets":[],"OperatorAmps":[],"MechSuits":[],
      "XPInfo":[
        {"ItemType":"/Lotus/Types/Friendly/Pets/CatbrowPetPowerSuit","XP":900000},
        {"ItemType":"/Lotus/Weapons/Orokin/BallasSword/BallasSwordWeapon","XP":799999},
        {"ItemType":"/Lotus/Unknown/SoldItem","XP":999999999}
      ]
    }"#;
    let snapshot = InventoryJsonDecoder::with_catalog(&catalog)
        .decode(payload)
        .unwrap();
    let by_id = |id: &str| {
        snapshot
            .entries()
            .iter()
            .find(|entry| entry.item.id.as_str() == id)
            .unwrap()
    };

    let catbrow = by_id("/Lotus/Types/Friendly/Pets/CatbrowPetPowerSuit");
    assert_eq!(catbrow.item.name, "Smeeta Kavat");
    assert_eq!(catbrow.item.category, Category::Companion);
    assert!(catbrow.mastered);

    let paracesis = by_id("/Lotus/Weapons/Orokin/BallasSword/BallasSwordWeapon");
    assert_eq!(paracesis.item.name, "Paracesis");
    assert!(!paracesis.mastered);

    let unknown = by_id("/Lotus/Unknown/SoldItem");
    assert!(!unknown.mastered);

    let prime = by_id("/Lotus/Types/Recipes/Weapons/WeaponParts/PrimeBowString");
    assert_eq!(prime.item.name, "Paris Prime String");
    assert_eq!(prime.item.category, Category::PrimePart);
}

#[test]
fn unclassified_catalog_items_keep_artwork_and_the_inventory_section_category() {
    let catalog = CatalogIndex::from_wfcd_json(&wfcd_fixture()).unwrap();
    let payload = br#"{
      "LastInventorySync":{"$date":{"$numberLong":"1753392000000"}},
      "Suits":[],"LongGuns":[],"Pistols":[],"Melee":[],"Sentinels":[],
      "MiscItems":[{"ItemType":"/Lotus/Types/Items/MiscItems/Alertium","ItemCount":3}],
      "Recipes":[],"PendingRecipes":[],"SpaceSuits":[],"SpaceMelee":[],"SpaceGuns":[],
      "SentinelWeapons":[],"KubrowPets":[],"OperatorAmps":[],"MechSuits":[],"XPInfo":[]
    }"#;
    let snapshot = InventoryJsonDecoder::with_catalog(&catalog)
        .decode(payload)
        .unwrap();
    let alertium = &snapshot.entries()[0];
    assert_eq!(alertium.item.name, "Nitain Extract");
    assert_eq!(alertium.item.category, Category::Resource);
    assert_eq!(alertium.item.image_name.as_deref(), Some("Alertium.png"));
}

#[test]
fn non_inventory_records_do_not_invalidate_the_aggregate_catalog() {
    let catalog = CatalogIndex::from_wfcd_json(
        br#"[
          {"uniqueName":"SolNode203","name":"Abaddon","type":"Node","category":"Missions"},
          {"uniqueName":"/Lotus/Types/Items/MiscItems/Alertium","name":"Nitain Extract","type":"Misc","category":"Misc","imageName":"Alertium.png"}
        ]"#,
    )
    .unwrap();

    assert!(catalog.resolve("SolNode203").is_none());
    assert_eq!(
        catalog
            .resolve("/Lotus/Types/Items/MiscItems/Alertium")
            .unwrap()
            .image_name(),
        Some("Alertium.png")
    );
}

#[test]
fn unsafe_artwork_names_are_omitted_without_discarding_the_catalog() {
    let catalog = CatalogIndex::from_wfcd_json(
        br#"[{"uniqueName":"/Lotus/Types/Items/MiscItems/Retreat","name":"Retreat","type":"Misc","category":"Misc","imageName":"NewLokaAmaryn'sRetreat.png"}]"#,
    )
    .unwrap();

    assert_eq!(
        catalog
            .resolve("/Lotus/Types/Items/MiscItems/Retreat")
            .unwrap()
            .image_name(),
        None
    );
}
