mod common;

use common::{FakeTransport, ok};
use warframe_market::{MarketError, MarketItems};

/// The shape of `/v2/items`, verified against the live endpoint on 2026-08-01. Trimmed to three
/// entries; the real payload carries 3,837 in 1.61 MB.
const ITEMS: &str = r#"{"apiVersion":"0.25.0","data":[
    {"id":"54a73e65e779893a797fff33","slug":"braton_prime_blueprint",
     "gameRef":"/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint","tags":["weapon","prime"],
     "ducats":25,"i18n":{"en":{"name":"Braton Prime Blueprint"}}},
    {"id":"54ca39abe7798915c1c11e10","slug":"creeping_bullseye",
     "gameRef":"/Lotus/Upgrades/Mods/Pistol/DualStat/CorruptedCritChanceFireRatePistol",
     "tags":["mod"],"maxRank":5,"i18n":{"en":{"name":"Creeping Bullseye"}}},
    {"id":"5program0000000000000000","slug":"legendary_fusion_core",
     "tags":["fusion core"],"i18n":{"en":{"name":"Legendary Fusion Core"}}}
],"error":null}"#;

#[test]
fn an_item_id_resolves_to_the_collections_own_identity() {
    let items = MarketItems::from_response(ITEMS.as_bytes()).expect("items parse");

    assert_eq!(
        items.catalog_path("54a73e65e779893a797fff33"),
        Some("/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint")
    );
}

/// 35 of 3,837 live entries carry no `gameRef`. They must resolve to nothing rather than to a
/// guess: an order for one of them is unverifiable, and the reconciliation rule depends on being
/// able to say so.
#[test]
fn an_item_with_no_game_reference_resolves_to_nothing() {
    let items = MarketItems::from_response(ITEMS.as_bytes()).expect("items parse");

    assert_eq!(items.catalog_path("5program0000000000000000"), None);
    // It is still known, so its name can be shown on the row.
    assert_eq!(
        items.name("5program0000000000000000"),
        Some("Legendary Fusion Core")
    );
}

#[test]
fn an_unknown_item_id_resolves_to_nothing() {
    let items = MarketItems::from_response(ITEMS.as_bytes()).expect("items parse");

    assert_eq!(items.catalog_path("not-an-item-id"), None);
    assert_eq!(items.name("not-an-item-id"), None);
}

#[test]
fn the_english_name_is_kept_for_display() {
    let items = MarketItems::from_response(ITEMS.as_bytes()).expect("items parse");

    assert_eq!(
        items.name("54ca39abe7798915c1c11e10"),
        Some("Creeping Bullseye")
    );
    assert_eq!(items.len(), 3);
}

#[test]
fn a_malformed_payload_is_rejected() {
    assert_eq!(
        MarketItems::from_response(b"not json").unwrap_err(),
        MarketError::Malformed
    );
}

#[test]
fn fetching_asks_the_items_route() {
    let transport = FakeTransport::new(vec![ok(ITEMS)]);

    let items = MarketItems::fetch(&transport).expect("items fetch");

    assert_eq!(items.len(), 3);
    let seen = transport.seen();
    assert!(seen[0].url.ends_with("/v2/items"), "url: {}", seen[0].url);
    // The item table is public. Sending the credential with it would spend the account's identity
    // on a request that does not need it.
    assert_eq!(seen[0].token, None);
}

/// Selling needs the opposite direction from reconciliation: the player picks a collection row,
/// and the market wants an id for it.
#[test]
fn a_collection_path_resolves_back_to_the_market_id_that_can_be_listed() {
    let items = MarketItems::from_response(ITEMS.as_bytes()).expect("items parse");

    assert_eq!(
        items.market_id_for_path("/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint"),
        Some("54a73e65e779893a797fff33")
    );
    assert_eq!(items.market_id_for_path("/Lotus/Types/Nothing"), None);
}

/// Only comparable entries answer. An incomparable path is one that does not name a single
/// collection row, so there is no single item it could list -- and posting a guess would publish a
/// listing against something the player did not choose.
#[test]
fn a_path_that_names_no_single_row_offers_nothing_to_list() {
    // A relic: one market entry standing for four refinements.
    const RELIC: &str = r#"{"apiVersion":"0.25.0","data":[
        {"id":"relic-id","slug":"lith_a1_relic","gameRef":"/Lotus/Types/Game/Projections/T1VoidProjectionBratonPrimeDBronze",
         "subtypes":["intact","exceptional","flawless","radiant"],"i18n":{"en":{"name":"Lith A1 Relic"}}}
    ],"error":null}"#;
    let items = MarketItems::from_response(RELIC.as_bytes()).expect("items parse");

    assert!(!items.comparable("relic-id"));
    assert_eq!(
        items
            .market_id_for_path("/Lotus/Types/Game/Projections/T1VoidProjectionBratonPrimeDBronze"),
        None
    );
}
