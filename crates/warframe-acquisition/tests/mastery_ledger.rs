mod common;

use std::collections::BTreeMap;

use common::recipes::*;
use warframe_acquisition::{CatalogIndex, Holdings, MasteryEvidence, MasteryFacts, MasteryLedger};
use warframe_domain::{KioskMastery, MasteryMark, SetPart};

fn holdings(quantity: &[(&str, u32)], mastered: &[&str]) -> Holdings {
    Holdings::from_parts(
        quantity
            .iter()
            .map(|(p, q)| ((*p).to_owned(), *q))
            .collect(),
        mastered.iter().map(|p| (*p).to_owned()).collect(),
    )
}
fn live(xp: &[(&str, u64)], pending: &[(&str, u32)]) -> MasteryFacts {
    MasteryFacts::from_parts(
        xp.iter()
            .map(|(p, x)| ((*p).to_owned(), *x))
            .collect::<BTreeMap<_, _>>(),
        pending
            .iter()
            .map(|(p, n)| ((*p).to_owned(), *n))
            .collect::<BTreeMap<_, _>>(),
    )
}
fn part(name: &str, image: &str, uses: u32, held: u32, this: bool) -> SetPart {
    SetPart {
        name: name.into(),
        image: Some(image.into()),
        uses,
        held,
        this,
    }
}

#[test]
fn a_part_of_an_unbuilt_unmastered_item_is_missing() {
    let catalog = catalog();
    let held = holdings(&[(GUANDAO_BP, 1), (GUANDAO_HANDLE, 1)], &[]);
    let facts = live(&[], &[]);
    let mark = MasteryLedger::new(&catalog, &held, Some(&facts)).reward_mark("Guandao Prime Blade");
    assert_eq!(
        mark,
        Some(MasteryMark::Unmastered {
            subject: None,
            parts: vec![
                part("Blueprint", "blueprint.png", 1, 1, false),
                part("Blade", "GenericWeaponPrimeBlade.png", 2, 0, true),
                part("Handle", "GenericWeaponPrimeHandle.png", 1, 1, false),
            ],
            missing: true,
            completes: false, // two Blades still short, this reward is one
        })
    );
}

#[test]
fn the_second_copy_of_a_two_per_build_part_completes_the_set() {
    let catalog = catalog();
    let held = holdings(
        &[(GUANDAO_BP, 1), (GUANDAO_HANDLE, 1), (GUANDAO_BLADE, 1)],
        &[],
    );
    let facts = live(&[], &[]);
    let Some(MasteryMark::Unmastered {
        missing, completes, ..
    }) = MasteryLedger::new(&catalog, &held, Some(&facts)).reward_mark("Guandao Prime Blade")
    else {
        panic!("expected unmastered")
    };
    assert!(missing && completes);
}

#[test]
fn a_blueprint_reward_counts_the_crafted_component_and_a_pending_build_as_held() {
    let catalog = catalog();
    let crafted = holdings(&[(REVENANT_CHASSIS, 1)], &[]);
    let facts = live(&[], &[]);
    let ledger = MasteryLedger::new(&catalog, &crafted, Some(&facts));
    let Some(MasteryMark::Unmastered { missing, .. }) =
        ledger.reward_mark("Revenant Prime Chassis Blueprint")
    else {
        panic!("expected unmastered")
    };
    assert!(!missing, "a crafted Chassis is a held Chassis");

    let none = holdings(&[(REVENANT_BP, 1)], &[]);
    let building = live(&[], &[(REVENANT_CHASSIS_BP, 1)]);
    let Some(MasteryMark::Unmastered { missing, .. }) =
        MasteryLedger::new(&catalog, &none, Some(&building))
            .reward_mark("Revenant Prime Chassis Blueprint")
    else {
        panic!("expected unmastered")
    };
    assert!(!missing, "a Chassis in the foundry is a held Chassis");
}

#[test]
fn the_last_missing_warframe_part_completes_the_set() {
    let catalog = catalog();
    let held = holdings(
        &[
            (REVENANT_BP, 1),
            (REVENANT_NEUROPTICS_BP, 1),
            (REVENANT_SYSTEMS, 1),
        ],
        &[],
    );
    let facts = live(&[], &[]);
    let Some(MasteryMark::Unmastered {
        missing, completes, ..
    }) = MasteryLedger::new(&catalog, &held, Some(&facts))
        .reward_mark("Revenant Prime Chassis Blueprint")
    else {
        panic!("expected unmastered")
    };
    assert!(missing && completes);
}

