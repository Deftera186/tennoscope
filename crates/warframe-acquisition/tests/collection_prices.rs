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

/// Built equipment must not borrow its blueprint's price. Measured against a real 1,106-item
/// collection, appending " Blueprint" fired 25 times and was wrong 25 times: a mastered `Ash Prime`
/// priced at what somebody asks for the blueprint, an item you cannot sell at all. Every prime part
/// in that collection is in the dump under its own name and resolves by rule 1, so nothing is lost.
#[test]
fn built_equipment_is_not_priced_from_its_blueprint_listing() {
    let dump = r#"{
        "Ash Prime Blueprint": [{"order_type":"sell","median":14.0,"volume":22}],
        "Ash Prime Set": [{"order_type":"sell","median":110.0,"volume":8}]
    }"#;
    let table = PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").expect("fixture parses");

    assert_eq!(table.price_for("Ash Prime"), None);
    assert_eq!(table.market_name("Ash Prime"), None);
    assert_eq!(
        table.price_for("Ash Prime Blueprint"),
        Some(14),
        "the part the player can actually sell keeps its price"
    );
}

#[test]
fn a_blueprint_resolves_to_a_listing_without_the_suffix() {
    let dump = r#"{"Forma": [{"order_type":"sell","median":8.0,"volume":3}]}"#;
    let table = PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").unwrap();
    assert_eq!(table.price_for("Forma Blueprint"), Some(8));
}

/// The dump cannot be corrected for bulk listings, so a relic's daily median runs high — measured
/// at 1.5x on Axi A1. Relics are priced from a live sweep instead, and until that lands they have
/// no price rather than an inflated one.
#[test]
fn a_relic_is_not_priced_from_the_dump() {
    let table = table();
    assert_eq!(table.price_for("Axi A1 Radiant"), None);
    assert_eq!(table.price_for("Axi A1 Relic"), None);
}

/// Resolution still works, because the live sweep needs the market name to build its slug.
#[test]
fn a_relic_still_resolves_to_its_market_name() {
    assert_eq!(table().market_name("Axi A1 Radiant"), Some("Axi A1 Relic"));
}

#[test]
fn a_swept_relic_price_is_served_like_any_other() {
    let mut table = table();
    table.insert_live("Axi A1 Relic", 17);
    assert_eq!(table.price_for("Axi A1 Radiant"), Some(17));
    assert_eq!(table.price_for("Axi A1 Relic"), Some(17));
}

/// `REFINEMENTS` is a hand-written list of four suffixes; a test that only ever tries `Radiant`
/// cannot catch a typo or reordering in `Intact`, `Exceptional`, or `Flawless`. Resolution must
/// keep working for all four, because the live sweep builds its warframe.market slug from it, and
/// a swept price must reach all four the same way a dump price used to.
#[test]
fn every_refinement_tier_resolves_and_prices_alike() {
    let names = [
        "Axi A1 Intact",
        "Axi A1 Exceptional",
        "Axi A1 Flawless",
        "Axi A1 Radiant",
    ];
    for name in names {
        assert_eq!(
            table().market_name(name),
            Some("Axi A1 Relic"),
            "for {name}"
        );
    }

    let mut table = table();
    table.insert_live("Axi A1 Relic", 17);
    for name in names {
        assert_eq!(table.price_for(name), Some(17), "for {name}");
    }
}

#[test]
fn the_relics_needing_a_sweep_are_the_ones_the_dump_lists() {
    let names = table().relic_market_names();
    assert_eq!(names, vec!["Axi A1 Relic".to_owned()]);
}

#[test]
fn a_non_relic_is_still_priced_from_the_dump() {
    assert_eq!(table().price_for("Serration"), Some(50));
}

#[test]
fn a_median_is_rounded_to_whole_platinum() {
    assert_eq!(
        table().price_for("Zephyr Prime Chassis Blueprint"),
        Some(28)
    );
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
    assert_eq!(
        table.len(),
        3,
        "the buy-only item is not a price, and the relic is priced separately"
    );
}

