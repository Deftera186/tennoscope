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
    live.insert("Axi A1 Relic", 31);
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

/// A swept relic price is a live price that was persisted, and it must keep saying so once the
/// fifteen-minute live cache has dropped it. The alternative is a relic silently presenting as a
/// dump price under a line reading "prices from the 27 Jul market summary" -- which is false about
/// every relic on screen, because the dump deliberately prices no relics at all.
#[test]
fn a_swept_relic_price_stays_marked_live_after_the_cache_expires() {
    let dump = r#"{"Axi A1 Relic": [{"order_type":"sell","median":25.0,"volume":30}]}"#;
    let mut table = PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").expect("parses");
    table.insert_live("Axi A1 Relic", 17);
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

/// Four refinements of one relic are one item on warframe.market. Asking about a page holding all
/// four must cost one request, not four. The store returns entries ordered by item id (see
/// `SqliteStore::load_collection`'s `ORDER BY item_id`), so "/b" sits between the two "/a"/"/c"
/// refinements of the same relic pre-sort -- the duplicate market names are not adjacent until
/// `market_names_for` sorts them, which is what makes the following `dedup()` sufficient.
#[test]
fn relic_refinements_on_one_page_collapse_to_a_single_request() {
    let dump = r#"{
        "Axi A1 Relic": [{"order_type":"sell","median":20.0,"volume":30}],
        "Meso B2 Relic": [{"order_type":"sell","median":15.0,"volume":25}]
    }"#;
    let mut core = core_with_items(vec![
        item("/a", "Axi A1 Intact", Category::Relic, 2),
        item("/b", "Meso B2 Radiant", Category::Relic, 1),
        item("/c", "Axi A1 Flawless", Category::Relic, 4),
    ]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    let names = core
        .market_names_for(&["/a".to_owned(), "/b".to_owned(), "/c".to_owned()])
        .expect("resolves");

    assert_eq!(
        names,
        vec!["Axi A1 Relic".to_owned(), "Meso B2 Relic".to_owned()]
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
        vec!["Axi A1 Relic".to_owned()],
        "a relic at quantity 0 is not owned, and a resource is not a relic"
    );
}

/// Four refinement tiers of one relic are one request.
#[test]
fn relic_refinements_collapse_before_the_sweep() {
    let dump = r#"{"Axi A1 Relic": [{"order_type":"sell","median":20.0,"volume":30}]}"#;
    let mut core = core_with_items(vec![
        item("/a", "Axi A1 Intact", Category::Relic, 1),
        item("/b", "Axi A1 Radiant", Category::Relic, 3),
    ]);
    core.set_collection_prices(Arc::new(
        PriceTable::from_dump_json(dump.as_bytes(), "2026-07-27").expect("fixture parses"),
    ));

    assert_eq!(core.owned_relic_market_names().unwrap().len(), 1);
}
