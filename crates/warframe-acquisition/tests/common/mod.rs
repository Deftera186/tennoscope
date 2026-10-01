//! Shared test helpers for the acquisition integration tests.
#![allow(dead_code)]

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// A log-crate logger that appends formatted lines to a file, so instrumented
/// scans keep an evidence trail that assertions can read back. One file per
/// test binary; installed at most once per process.
pub fn install_test_logger() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let path = std::env::var_os("TENNOSCOPE_TEST_LOG")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("tennoscope-test.log"));
        let file = Mutex::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .expect("test log opens"),
        );
        log::set_boxed_logger(Box::new(TestLogger(file))).expect("logger installs once");
        log::set_max_level(log::LevelFilter::Debug);
    });
}

struct TestLogger(Mutex<std::fs::File>);

impl log::Log for TestLogger {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        if let Ok(mut file) = self.0.lock() {
            let _ = writeln!(file, "{} {}", record.level(), record.args());
        }
    }
    fn flush(&self) {}
}

/// Every shape marks meet: a part used twice (Guandao Blade), blueprint rewards (Revenant), an
/// ingredient WFCD lists twice (Bronco into Akbronco), and an unmasterable Prime (Kavasa).
#[allow(dead_code)]
pub mod recipes {
    pub const GUANDAO: &str = "/Lotus/Weapons/Tenno/Melee/Polearms/GuandaoPrime";
    pub const GUANDAO_BP: &str = "/Lotus/Types/Recipes/Weapons/GuandaoPrimeBlueprint";
    pub const GUANDAO_BLADE: &str = "/Lotus/Types/Recipes/Weapons/WeaponParts/GuandaoPrimeBlade";
    pub const GUANDAO_HANDLE: &str = "/Lotus/Types/Recipes/Weapons/WeaponParts/GuandaoPrimeHandle";
    pub const REVENANT: &str = "/Lotus/Powersuits/Revenant/RevenantPrime";
    pub const REVENANT_BP: &str = "/Lotus/Types/Recipes/WarframeRecipes/RevenantPrimeBlueprint";
    pub const REVENANT_CHASSIS: &str =
        "/Lotus/Types/Recipes/WarframeRecipes/RevenantPrimeChassisComponent";
    pub const REVENANT_CHASSIS_BP: &str =
        "/Lotus/Types/Recipes/WarframeRecipes/RevenantPrimeChassisBlueprint";
    pub const REVENANT_NEUROPTICS: &str =
        "/Lotus/Types/Recipes/WarframeRecipes/RevenantPrimeHelmetComponent";
    pub const REVENANT_NEUROPTICS_BP: &str =
        "/Lotus/Types/Recipes/WarframeRecipes/RevenantPrimeHelmetBlueprint";
    pub const REVENANT_SYSTEMS: &str =
        "/Lotus/Types/Recipes/WarframeRecipes/RevenantPrimeSystemsComponent";
    pub const BRONCO: &str = "/Lotus/Weapons/Tenno/Pistol/BroncoPrime";
    pub const BRONCO_BP: &str = "/Lotus/Types/Recipes/Weapons/BroncoPrimeBlueprint";
    pub const BRONCO_BARREL: &str = "/Lotus/Types/Recipes/Weapons/WeaponParts/BroncoPrimeBarrel";
    pub const BRONCO_RECEIVER: &str =
        "/Lotus/Types/Recipes/Weapons/WeaponParts/BroncoPrimeReceiver";
    pub const AKBRONCO: &str = "/Lotus/Weapons/Tenno/Pistol/AkbroncoPrime";
    pub const AKBRONCO_BP: &str = "/Lotus/Types/Recipes/Weapons/AkbroncoPrimeBlueprint";
    pub const AKBRONCO_LINK: &str = "/Lotus/Types/Recipes/Weapons/WeaponParts/AkbroncoPrimeLink";
    pub const KAVASA: &str = "/Lotus/Types/Game/KubrowPet/Collars/KavasaPrimeCollar";
    pub const KAVASA_BAND: &str = "/Lotus/Types/Recipes/Collars/KavasaPrimeBand";
    pub const OROKIN_CELL: &str = "/Lotus/Types/Items/MiscItems/OrokinCell";
    pub const FORMA_BP: &str = "/Lotus/Types/Recipes/Components/FormaBlueprint";