/// The live path needs warframe.market's own name to build a slug from. No derivation from the
/// catalog's name would turn "Axi A1 Radiant" into "axi_a1_relic"; resolving through the dump does.
#[test]
fn resolution_yields_the_market_name_the_live_lookup_needs() {
    let table = table();
    assert_eq!(table.market_name("Axi A1 Radiant"), Some("Axi A1 Relic"));
    assert_eq!(
        table.market_name("Mirage Prime Systems Blueprint"),
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

use std::{cell::RefCell, collections::HashMap as Map};
use warframe_acquisition::{
    CollectionPriceSource, PriceFetch, civil_date, dump_is_current, latest_dump,
    relic_sweep_is_current,
};

struct FakeDumps {
    available: Map<String, String>,
    asked: RefCell<Vec<String>>,
}

impl FakeDumps {
    fn new(available: &[(&str, &str)]) -> Self {
        Self {
            available: available
                .iter()
                .map(|(date, body)| ((*date).to_owned(), (*body).to_owned()))
                .collect(),
            asked: RefCell::new(Vec::new()),
        }
    }
}

impl CollectionPriceSource for FakeDumps {
    fn fetch(&self, date: &str) -> Result<Vec<u8>, PriceFetch> {
        self.asked.borrow_mut().push(date.to_owned());
        self.available
            .get(date)
            .map(|body| body.as_bytes().to_vec())
            .ok_or(PriceFetch::Missing)
    }
}

/// 2026-07-29T00:00:00Z. The dumps lag: on that day the newest was dated the 27th.
const TODAY: u64 = 1_785_283_200;

#[test]
fn a_unix_time_becomes_the_dump_date_the_url_needs() {
    assert_eq!(civil_date(0), "1970-01-01");
    assert_eq!(civil_date(TODAY), "2026-07-29");
}

/// The dump for today usually does not exist yet, so asking only for today would price nothing.
#[test]
fn the_newest_available_dump_is_found_by_walking_back() {
    let source = FakeDumps::new(&[("2026-07-27", DUMP)]);

    let table = latest_dump(&source, TODAY).expect("an older dump is still a dump");

    assert_eq!(table.dump_date(), "2026-07-27");
    assert_eq!(
        source.asked.borrow().as_slice(),
        ["2026-07-29", "2026-07-28", "2026-07-27"],
        "each day is tried once, newest first"
    );
}

#[test]
fn the_newest_dump_wins_when_several_exist() {
    let source = FakeDumps::new(&[("2026-07-28", DUMP), ("2026-07-27", DUMP)]);

    assert_eq!(
        latest_dump(&source, TODAY).unwrap().dump_date(),
        "2026-07-28"
    );
}

/// Walking back forever would hammer a dead host on every start, and a week-old valuation is not
/// worth the requests it would cost to find.
#[test]
fn the_walk_back_gives_up_rather_than_searching_forever() {
    let source = FakeDumps::new(&[]);

    assert!(latest_dump(&source, TODAY).is_err());
    assert_eq!(source.asked.borrow().len(), 6, "today plus five days back");
}

/// A dump that parses but is empty is a bad dump, not a collection where nothing is worth anything.
#[test]
fn a_dump_with_no_prices_is_not_accepted() {
    let source = FakeDumps::new(&[("2026-07-29", "{}")]);

    assert!(latest_dump(&source, TODAY).is_err());
}

/// A dump older than yesterday is the only one worth 3.9 MB of network on a launch.
#[test]
fn a_dump_from_today_or_yesterday_is_not_downloaded_again() {
    assert!(
        dump_is_current("2026-07-29", TODAY),
        "today's dump is as new as it gets"
    );
    assert!(
        dump_is_current("2026-07-28", TODAY),
        "the feed lagged two days when this was measured, so yesterday's may be the newest there is"
    );
    assert!(
        !dump_is_current("2026-07-27", TODAY),
        "older than yesterday is worth one request to check"
    );
    assert!(!dump_is_current("", TODAY), "no date is not a fresh date");
}

/// The sweep shares the dump's clock rather than keeping one of its own: a fresh dump replaces the
/// whole table, so swept prices surviving in a loaded table are only as old as that table's dump.
#[test]
fn a_relic_sweep_is_current_exactly_when_its_dump_is() {
    let stale = table(); // dated 2026-07-27, already older than `dump_is_current` accepts at TODAY
    assert!(!relic_sweep_is_current(&stale, TODAY));

    let fresh = PriceTable::from_dump_json(DUMP.as_bytes(), "2026-07-29").expect("fixture parses");
    assert!(relic_sweep_is_current(&fresh, TODAY));
}

use warframe_acquisition::CollectionPriceCache;

#[test]
fn a_refreshed_table_is_readable_without_the_network() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cache = CollectionPriceCache::new(directory.path());
    let source = FakeDumps::new(&[("2026-07-27", DUMP)]);

    cache.refresh(&source, TODAY).expect("refresh stores");
    let cached = cache.load_cached().expect("a stored table is readable");

    assert_eq!(cached.price_for("Serration"), Some(50));
    assert_eq!(cached.dump_date(), "2026-07-27");
}

#[test]
fn an_empty_cache_directory_yields_no_table() {
    let directory = tempfile::tempdir().expect("temp dir");

    assert!(
        CollectionPriceCache::new(directory.path())
            .load_cached()
            .is_none()
    );
}

/// A failed refresh must leave yesterday's prices alone. Discarding them because a download failed
/// would turn a network blip into a collection that reads as worthless.
#[test]
fn a_failed_refresh_leaves_the_cached_prices_in_place() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cache = CollectionPriceCache::new(directory.path());
    cache
        .refresh(&FakeDumps::new(&[("2026-07-27", DUMP)]), TODAY)
        .expect("first refresh stores");

    assert!(cache.refresh(&FakeDumps::new(&[]), TODAY).is_err());

    let cached = cache.load_cached().expect("the old table survives");
    assert_eq!(cached.dump_date(), "2026-07-27");
    assert_eq!(cached.price_for("Serration"), Some(50));
}

