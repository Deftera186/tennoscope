use warframe_acquisition::{CatalogIndex, RelicRewardIndex};

#[test]
fn loaded_projection_paths_resolve_to_deduplicated_reward_needles() {
    let relics = RelicRewardIndex::from_wfcd_json(
        br#"[
          {"uniqueName":"/Lotus/Types/Game/Projections/TestABronze","name":"Lith A1 Intact","rewards":[
            {"item":{"name":"Perigale Prime Receiver"}},
            {"item":{"name":"Forma Blueprint"}}
          ]},
          {"uniqueName":"/Lotus/Types/Game/Projections/TestBBronze","name":"Lith B1 Intact","rewards":[
            {"item":{"name":"Perigale Prime Receiver"}},
            {"item":{"name":"Burston Prime Receiver"}}
          ]}
        ]"#,
    )
    .unwrap();
    let catalog = CatalogIndex::from_wfcd_json(
        br#"[
          {"uniqueName":"/Lotus/Types/Recipes/PerigalePrimeReceiver","name":"Perigale Prime Receiver","type":"PrimePart","category":"PrimeParts"},
          {"uniqueName":"/Lotus/Types/Recipes/BurstonPrimeReceiver","name":"Burston Prime Receiver","type":"PrimePart","category":"PrimeParts"},
          {"uniqueName":"/Lotus/Types/Recipes/FormaBlueprint","name":"Forma Blueprint","type":"Blueprint","category":"Blueprints"}
        ]"#,
    )
    .unwrap();

    let candidates = relics.candidates_for_projection_paths(
        &[
            "/Lotus/Types/Game/Projections/TestABronze".into(),
            "/Lotus/Types/Game/Projections/TestBBronze".into(),
        ],
        &catalog,
    );

    assert_eq!(
        candidates
            .iter()
            .map(|candidate| candidate.choice_name())
            .collect::<Vec<_>>(),
        vec![
            "Burston Prime Receiver",
            "Forma Blueprint",
            "Perigale Prime Receiver"
        ]
    );
    assert!(
        candidates
            .iter()
            .all(|candidate| candidate.internal_paths().len() == 1)
    );
}

#[test]
fn unrelated_projection_paths_produce_no_candidates() {
    let relics = RelicRewardIndex::from_wfcd_json(br#"[]"#).unwrap();
    let catalog = CatalogIndex::default();

    assert!(
        relics
            .candidates_for_projection_paths(&["/Lotus/Unknown".into()], &catalog)
            .is_empty()
    );
}
