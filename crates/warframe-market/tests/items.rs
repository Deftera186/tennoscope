mod common;

use common::{FakeTransport, ok};
use warframe_market::{Listing, MarketError, MarketItems};

/// The shape of `/v2/items`, verified against the live endpoint. Trimmed from the real payload of
/// 3,838 entries (1.61 MB, measured 2026-08-22); every entry below is a real one, kept whole so
/// the shapes the resolver branches on are the shapes the market actually publishes.
const ITEMS: &str = r#"{"apiVersion":"0.25.0","data":[
    {"id":"54a73e65e779893a797fff33","slug":"braton_prime_blueprint",
     "gameRef":"/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint","tags":["weapon","prime"],
     "ducats":25,"i18n":{"en":{"name":"Braton Prime Blueprint"}}},
    {"id":"54ca39abe7798915c1c11e10","slug":"creeping_bullseye",
     "gameRef":"/Lotus/Upgrades/Mods/Pistol/DualStat/CorruptedCritChanceFireRatePistol",
     "tags":["mod"],"maxRank":5,"i18n":{"en":{"name":"Creeping Bullseye"}}},
    {"id":"5program0000000000000000","slug":"legendary_fusion_core",
     "tags":["fusion core"],"i18n":{"en":{"name":"Legendary Fusion Core"}}},
    {"id":"551085aee77989729e1416d0","slug":"arcane_barrier",
     "gameRef":"/Lotus/Upgrades/CosmeticEnhancers/Defensive/InstantShieldOnDamage",
     "tags":["arcane"],"maxRank":5,"bulkTradable":true,"i18n":{"en":{"name":"Arcane Barrier"}}},
    {"id":"57e91e65c76eb74c087f492f","slug":"ayatan_orta_sculpture",
     "gameRef":"/Lotus/Types/Items/FusionTreasures/OroFusexC","tags":["ayatan_sculpture"],
     "bulkTradable":true,"maxAmberStars":1,"maxCyanStars":3,
     "i18n":{"en":{"name":"Ayatan Orta Sculpture"}}},
    {"id":"54a74454e779892d5e5155ff","slug":"streamline",
     "gameRef":"/Lotus/Upgrades/Mods/Warframe/AvatarAbilityEfficiencyMod",
     "tags":["mod","warframe","rare"],"subtypes":["regular","atragraph"],"maxRank":5,
     "i18n":{"en":{"name":"Streamline"}}}
],"error":null}"#;

#[test]
fn an_item_id_resolves_to_the_collections_own_identity() {
    let items = MarketItems::from_response(ITEMS.as_bytes()).expect("items parse");

    assert_eq!(
        items.catalog_path("54a73e65e779893a797fff33"),
        Some("/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint")
    );
}

/// 35 of the entries measured on 2026-08-01 carried no `gameRef` (none did on 2026-08-22, but the
/// table is under no obligation to stay that way). They must resolve to nothing rather than to a
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
    assert_eq!(items.len(), 6);
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

    assert_eq!(items.len(), 6);
    let seen = transport.seen();
    assert!(seen[0].url.ends_with("/v2/items"), "url: {}", seen[0].url);
    // The item table is public. Sending the credential with it would spend the account's identity
    // on a request that does not need it.
    assert_eq!(seen[0].token, None);
}

/// Selling needs the opposite direction from reconciliation: the player picks a collection row,
/// and the market wants the listing that row becomes. For most of the collection that listing is
/// price and quantity and nothing else -- the row's path already names the item exactly.
#[test]
fn a_plain_collection_row_resolves_to_a_listing_with_no_context() {
    let items = MarketItems::from_response(ITEMS.as_bytes()).expect("items parse");

    assert_eq!(
        items.listing_for("/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint", false),
        Some(Listing {
            item_id: "54a73e65e779893a797fff33",
            rank: None,
            subtype: None,
            per_trade: None,
        })
    );
    assert_eq!(items.listing_for("/Lotus/Types/Nothing", false), None);
}

/// warframe.market quotes a mod at exactly two ranks -- unranked and fully ranked -- and the
/// create body has to say which. The unranked stack and a maxed copy are therefore the two rows
/// of a card that can list; a part-ranked copy is neither, and there is no rank the API would
/// accept for it, so it resolves to nothing rather than to a listing that would be refused.
#[test]
fn the_two_quoted_ranks_are_the_unranked_stack_and_a_maxed_copy() {
    let items = MarketItems::from_response(ITEMS.as_bytes()).expect("items parse");
    const MOD: &str = "/Lotus/Upgrades/Mods/Pistol/DualStat/CorruptedCritChanceFireRatePistol";

    // Still reconciled, still flaggable, still removable -- comparability is a different question.
    assert!(items.comparable("54ca39abe7798915c1c11e10"));

    assert_eq!(
        items.listing_for(MOD, false),
        Some(Listing {
            item_id: "54ca39abe7798915c1c11e10",
            rank: Some(0),
            subtype: None,
            per_trade: None,
        })
    );
    assert_eq!(
        items.listing_for(&format!("{MOD}#5"), true),
        Some(Listing {
            item_id: "54ca39abe7798915c1c11e10",
            rank: Some(5),
            subtype: None,
            per_trade: None,
        })
    );
    assert_eq!(items.listing_for(&format!("{MOD}#3"), false), None);
}