#[test]
fn a_corrupt_cache_file_yields_no_table_rather_than_a_panic() {
    let directory = tempfile::tempdir().expect("temp dir");
    std::fs::write(
        directory.path().join("collection-prices.json"),
        b"{not json",
    )
    .expect("write corrupt cache");

    assert!(
        CollectionPriceCache::new(directory.path())
            .load_cached()
            .is_none()
    );
}

/// A swept price survives the cache round-trip, or every restart would cost another sweep.
#[test]
fn swept_relic_prices_survive_the_disk_cache() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cache = CollectionPriceCache::new(directory.path());
    let mut table = cache
        .refresh(&FakeDumps::new(&[("2026-07-27", DUMP)]), TODAY)
        .expect("refresh stores");
    table.insert_live("Axi A1 Relic", 17);
    cache.store_table(&table).expect("store");

    let reloaded = cache.load_cached().expect("a stored table is readable");
    assert_eq!(reloaded.price_for("Axi A1 Radiant"), Some(17));
}

#[test]
fn a_cache_write_failure_is_not_blamed_on_the_dump() {
    let directory = tempfile::tempdir().expect("temp dir");
    // Create a file where the cache directory needs to be, so create_dir_all fails
    let blocking_file = directory.path().join("collection-prices");
    std::fs::write(&blocking_file, b"").expect("write blocking file");

    let cache = CollectionPriceCache::new(&blocking_file);
    let source = FakeDumps::new(&[("2026-07-27", DUMP)]);

    let result = cache.refresh(&source, TODAY);
    assert!(matches!(result, Err(PriceDumpError::CacheWrite)));
}
