use warframe_acquisition::{CatalogIndex, InventoryJsonDecoder, SnapshotDecoder};
use warframe_domain::Category;

fn wfcd_fixture() -> Vec<u8> {
    br#"[
      {
        "uniqueName":"/Lotus/Weapons/Tenno/Bows/PrimeHuntingBow",
        "name":"Paris Prime","type":"Bow","category":"Primary","masterable":true,
        "components":[
          {"uniqueName":"/Lotus/Types/Recipes/Weapons/WeaponParts/PrimeBowString","name":"String","tradable":true,"ducats":15,"primeSellingPrice":15},
          {"uniqueName":"/Lotus/Types/Items/MiscItems/Ferrite","name":"Ferrite","tradable":false}
        ]
      },
      {"uniqueName":"/Lotus/Types/Items/MiscItems/ArgonCrystal","name":"Argon Crystal","type":"Resource","category":"Resources","masterable":false,"components":[]},
      {"uniqueName":"/Lotus/Types/Friendly/Pets/CatbrowPetPowerSuit","name":"Smeeta Kavat","type":"Kavat","category":"Companions","masterable":true,"components":[]},
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
    assert_eq!(prime.category(), Category::PrimePart);
    assert!(!prime.masterable());

    let resource = catalog
        .resolve("/Lotus/Types/Items/MiscItems/ArgonCrystal")
        .unwrap();
    assert_eq!(resource.name(), "Argon Crystal");
    assert_eq!(resource.category(), Category::Resource);

    let companion = catalog
        .resolve("/Lotus/Types/Friendly/Pets/CatbrowPetPowerSuit")
        .unwrap();
    assert_eq!(companion.name(), "Smeeta Kavat");
    assert_eq!(companion.category(), Category::Companion);
    assert_eq!(companion.max_rank(), 30);

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
