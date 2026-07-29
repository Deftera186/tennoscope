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

/// Download, fold in what has already been checked, store -- the three steps the startup path
/// takes. It composes them here because these tests have no runtime lock to straddle; production
/// keeps them apart on purpose, and
/// `the_startup_refresh_must_adopt_from_the_current_table_not_a_pre_download_snapshot` says why.
fn refresh(
    cache: &CollectionPriceCache,
    source: &dyn CollectionPriceSource,
    now_unix: u64,
    previous: Option<&PriceTable>,
) -> Result<PriceTable, PriceDumpError> {
    let mut table = latest_dump(source, now_unix)?;
    if let Some(previous) = previous {
        table.adopt_checked(previous);
    }
    cache.store_table(&table)?;
    Ok(table)
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
fn a_checked_relic_price_is_served_like_any_other() {
    let mut table = table();
    table.insert_checked("Axi A1 Relic", 17);
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
    table.insert_checked("Axi A1 Relic", 17);
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

/// The sweep shares the dump's clock rather than keeping one of its own: a refresh that brings back
/// the same dump keeps the prices swept against it, and a genuinely newer dump clears them so the
/// sweep runs again for the day it describes.
///
/// This replaces `a_relic_sweep_is_current_exactly_when_its_dump_is`, which pinned a second date
/// gate on the sweep. That gate was false on any ordinary day -- the dumps lag, so the cached table
/// is usually older than yesterday -- and its falseness re-swept all 65 relics on every launch.
/// Adoption across a same-date refresh is what the gate was trying to express, and it works on the
/// days the gate did not.
#[test]
fn checked_prices_survive_a_refresh_of_the_same_dump() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cache = CollectionPriceCache::new(directory.path());
    let mut checked = refresh(
        &cache,
        &FakeDumps::new(&[("2026-07-27", DUMP)]),
        TODAY,
        None,
    )
    .expect("first refresh stores");
    checked.insert_checked("Axi A1 Relic", 17);
    // A non-relic too: the page refresh checks whatever is on screen, and `Serration` has a dump
    // price of 50 for the carried-over 42 to keep beating.
    checked.insert_checked("Serration", 42);

    // The same lagging dump comes back, as it does on any ordinary day.
    let refreshed = refresh(
        &cache,
        &FakeDumps::new(&[("2026-07-27", DUMP)]),
        TODAY,
        Some(&checked),
    )
    .expect("second refresh stores");

    assert_eq!(refreshed.price_for("Axi A1 Radiant"), Some(17));
    assert_eq!(
        refreshed.price_for("Serration"),
        Some(42),
        "a checked price outlives the refresh that re-parsed its dump"
    );
    let reloaded = cache.load_cached().expect("a stored table is readable");
    assert_eq!(
        reloaded.price_for("Axi A1 Relic"),
        Some(17),
        "the adopted price reaches disk, or the next launch re-sweeps"
    );
    assert_eq!(reloaded.price_for("Serration"), Some(42));
}

#[test]
fn a_newer_dump_discards_the_prices_checked_against_the_old_one() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cache = CollectionPriceCache::new(directory.path());
    let mut checked = refresh(
        &cache,
        &FakeDumps::new(&[("2026-07-27", DUMP)]),
        TODAY,
        None,
    )
    .expect("first refresh stores");
    checked.insert_checked("Axi A1 Relic", 17);
    checked.insert_checked("Serration", 42);

    let refreshed = refresh(
        &cache,
        &FakeDumps::new(&[("2026-07-29", DUMP)]),
        TODAY,
        Some(&checked),
    )
    .expect("second refresh stores");

    assert_eq!(refreshed.dump_date(), "2026-07-29");
    assert_eq!(
        refreshed.price_for("Axi A1 Radiant"),
        None,
        "a price checked for another day's table is re-swept, not carried over"
    );
    assert_eq!(
        refreshed.price_for("Serration"),
        Some(50),
        "an item the new dump prices falls back to the new dump, not to the stale checked number"
    );
}

/// The health row reports what the table can price. Counting only dump prices left it stuck at the
/// dump's count while 65 relics gained prices underneath it.
#[test]
fn the_reported_count_grows_as_relics_are_swept() {
    let mut table = table();
    let before = table.len();
    table.insert_checked("Axi A1 Relic", 17);

    assert_eq!(table.len(), before + 1);

    // Improving a price the dump already had is not another item priced.
    table.insert_checked("Serration", 42);
    assert_eq!(table.len(), before + 1);
}

