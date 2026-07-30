use std::sync::Arc;

use app_core::AppCore;
use local_store::SnapshotMeta;
use warframe_acquisition::{MarketPriceCache, PriceTable};
use warframe_domain::{CatalogItem, Category, InventoryEntry, InventorySnapshot, ItemId};

const DUMP: &str = r#"{
    "Serration": [{"order_type":"sell","median":50.0,"volume":12}],
    "Mirage Prime Systems Blueprint": [{"order_type":"sell","median":20.0,"volume":9}]
}"#;

fn item(id: &str, name: &str, category: Category, quantity: u32) -> InventoryEntry {
    InventoryEntry::new(
        CatalogItem::new(ItemId::new(id).expect("valid id"), name, category).expect("valid item"),
        quantity,
    )
}

fn core_with_items(entries: Vec<InventoryEntry>) -> AppCore {
    let mut core = AppCore::in_memory().expect("in-memory core");
    core.apply_inventory_snapshot(
        InventorySnapshot::coherent(entries).expect("coherent snapshot"),
        SnapshotMeta::fake("build").expect("meta"),
    )
    .expect("snapshot applies");
    core
}

#[test]
fn a_priced_item_carries_its_platinum_into_the_view() {
    let mut core = core_with_items(vec![
        item("/a", "Serration", Category::Resource, 1),
        item(
            "/b",
            "Mirage Prime Systems Blueprint",
            Category::PrimePart,
            3,
        ),
    ]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(DUMP.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    let view = core.current_view().expect("view builds");
    let prices: Vec<_> = view
        .collection()
        .items()
        .iter()
        .map(|item| (item.name().to_owned(), item.platinum()))
        .collect();

    assert_eq!(
        prices,
        vec![
            ("Serration".to_owned(), Some(50)),
            ("Mirage Prime Systems Blueprint".to_owned(), Some(20)),
        ]
    );
}

/// Before the dump loads, and for anything the dump does not list, the view says nothing rather
/// than zero. Zero would read as "worthless" for an item that is merely unpriced.
#[test]
fn an_item_the_dump_does_not_list_has_no_price() {
    let mut core = core_with_items(vec![item("/a", "Bottomless Pit", Category::Resource, 1)]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(DUMP.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    assert_eq!(
        core.current_view().unwrap().collection().items()[0].platinum(),
        None
    );
}

#[test]
fn a_view_built_before_any_prices_load_is_unpriced_rather_than_broken() {
    let core = core_with_items(vec![item("/a", "Serration", Category::Resource, 1)]);

    assert_eq!(
        core.current_view().unwrap().collection().items()[0].platinum(),
        None
    );
}

/// A live price is the cheapest online seller right now; the dump's is the middle of yesterday's
/// listings. Where both exist the live one wins, and the view says which it gave.
#[test]
fn a_live_price_takes_precedence_over_the_dump_and_says_so() {
    let mut core = core_with_items(vec![item("/a", "Serration", Category::Resource, 1)]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(DUMP.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));
    let live = MarketPriceCache::new();
    live.insert("Serration", 44);
    core.set_live_prices(live);

    let view = core.current_view().expect("view builds");
    assert_eq!(view.collection().items()[0].platinum(), Some(44));
    assert!(view.collection().items()[0].live());
}

/// The live cache keys on warframe.market's name, which is not always the catalog's. Resolving
/// through the dump is what lets a relic priced live be found again.
#[test]
fn a_live_price_is_found_through_the_market_name() {
    let dump = r#"{"Axi A1 Relic": [{"order_type":"sell","median":20.0,"volume":30}]}"#;
    let mut core = core_with_items(vec![item("/a", "Axi A1 Radiant", Category::Relic, 2)]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));
    let live = MarketPriceCache::new();
    live.insert("Axi A1 Relic (Radiant)", 31);
    core.set_live_prices(live);

    let view = core.current_view().expect("view builds");
    assert_eq!(view.collection().items()[0].platinum(), Some(31));
    assert!(view.collection().items()[0].live());
}

/// An item the live pass never reached keeps the dump's price and does not claim to be live.
#[test]
fn an_item_with_no_live_price_falls_back_to_the_dump_unmarked() {
    let mut core = core_with_items(vec![item("/a", "Serration", Category::Resource, 1)]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(DUMP.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));
    core.set_live_prices(MarketPriceCache::new());

    let view = core.current_view().expect("view builds");
    assert_eq!(view.collection().items()[0].platinum(), Some(50));
    assert!(!view.collection().items()[0].live());
}

/// A persisted checked price is a live price that outlived its cache entry, and it must keep
/// saying so. The alternative is a relic silently presenting as a dump price under a line reading
/// "prices from the 27 Jul market summary" -- which is false about every relic on screen, because
/// the dump deliberately prices no relics at all.
#[test]
fn a_swept_relic_price_stays_marked_live_after_the_cache_expires() {
    let dump = r#"{"Axi A1 Relic": [{"order_type":"sell","median":25.0,"volume":30}]}"#;
    let mut table = PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").expect("parses");
    table.insert_checked("Axi A1 Relic", 17);
    let mut core = core_with_items(vec![item("/a", "Axi A1 Radiant", Category::Relic, 2)]);
    core.set_collection_prices(Arc::new(table));
    // An empty live cache is what a swept price outlives: the entry has aged out.
    core.set_live_prices(MarketPriceCache::new());

    let view = core.current_view().expect("view builds");
    assert_eq!(view.collection().items()[0].platinum(), Some(17));
    assert!(
        view.collection().items()[0].live(),
        "a swept price was checked live, whatever the live cache still remembers"
    );
}

/// The same for an item the dump *does* price. A page refresh checks whatever is on screen, and
/// once its answer is persisted the card must show that number and say it was checked live --
/// otherwise the better measurement is presented as the day-old one it replaced.
#[test]
fn a_persisted_checked_price_reads_as_live_for_a_non_relic() {
    let mut table = PriceTable::from_dump_json(DUMP.as_bytes(), "2026-07-27").expect("parses");
    table.insert_checked("Serration", 42);
    let mut core = core_with_items(vec![item("/a", "Serration", Category::Resource, 1)]);
    core.set_collection_prices(Arc::new(table));
    // The fifteen-minute cache entry has aged out; only the persisted price remains.
    core.set_live_prices(MarketPriceCache::new());

    let view = core.current_view().expect("view builds");
    assert_eq!(
        view.collection().items()[0].platinum(),
        Some(42),
        "the checked price, not the dump's 50"
    );
    assert!(
        view.collection().items()[0].live(),
        "a persisted checked price was checked live, whatever the live cache still remembers"
    );
}

#[test]
fn only_the_named_items_are_resolved_for_a_live_lookup() {
    let mut core = core_with_items(vec![
        item("/a", "Serration", Category::Resource, 1),
        item(
            "/b",
            "Mirage Prime Systems Blueprint",
            Category::PrimePart,
            3,
        ),
    ]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(DUMP.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    let names = core.market_names_for(&["/b".to_owned()]).expect("resolves");

    assert_eq!(names, vec!["Mirage Prime Systems Blueprint".to_owned()]);
}

/// Two refinements of one relic are two prices on warframe.market -- separate subtypes of one
/// listing, and a radiant sells for a median 1.46x its intact tier -- so a page holding both costs
/// two requests, not one. Repeats of the *same* tier still collapse. The store returns entries
/// ordered by item id (see `SqliteStore::load_collection`'s `ORDER BY item_id`), so "/b" sits
/// between the two "/a"/"/c" entries of the same relic pre-sort -- the duplicate market names are
/// not adjacent until `market_names_for` sorts them, which is what makes the `dedup()` sufficient.
#[test]
fn one_relic_tier_asked_for_twice_collapses_to_a_single_request() {
    let dump = r#"{
        "Axi A1 Relic": [{"order_type":"sell","median":20.0,"volume":30}],
        "Meso B2 Relic": [{"order_type":"sell","median":15.0,"volume":25}]
    }"#;
    let mut core = core_with_items(vec![
        item("/a", "Axi A1 Intact", Category::Relic, 2),
        item("/b", "Meso B2 Radiant", Category::Relic, 1),
        item("/c", "Axi A1 Intact", Category::Relic, 4),
    ]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    let names = core
        .market_names_for(&["/a".to_owned(), "/b".to_owned(), "/c".to_owned()])
        .expect("resolves");

    assert_eq!(
        names,
        vec![
            "Axi A1 Relic".to_owned(),
            "Meso B2 Relic (Radiant)".to_owned()
        ]
    );
}

/// An item the dump never listed cannot be asked about either: there is no slug to build.
#[test]
fn an_unresolvable_item_is_not_requested() {
    let mut core = core_with_items(vec![item("/a", "Bottomless Pit", Category::Resource, 1)]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(DUMP.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    assert!(
        core.market_names_for(&["/a".to_owned()])
            .unwrap()
            .is_empty()
    );
}

/// Mastery is not ownership. An item at quantity 0 is not in the inventory and must not carry a
/// price, appear under the tradeable filter, or contribute to the collection's worth.
#[test]
fn an_item_the_player_does_not_own_is_not_priced() {
    let mut core = core_with_items(vec![
        item("/a", "Serration", Category::Resource, 0).with_mastered(true),
        item(
            "/b",
            "Mirage Prime Systems Blueprint",
            Category::PrimePart,
            2,
        ),
    ]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(DUMP.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    let view = core.current_view().expect("view builds");
    let items = view.collection().items();
    assert_eq!(items[0].platinum(), None, "quantity 0 is not owned");
    assert_eq!(items[1].platinum(), Some(20));
}

/// The sweep is bounded by what the player owns, not by what exists. Measured against a real
/// collection that is 65 relics and about 22 seconds; the dump lists 772.
#[test]
fn only_owned_relics_are_swept() {
    let dump = r#"{
        "Axi A1 Relic": [{"order_type":"sell","median":20.0,"volume":30}],
        "Meso B2 Relic": [{"order_type":"sell","median":9.0,"volume":12}],
        "Serration": [{"order_type":"sell","median":50.0,"volume":12}]
    }"#;
    let mut core = core_with_items(vec![
        item("/a", "Axi A1 Radiant", Category::Relic, 2),
        item("/b", "Meso B2 Intact", Category::Relic, 0),
        item("/c", "Serration", Category::Resource, 1),
    ]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    assert_eq!(
        core.owned_relic_market_names().expect("resolves"),
        vec![
            "Axi A1 Relic".to_owned(),
            "Axi A1 Relic (Radiant)".to_owned()
        ],
        "a relic at quantity 0 is not owned, and a resource is not a relic"
    );
}

/// A radiant relic drags its intact listing into the sweep, because that listing is what its price
/// falls back to when nobody is selling the refined tier -- which measured over 80 relics is 61% of
/// radiants, and every single `exceptional` and `flawless`.
#[test]
fn a_refined_relic_sweeps_the_intact_listing_it_falls_back_to() {
    let dump = r#"{"Axi A1 Relic": [{"order_type":"sell","median":20.0,"volume":30}]}"#;
    let mut core = core_with_items(vec![item("/a", "Axi A1 Radiant", Category::Relic, 3)]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    assert_eq!(
        core.owned_relic_market_names().unwrap(),
        vec![
            "Axi A1 Relic".to_owned(),
            "Axi A1 Relic (Radiant)".to_owned()
        ]
    );
}

/// Owning the intact copy as well costs nothing extra: it is the same name the refined tier
/// already pulled in, and the dedup folds them.
#[test]
fn owning_a_tier_and_its_intact_fallback_is_one_request_each() {
    let dump = r#"{"Axi A1 Relic": [{"order_type":"sell","median":20.0,"volume":30}]}"#;
    let mut core = core_with_items(vec![
        item("/a", "Axi A1 Intact", Category::Relic, 1),
        item("/b", "Axi A1 Radiant", Category::Relic, 3),
    ]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    assert_eq!(core.owned_relic_market_names().unwrap().len(), 2);
}

/// The page control promised prices for items it was never going to send. It counted everything
/// owned on screen, while the backend drops every name the price table cannot resolve -- so a
/// register full of untradeable resources read "Price these 48" and priced a handful. `priceable`
/// is the same question `market_names_for` already answers, asked per item so the control can
/// count what it is actually about to do.
///
/// It is deliberately not "has a price": a relic the sweep has not reached is priceable, unpriced,
/// and precisely the item somebody clicks that control for.
#[test]
fn only_items_the_backend_can_ask_about_are_offered_for_pricing() {
    let mut core = core_with_items(vec![
        item("/a", "Serration", Category::Resource, 1),
        item("/b", "Bottomless Pit", Category::Resource, 1),
        item("/c", "Axi A1 Radiant", Category::Relic, 3),
        item("/d", "Serration", Category::Resource, 0).with_mastered(true),
    ]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(
            br#"{
                "Serration": [{"order_type":"sell","median":50.0,"volume":12}],
                "Axi A1 Relic": [{"order_type":"sell","median":20.0,"volume":30}]
            }"#,
            "2026-07-27",
        )
        .expect("fixture parses"),
    ));

    let view = core.current_view().expect("view builds");
    let items = view.collection().items();
    assert!(items[0].priceable(), "owned and listed");
    assert!(
        !items[1].priceable(),
        "owned, but no rule reaches a listing"
    );
    assert!(
        items[2].priceable() && items[2].platinum().is_none(),
        "an unswept relic is priceable and unpriced -- the case the control exists for"
    );
    assert!(!items[3].priceable(), "quantity 0 is not owned");

    // The count the control shows and the work the backend does are the same set.
    let offered: Vec<String> = items
        .iter()
        .filter(|item| item.priceable())
        .map(|item| item.id().to_owned())
        .collect();
    assert_eq!(
        offered.len(),
        core.market_names_for(&offered).unwrap().len()
    );
}

/// The sweep runs for about twenty-two seconds and the collection's worth climbs the whole time.
/// Nothing on the page said so, so the figure moved with no account of itself. Both passes that
/// spend requests publish into this one cell, because they share one rate-limited budget and two
/// counters would be describing one queue twice.
#[test]
fn a_live_pricing_pass_reports_its_own_progress() {
    let mut core = core_with_items(vec![item("/a", "Serration", Category::Resource, 1)]);

    assert_eq!(
        core.current_view()
            .expect("view builds")
            .collection()
            .pricing(),
        None,
        "no pass running is not a pass at zero"
    );

    core.set_pricing_progress(Some(app_core::PricingProgress {
        done: 12,
        total: 65,
    }));
    let running = core.current_view().expect("view builds");
    assert_eq!(
        running.collection().pricing(),
        Some(app_core::PricingProgress {
            done: 12,
            total: 65
        })
    );

    core.set_pricing_progress(None);
    assert_eq!(
        core.current_view()
            .expect("view builds")
            .collection()
            .pricing(),
        None,
        "the readout has to clear, or the control stays disabled forever"
    );
}
