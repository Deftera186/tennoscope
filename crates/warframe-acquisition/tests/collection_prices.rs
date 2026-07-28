use warframe_acquisition::{PriceDumpError, PriceTable};

/// A trimmed copy of the real dump's shape: keyed by English name, one record per order type.
const DUMP: &str = r#"{
    "Mirage Prime Systems Blueprint": [
        {"order_type":"closed","median":18.0,"volume":4},
        {"order_type":"sell","median":20.0,"min_price":10,"volume":1127},
        {"order_type":"buy","median":12.0,"volume":172}
    ],
    "Axi A1 Relic": [
        {"order_type":"sell","median":20.0,"volume":30}
    ],
    "Serration": [
        {"order_type":"sell","median":50.0,"volume":12}
    ],
    "Zephyr Prime Chassis Blueprint": [
        {"order_type":"sell","median":27.5,"volume":9}
    ],
    "Bottomless Pit": [
        {"order_type":"buy","median":3.0,"volume":2}
    ]
}"#;

fn table() -> PriceTable {
    PriceTable::from_dump_json(DUMP.as_bytes(), "2026-07-27").expect("fixture parses")
}

#[test]
fn a_sell_median_becomes_the_price() {
    assert_eq!(table().price_for("Serration"), Some(50));
}

/// The dump names the blueprint; the catalog names the part. Neither is wrong, so both resolve.
#[test]
fn a_part_resolves_to_its_blueprint_listing() {
    assert_eq!(table().price_for("Mirage Prime Systems"), Some(20));
}

#[test]
fn a_blueprint_resolves_to_a_listing_without_the_suffix() {
    let dump = r#"{"Forma": [{"order_type":"sell","median":8.0,"volume":3}]}"#;
    let table = PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").unwrap();
    assert_eq!(table.price_for("Forma Blueprint"), Some(8));
}

/// The catalog names a relic by refinement, the market by relic. All four tiers share one price,
/// which understates a radiant relic and is the accepted cost of pricing relics at all.
#[test]
fn every_relic_refinement_resolves_to_the_one_relic_listing() {
    let table = table();
    for name in [
        "Axi A1 Intact",
        "Axi A1 Exceptional",
        "Axi A1 Flawless",
        "Axi A1 Radiant",
    ] {
        assert_eq!(table.price_for(name), Some(20), "for {name}");
    }
}

#[test]
fn a_median_is_rounded_to_whole_platinum() {
    assert_eq!(table().price_for("Zephyr Prime Chassis"), Some(28));
}

/// An item nobody is selling has no sell record. It is unpriced, not free.
#[test]
fn an_item_with_no_sell_listing_has_no_price() {
    assert_eq!(table().price_for("Bottomless Pit"), None);
}

#[test]
fn an_unknown_name_has_no_price() {
    assert_eq!(table().price_for("Not An Item"), None);
}

#[test]
fn the_table_reports_what_it_parsed() {
    let table = table();
    assert_eq!(table.dump_date(), "2026-07-27");
    assert_eq!(table.len(), 4, "the buy-only item is not a price");
}

/// The live path needs warframe.market's own name to build a slug from. No derivation from the
/// catalog's name would turn "Axi A1 Radiant" into "axi_a1_relic"; resolving through the dump does.
#[test]
fn resolution_yields_the_market_name_the_live_lookup_needs() {
    let table = table();
    assert_eq!(table.market_name("Axi A1 Radiant"), Some("Axi A1 Relic"));
    assert_eq!(
        table.market_name("Mirage Prime Systems"),
        Some("Mirage Prime Systems Blueprint")
    );
    assert_eq!(table.market_name("Serration"), Some("Serration"));
    assert_eq!(table.market_name("Not An Item"), None);
}

/// A truncated download must be rejected whole. Half a dump applied silently would halve the
/// reported worth of a collection with nothing to show that it had.
#[test]
fn a_malformed_dump_is_rejected_whole() {
    let truncated = &DUMP.as_bytes()[..DUMP.len() / 2];
    assert!(matches!(
        PriceTable::from_dump_json(truncated, "2026-07-27"),
        Err(PriceDumpError::Malformed)
    ));
}