#[test]
fn a_parent_that_is_mastered_built_or_in_the_foundry_says_so() {
    let catalog = catalog();
    let facts = live(&[(GUANDAO, 98_000)], &[]);
    let mastered = holdings(&[], &[GUANDAO]);
    assert_eq!(
        MasteryLedger::new(&catalog, &mastered, Some(&facts)).reward_mark("Guandao Prime Blade"),
        Some(MasteryMark::Mastered)
    );
    let built = holdings(&[(GUANDAO, 1)], &[]);
    assert_eq!(
        MasteryLedger::new(&catalog, &built, Some(&facts)).reward_mark("Guandao Prime Blade"),
        Some(MasteryMark::Built {
            rank: 14,
            max_rank: 30
        }) // 98 000 / 500 = 196, √196 = 14
    );
    let empty = holdings(&[], &[]);
    let foundry = live(&[], &[(GUANDAO_BP, 1)]);
    assert_eq!(
        MasteryLedger::new(&catalog, &empty, Some(&foundry)).reward_mark("Guandao Prime Blade"),
        Some(MasteryMark::InFoundry)
    );
}

#[test]
fn a_parent_both_built_and_in_the_foundry_reads_built() {
    let catalog = catalog();
    let held = holdings(&[(GUANDAO, 1)], &[]);
    let facts = live(&[(GUANDAO, 98_000)], &[(GUANDAO_BP, 1)]);
    assert_eq!(
        MasteryLedger::new(&catalog, &held, Some(&facts)).reward_mark("Guandao Prime Blade"),
        Some(MasteryMark::Built {
            rank: 14,
            max_rank: 30
        })
    );
}

#[test]
fn a_mastered_ingredient_still_counts_toward_its_unmastered_consumer() {
    let catalog = catalog();
    let held = holdings(&[(AKBRONCO_BP, 1)], &[BRONCO]);
    let facts = live(&[], &[]);
    let ledger = MasteryLedger::new(&catalog, &held, Some(&facts));
    assert_eq!(
        ledger.reward_mark("Bronco Prime Barrel"),
        Some(MasteryMark::Unmastered {
            subject: Some("Akbronco".into()),
            parts: vec![
                part("Blueprint", "blueprint.png", 1, 1, false),
                part("Link", "GenericComponentPrimeLink.png", 1, 0, false),
                part("Bronco Prime", "bronco-prime.png", 2, 0, true),
            ],
            missing: true,
            completes: false, // the chain view never claims a set completes
        })
    );
    assert_eq!(
        ledger.kiosk_mastery("Bronco Prime Barrel"),
        None,
        "Bronco Prime itself is mastered, so its parts get no kiosk strip"
    );
}

#[test]
fn an_ingredient_is_mastered_only_when_its_consumers_are() {
    let catalog = catalog();
    let facts = live(&[], &[]);
    let both = holdings(&[], &[BRONCO, AKBRONCO]);
    assert_eq!(
        MasteryLedger::new(&catalog, &both, Some(&facts)).reward_mark("Bronco Prime Barrel"),
        Some(MasteryMark::Mastered)
    );
}

#[test]
fn an_unmastered_ingredient_needed_twice_asks_for_two_builds_of_its_own_parts() {
    let catalog = catalog();
    let held = holdings(&[(BRONCO_BARREL, 1)], &[]);
    let facts = live(&[], &[]);
    let Some(MasteryMark::Unmastered {
        subject,
        parts,
        missing,
        ..
    }) = MasteryLedger::new(&catalog, &held, Some(&facts)).reward_mark("Bronco Prime Barrel")
    else {
        panic!("expected unmastered")
    };
    assert_eq!(subject, None);
    assert_eq!(
        parts.iter().map(|p| p.uses).collect::<Vec<_>>(),
        vec![2, 2, 2]
    );
    assert!(missing, "one Barrel held, two builds want two");
}

#[test]
fn a_built_ingredient_still_short_for_its_consumer_shows_the_consumer() {
    let catalog = catalog();
    let held = holdings(&[(BRONCO, 1)], &[]);
    let facts = live(&[(BRONCO, 50_000)], &[]);
    let Some(MasteryMark::Unmastered {
        subject,
        parts,
        missing,
        ..
    }) = MasteryLedger::new(&catalog, &held, Some(&facts)).reward_mark("Bronco Prime Barrel")
    else {
        panic!("expected the Akbronco view")
    };
    assert_eq!(subject.as_deref(), Some("Akbronco"));
    let bronco = parts.iter().find(|p| p.this).expect("this slot");
    assert_eq!((bronco.held, bronco.uses), (1, 2));
    assert!(
        missing,
        "Akbronco needs a second Bronco, so Bronco parts still matter"
    );
}