    pub fn json() -> String {
        serde_json::json!([
          {"uniqueName": GUANDAO, "name": "Guandao Prime", "category": "Melee", "type": "Melee",
           "masterable": true, "tradable": false, "masteryReq": 12, "maxLevelCap": 30,
           "components": [
             {"uniqueName": GUANDAO_BP, "name": "Blueprint", "itemCount": 1, "tradable": true, "ducats": 45, "imageName": "blueprint.png"},
             {"uniqueName": GUANDAO_BLADE, "name": "Blade", "itemCount": 2, "tradable": true, "ducats": 15, "imageName": "GenericWeaponPrimeBlade.png"},
             {"uniqueName": GUANDAO_HANDLE, "name": "Handle", "itemCount": 1, "tradable": true, "ducats": 100, "imageName": "GenericWeaponPrimeHandle.png"},
             {"uniqueName": OROKIN_CELL, "name": "Orokin Cell", "itemCount": 10, "tradable": false}
           ]},
          {"uniqueName": REVENANT, "name": "Revenant Prime", "category": "Warframes", "type": "Warframe",
           "masterable": true, "tradable": false, "masteryReq": 0, "maxLevelCap": 30,
           "components": [
             {"uniqueName": REVENANT_BP, "name": "Blueprint", "itemCount": 1, "tradable": true, "ducats": 100, "imageName": "blueprint.png"},
             {"uniqueName": REVENANT_CHASSIS, "name": "Chassis", "itemCount": 1, "tradable": true, "ducats": 15, "imageName": "GenericWarframePrimeChassis.png"},
             {"uniqueName": REVENANT_NEUROPTICS, "name": "Neuroptics", "itemCount": 1, "tradable": true, "ducats": 45, "imageName": "GenericWarframePrimeHelmet.png"},
             {"uniqueName": REVENANT_SYSTEMS, "name": "Systems", "itemCount": 1, "tradable": true, "ducats": 65, "imageName": "GenericWarframePrimeSystem.png"}
           ]},
          {"uniqueName": BRONCO, "name": "Bronco Prime", "category": "Secondary", "type": "Pistol",
           "masterable": true, "tradable": false, "masteryReq": 3, "maxLevelCap": 30,
           "components": [
             {"uniqueName": BRONCO_BP, "name": "Blueprint", "itemCount": 1, "tradable": true, "ducats": 15, "imageName": "blueprint.png"},
             {"uniqueName": BRONCO_BARREL, "name": "Barrel", "itemCount": 1, "tradable": true, "ducats": 15, "imageName": "GenericGunPrimeBarrel.png"},
             {"uniqueName": BRONCO_RECEIVER, "name": "Receiver", "itemCount": 1, "tradable": true, "ducats": 45, "imageName": "GenericGunPrimeReceiver.png"}
           ]},
          {"uniqueName": AKBRONCO, "name": "Akbronco Prime", "category": "Secondary", "type": "Pistol",
           "masterable": true, "tradable": false, "masteryReq": 8, "maxLevelCap": 30,
           "components": [
             {"uniqueName": AKBRONCO_BP, "name": "Blueprint", "itemCount": 1, "tradable": true, "ducats": 45, "imageName": "blueprint.png"},
             {"uniqueName": BRONCO, "name": "Bronco Prime", "itemCount": 1, "tradable": false, "imageName": "bronco-prime.png"},
             {"uniqueName": BRONCO, "name": "Bronco Prime", "itemCount": 1, "tradable": false, "imageName": "bronco-prime.png"},
             {"uniqueName": AKBRONCO_LINK, "name": "Link", "itemCount": 1, "tradable": true, "ducats": 45, "imageName": "GenericComponentPrimeLink.png"}
           ]},
          {"uniqueName": KAVASA, "name": "Kavasa Prime Kubrow Collar", "category": "Misc", "type": "Kubrow Collar",
           "masterable": false, "tradable": false,
           "components": [
             {"uniqueName": KAVASA_BAND, "name": "Band", "itemCount": 1, "tradable": true, "ducats": 45, "imageName": "GenericComponentPrimeBand.png"}
           ]},
          {"uniqueName": FORMA_BP, "name": "Forma Blueprint", "category": "Misc", "type": "Misc",
           "masterable": false, "tradable": true}
        ])
        .to_string()
    }

    pub fn catalog() -> warframe_acquisition::CatalogIndex {
        warframe_acquisition::CatalogIndex::from_wfcd_json(json().as_bytes())
            .expect("recipe fixture parses")
    }
}