/// A relic is one market entry standing for four collection rows. The row names its refinement in
/// the game's own vocabulary -- a metal tier on the end of the path -- and the listing has to
/// translate that into the subtype the market expects, and declare the per-trade size that every
/// bulk-tradable demands. Measured 2026-08-22: all 772 relic entries publish the base projection
/// path with the four refinements as subtypes, all of them bulk-tradable.
#[test]
fn a_relic_row_resolves_to_its_refinement_subtype() {
    const RELIC: &str = r#"{"apiVersion":"0.25.0","data":[
        {"id":"6054dd685221e30057500f63","slug":"axi_a1_relic",
         "gameRef":"/Lotus/Types/Game/Projections/T4VoidProjectionE","tags":["relic","axi"],
         "bulkTradable":true,"subtypes":["intact","exceptional","flawless","radiant"],
         "i18n":{"en":{"name":"Axi A1 Relic"}}}
    ],"error":null}"#;
    let items = MarketItems::from_response(RELIC.as_bytes()).expect("items parse");
    const BASE: &str = "/Lotus/Types/Game/Projections/T4VoidProjectionE";

    // One market entry for four rows: the refinement is what tells them apart.
    assert!(!items.comparable("6054dd685221e30057500f63"));

    assert_eq!(
        items.listing_for(&format!("{BASE}Bronze"), false),
        Some(Listing {
            item_id: "6054dd685221e30057500f63",
            rank: None,
            subtype: Some("intact"),
            per_trade: Some(1),
        })
    );
    assert_eq!(
        items.listing_for(&format!("{BASE}Platinum"), false),
        Some(Listing {
            item_id: "6054dd685221e30057500f63",
            rank: None,
            subtype: Some("radiant"),
            per_trade: Some(1),
        })
    );
    // The base path names no refinement, and a listing has to name one.
    assert_eq!(items.listing_for(BASE, false), None);
    // A suffix that is not a tier is not a refinement this table knows.
    assert_eq!(items.listing_for(&format!("{BASE}Copper"), false), None);
}

/// An arcane is ranked and bulk-tradable at once -- every arcane entry measured on 2026-08-22
/// carries both -- so its listing declares a rank and a per-trade size together. The two
/// dimensions are independent, and neither may be assumed to imply the other.
#[test]
fn an_arcane_declares_its_rank_and_its_trade_size_together() {
    let items = MarketItems::from_response(ITEMS.as_bytes()).expect("items parse");
    const ARCANE: &str = "/Lotus/Upgrades/CosmeticEnhancers/Defensive/InstantShieldOnDamage";

    assert_eq!(
        items.listing_for(ARCANE, false),
        Some(Listing {
            item_id: "551085aee77989729e1416d0",
            rank: Some(0),
            subtype: None,
            per_trade: Some(1),
        })
    );
    assert_eq!(
        items.listing_for(&format!("{ARCANE}#5"), true),
        Some(Listing {
            item_id: "551085aee77989729e1416d0",
            rank: Some(5),
            subtype: None,
            per_trade: Some(1),
        })
    );
}

/// An Ayatan sculpture's listing has to say how many stars are socketed, and nothing in a
/// collection row knows that. Refused here rather than published as an empty sculpture the player
/// may not be holding.
#[test]
fn a_sculpture_whose_stars_are_unknown_resolves_to_nothing() {
    let items = MarketItems::from_response(ITEMS.as_bytes()).expect("items parse");

    assert_eq!(
        items.listing_for("/Lotus/Types/Items/FusionTreasures/OroFusexC", false),
        None
    );
}

/// 19 mods publish one entry under two subtypes -- the card and its atragraph variant -- with a
/// single `gameRef` between them. The path alone cannot say which variant a row holds, and a
/// listing that guessed would publish against something the player did not choose.
#[test]
fn a_path_shared_by_subtypes_that_are_not_refinements_resolves_to_nothing() {
    let items = MarketItems::from_response(ITEMS.as_bytes()).expect("items parse");

    assert_eq!(
        items.listing_for(
            "/Lotus/Upgrades/Mods/Warframe/AvatarAbilityEfficiencyMod",
            false
        ),
        None
    );
}