#[test]
fn a_mastered_ingredient_held_once_is_still_short_for_two_open_consumers() {
    const SHARED: &str = "/Lotus/Weapons/Tenno/Pistol/SharedPrime";
    let catalog = CatalogIndex::from_wfcd_json(
        br#"[
          {"uniqueName": "/Lotus/Weapons/Tenno/Pistol/SharedPrime", "name": "Shared Prime",
           "category": "Secondary", "type": "Pistol", "masterable": true, "components": [
             {"uniqueName": "/Lotus/Types/Recipes/Weapons/SharedPrimeBlueprint", "name": "Blueprint", "itemCount": 1, "tradable": true, "ducats": 15},
             {"uniqueName": "/Lotus/Types/Recipes/Weapons/WeaponParts/SharedPrimeBarrel", "name": "Barrel", "itemCount": 1, "tradable": true, "ducats": 15}
           ]},
          {"uniqueName": "/Lotus/Weapons/Tenno/Pistol/FirstPrime", "name": "First Prime",
           "category": "Secondary", "type": "Pistol", "masterable": true, "components": [
             {"uniqueName": "/Lotus/Types/Recipes/Weapons/FirstPrimeBlueprint", "name": "Blueprint", "itemCount": 1, "tradable": true, "ducats": 45},
             {"uniqueName": "/Lotus/Weapons/Tenno/Pistol/SharedPrime", "name": "Shared Prime", "itemCount": 1, "tradable": false}
           ]},
          {"uniqueName": "/Lotus/Weapons/Tenno/Pistol/SecondPrime", "name": "Second Prime",
           "category": "Secondary", "type": "Pistol", "masterable": true, "components": [
             {"uniqueName": "/Lotus/Types/Recipes/Weapons/SecondPrimeBlueprint", "name": "Blueprint", "itemCount": 1, "tradable": true, "ducats": 45},
             {"uniqueName": "/Lotus/Weapons/Tenno/Pistol/SharedPrime", "name": "Shared Prime", "itemCount": 1, "tradable": false}
           ]}
        ]"#,
    )
    .expect("two-consumer fixture parses");
    let held = holdings(&[(SHARED, 1)], &[SHARED]);
    let facts = live(&[], &[]);
    let mark = MasteryLedger::new(&catalog, &held, Some(&facts)).reward_mark("Shared Prime Barrel");
    let Some(MasteryMark::Unmastered { subject, .. }) = mark else {
        panic!("each consumer takes one copy, so the one held cannot feed both: {mark:?}")
    };
    assert_eq!(
        subject.as_deref(),
        Some("First"),
        "neither consumer is short on its own, so the first one is named"
    );
}

#[test]
fn saved_evidence_claims_only_full_mastery() {
    let catalog = catalog();
    let held = holdings(&[(REVENANT_BP, 1)], &[GUANDAO, BRONCO]);
    let ledger = MasteryLedger::new(&catalog, &held, None);
    assert_eq!(ledger.evidence(), MasteryEvidence::Saved);
    assert_eq!(
        ledger.reward_mark("Guandao Prime Blade"),
        Some(MasteryMark::Mastered)
    );
    assert_eq!(
        ledger.reward_mark("Revenant Prime Chassis Blueprint"),
        Some(MasteryMark::Unknown)
    );
    assert_eq!(
        ledger.reward_mark("Bronco Prime Barrel"),
        Some(MasteryMark::Unknown),
        "Akbronco is not mastered, so Bronco parts may still be needed"
    );
}

#[test]
fn no_collection_means_unknown_and_non_mastery_items_get_no_mark() {
    let catalog = catalog();
    let empty = holdings(&[], &[]);
    let ledger = MasteryLedger::new(&catalog, &empty, None);
    assert_eq!(ledger.evidence(), MasteryEvidence::None);
    assert_eq!(
        ledger.reward_mark("Guandao Prime Blade"),
        Some(MasteryMark::Unknown)
    );
    assert_eq!(ledger.reward_mark("Forma Blueprint"), None);
    assert_eq!(ledger.reward_mark("Kavasa Prime Kubrow Collar Band"), None);
    assert_eq!(ledger.reward_mark("Not An Item"), None);
}

#[test]
fn kiosk_mastery_reports_held_and_uses_for_live_unmastered_parts_only() {
    let catalog = catalog();
    let held = holdings(&[(GUANDAO_BLADE, 1)], &[]);
    let facts = live(&[], &[]);
    assert_eq!(
        MasteryLedger::new(&catalog, &held, Some(&facts)).kiosk_mastery("Guandao Prime Blade"),
        Some(KioskMastery { held: 1, uses: 2 })
    );
    let mastered = holdings(&[(GUANDAO_BLADE, 1)], &[GUANDAO]);
    assert_eq!(
        MasteryLedger::new(&catalog, &mastered, Some(&facts)).kiosk_mastery("Guandao Prime Blade"),
        None
    );
    assert_eq!(
        MasteryLedger::new(&catalog, &held, None).kiosk_mastery("Guandao Prime Blade"),
        None,
        "kiosk strips need live evidence"
    );
}