/// The point of persisting a checked price: it is the better measurement, so it must win. For a
/// relic there is no dump price to lose to, which is why the order went unnoticed while only the
/// relic sweep wrote here -- for everything else the dump would shadow the number the player just
/// spent a request on.
#[test]
fn a_checked_price_outranks_the_dumps_for_the_same_item() {
    let mut table = table();
    assert_eq!(table.price_for("Serration"), Some(50), "the dump's median");

    table.insert_checked("Serration", 42);

    assert_eq!(table.price_for("Serration"), Some(42));
}

/// The same precedence through the name rules, since that is how the collection asks. Both names
/// here need a rule beyond the literal one: `Forma` is reached by stripping ` Blueprint`, and
/// `Axi A1 Radiant` by replacing the refinement suffix. A dump key spelled exactly as the catalog
/// spells it would prove nothing about either rule.
#[test]
fn a_checked_price_reaches_an_item_through_the_name_rules() {
    let dump = r#"{
        "Forma": [{"order_type":"sell","median":8.0,"volume":3}],
        "Axi A1 Relic": [{"order_type":"sell","median":25.0,"volume":30}]
    }"#;
    let mut table = PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").expect("parses");
    assert_eq!(
        table.price_for("Forma Blueprint"),
        Some(8),
        "the dump's median"
    );

    table.insert_checked("Forma", 6);
    table.insert_checked("Axi A1 Relic", 17);

    assert_eq!(table.price_for("Forma Blueprint"), Some(6));
    assert_eq!(table.price_for("Axi A1 Radiant"), Some(17));
}

/// The startup dump refresh spends seconds downloading 3.9 MB, and the relic sweep or a page
/// refresh can land in that window. Folding into the new table must therefore read the table the
/// runtime is serving *now*, not the snapshot startup took before the download -- adopting from
/// the snapshot silently erases a price the player has already paid a request for, from memory and
/// from disk. `start_collection_prices` keeps the fold under the same lock hold as the store and
/// the publish for this reason; this is what that ordering is protecting against.
#[test]
fn the_startup_refresh_must_adopt_from_the_current_table_not_a_pre_download_snapshot() {
    let snapshot = table(); // read from disk before the download began
    let mut current = table();
    current.insert_checked("Serration", 42); // a page refresh landed during the download

    let mut from_snapshot = table();
    from_snapshot.adopt_checked(&snapshot);
    assert_eq!(
        from_snapshot.price_for("Serration"),
        Some(50),
        "adopting from the pre-download snapshot loses the checked price"
    );

    let mut from_current = table();
    from_current.adopt_checked(&current);
    assert_eq!(from_current.price_for("Serration"), Some(42));
}

use warframe_acquisition::CollectionPriceCache;

#[test]
fn a_refreshed_table_is_readable_without_the_network() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cache = CollectionPriceCache::new(directory.path());
    let source = FakeDumps::new(&[("2026-07-27", DUMP)]);

    refresh(&cache, &source, TODAY, None).expect("refresh stores");
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
    refresh(
        &cache,
        &FakeDumps::new(&[("2026-07-27", DUMP)]),
        TODAY,
        None,
    )
    .expect("first refresh stores");

    assert!(refresh(&cache, &FakeDumps::new(&[]), TODAY, None).is_err());

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

/// A cache written before this map was generalised names it `relic_prices`. Reading it under the
/// old name is one serde attribute against re-spending a whole sweep's requests on the first
/// launch after an upgrade.
#[test]
fn a_cache_written_under_the_old_field_name_still_carries_its_prices() {
    let directory = tempfile::tempdir().expect("temp dir");
    let stored = r#"{"prices":{"Serration":50},"relic_names":["Axi A1 Relic"],"relic_prices":{"Axi A1 Relic":17},"dump_date":"2026-07-27"}"#;
    std::fs::write(directory.path().join("collection-prices.json"), stored).expect("write cache");

    let table = CollectionPriceCache::new(directory.path())
        .load_cached()
        .expect("a stored table is readable");

    assert_eq!(table.price_for("Axi A1 Radiant"), Some(17));
}

/// A checked price survives the cache round-trip, or every restart would cost another sweep.
#[test]
fn checked_prices_survive_the_disk_cache() {
    let directory = tempfile::tempdir().expect("temp dir");
    let cache = CollectionPriceCache::new(directory.path());
    let mut table = refresh(
        &cache,
        &FakeDumps::new(&[("2026-07-27", DUMP)]),
        TODAY,
        None,
    )
    .expect("refresh stores");
    table.insert_checked("Axi A1 Relic", 17);
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

    let result = refresh(&cache, &source, TODAY, None);
    assert!(matches!(result, Err(PriceDumpError::CacheWrite)));
}
