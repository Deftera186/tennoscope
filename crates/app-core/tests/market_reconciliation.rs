use app_core::{OrderStatus, reconcile_orders};
use local_store::SnapshotMeta;
use warframe_domain::{
    CatalogItem, Category, Collection, InventoryEntry, InventorySnapshot, ItemId,
};
use warframe_market::{MarketItems, MarketOrder, OrderKind};

const BRATON_PATH: &str = "/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint";
const BRATON_ID: &str = "54a73e65e779893a797fff33";
const MOD_PATH: &str = "/Lotus/Upgrades/Mods/Pistol/DualStat/CorruptedCritChanceFireRatePistol";
const MOD_ID: &str = "54ca39abe7798915c1c11e10";
/// A market item with no `gameRef`, of which the live table carries 35.
const RETIRED_ID: &str = "5program0000000000000000";
/// A relic and a set, both taken verbatim from `GET /v2/items` on 2026-08-03. Their published
/// paths name rows the collection never carries -- the relic's four refinements are stored
/// suffixed, and a set's parts are `/Recipes/` rows rather than the built item -- which is why
/// neither can be compared against an owned quantity.
const RELIC_ID: &str = "56783f24cbfa8f0432dd89a2";
const SET_ID: &str = "54a73e65e779893a797ffef1";

const ITEMS: &str = r#"{"apiVersion":"0.25.0","data":[
    {"id":"54a73e65e779893a797fff33",
     "gameRef":"/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint",
     "i18n":{"en":{"name":"Braton Prime Blueprint"}}},
    {"id":"54ca39abe7798915c1c11e10",
     "gameRef":"/Lotus/Upgrades/Mods/Pistol/DualStat/CorruptedCritChanceFireRatePistol",
     "maxRank":5,
     "i18n":{"en":{"name":"Creeping Bullseye"}}},
    {"id":"56783f24cbfa8f0432dd89a2",
     "gameRef":"/Lotus/Types/Game/Projections/T4VoidProjectionE",
     "subtypes":["intact","exceptional","flawless","radiant"],
     "i18n":{"en":{"name":"Axi A1 Relic"}}},
    {"id":"54a73e65e779893a797ffef1",
     "gameRef":"/Lotus/Weapons/Tenno/Rifle/BratonPrime",
     "i18n":{"en":{"name":"Braton Prime Set"}}},
    {"id":"5program0000000000000000","i18n":{"en":{"name":"Legendary Fusion Core"}}}
],"error":null}"#;

fn items() -> MarketItems {
    MarketItems::from_response(ITEMS.as_bytes()).expect("items parse")
}

fn collection_holding(entries: Vec<(&str, u32, Option<u32>)>) -> Collection {
    let entries = entries
        .into_iter()
        .map(|(path, quantity, rank)| {
            let id = match rank {
                Some(rank) => ItemId::new(format!("{path}#{rank}")).expect("item id"),
                None => ItemId::new(path).expect("item id"),
            };
            let item = CatalogItem::new(id, "Item", Category::PrimePart).expect("catalog item");
            let entry = InventoryEntry::new(item, quantity);
            match rank {
                Some(rank) => entry.with_rank(rank, Some(5)),
                None => entry,
            }
        })
        .collect::<Vec<_>>();
    let mut collection = Collection::default();
    collection.replace(InventorySnapshot::coherent(entries).expect("snapshot"));
    collection
}

fn snapshot_at(observed_at: &str) -> SnapshotMeta {
    SnapshotMeta::new(
        observed_at.to_owned(),
        "build-for-test".to_owned(),
        "test-fixture-source".to_owned(),
    )
    .expect("snapshot meta")
}

fn sell_order(item_id: &str, quantity: u32, updated_at: &str) -> MarketOrder {
    MarketOrder {
        id: "order-under-test".to_owned(),
        item_id: item_id.to_owned(),
        kind: OrderKind::Sell,
        platinum: 12,
        quantity,
        per_trade: 1,
        rank: None,
        subtype: None,
        visible: true,
        updated_at: Some(updated_at.to_owned()),
    }
}

#[test]
fn an_order_backed_by_the_collection_is_ok() {
    let orders = vec![sell_order(BRATON_ID, 2, "2026-07-30T10:00:00Z")];

    let reconciled = reconcile_orders(
        &orders,
        &items(),
        &collection_holding(vec![(BRATON_PATH, 3, None)]),
        Some(&snapshot_at("2026-07-31T10:00:00Z")),
    );

    assert_eq!(reconciled[0].status, OrderStatus::Ok);
}

/// The case the feature exists for: sold in game, never taken down.
#[test]
fn an_order_for_something_unowned_is_missing() {
    let orders = vec![sell_order(BRATON_ID, 1, "2026-07-30T10:00:00Z")];

    let reconciled = reconcile_orders(
        &orders,
        &items(),
        &collection_holding(vec![(MOD_PATH, 4, None)]),
        Some(&snapshot_at("2026-07-31T10:00:00Z")),
    );

    assert_eq!(reconciled[0].status, OrderStatus::Missing);
}

/// A quantity-zero row is not ownership. The collection keeps mastered items at zero, and an order
/// against one is as wrong as an order against an item that is absent entirely.
#[test]
fn an_order_against_a_zero_quantity_row_is_missing() {
    let orders = vec![sell_order(BRATON_ID, 1, "2026-07-30T10:00:00Z")];

    let reconciled = reconcile_orders(
        &orders,
        &items(),
        &collection_holding(vec![(BRATON_PATH, 0, None)]),
        Some(&snapshot_at("2026-07-31T10:00:00Z")),
    );

    assert_eq!(reconciled[0].status, OrderStatus::Missing);
}

#[test]
fn an_order_listing_more_than_is_owned_overshoots() {
    let orders = vec![sell_order(BRATON_ID, 3, "2026-07-30T10:00:00Z")];

    let reconciled = reconcile_orders(
        &orders,
        &items(),
        &collection_holding(vec![(BRATON_PATH, 1, None)]),
        Some(&snapshot_at("2026-07-31T10:00:00Z")),
    );

    assert_eq!(reconciled[0].status, OrderStatus::Overshoot { owned: 1 });
}

/// No snapshot means no opinion. Without this, a fresh installation that has never read the game
/// would flag every order the player holds and offer to delete each one.
#[test]
fn without_a_snapshot_nothing_is_claimed() {
    let orders = vec![sell_order(BRATON_ID, 1, "2026-07-30T10:00:00Z")];

    let reconciled = reconcile_orders(&orders, &items(), &Collection::default(), None);

    assert_eq!(reconciled[0].status, OrderStatus::Unverifiable);
}

/// The rule that keeps the rest trustworthy. A snapshot older than the order describes a world
/// before the order changed, and cannot contradict it.
///
/// This is also the failure posture the application already holds to elsewhere: a broken game
/// reader keeps its last coherent snapshot, so a stale snapshot looks exactly like a current one
/// from here. Judging against it would produce a screen of confident accusations, each with a
/// delete button beside it.
#[test]
fn a_snapshot_older_than_the_order_claims_nothing() {
    let orders = vec![sell_order(BRATON_ID, 1, "2026-07-31T10:00:00Z")];

    let reconciled = reconcile_orders(
        &orders,
        &items(),
        &collection_holding(vec![(MOD_PATH, 4, None)]),
        Some(&snapshot_at("2026-07-30T10:00:00Z")),
    );

    assert_eq!(reconciled[0].status, OrderStatus::Unverifiable);
}

/// An order the market publishes no game reference for cannot be looked up at all. 35 of the live
/// table's 3,837 entries are like this.
#[test]
fn an_order_with_no_resolvable_item_is_unverifiable() {
    let orders = vec![sell_order(RETIRED_ID, 1, "2026-07-30T10:00:00Z")];

    let reconciled = reconcile_orders(
        &orders,
        &items(),
        &collection_holding(vec![(BRATON_PATH, 1, None)]),
        Some(&snapshot_at("2026-07-31T10:00:00Z")),
    );

    assert_eq!(reconciled[0].status, OrderStatus::Unverifiable);
    // Still nameable, so the row reads as an order rather than as a blank.
    assert_eq!(reconciled[0].name.as_deref(), Some("Legendary Fusion Core"));
}

/// A ranked order names a copy at a rank, and the collection stores each rank as its own entry.
/// Which copies the order means is not answerable from the order alone.
#[test]
fn a_ranked_order_is_unverifiable() {
    let mut order = sell_order(MOD_ID, 1, "2026-07-30T10:00:00Z");
    order.rank = Some(5);

    let reconciled = reconcile_orders(
        &[order],
        &items(),
        &collection_holding(vec![(MOD_PATH, 2, Some(5))]),
        Some(&snapshot_at("2026-07-31T10:00:00Z")),
    );

    assert_eq!(reconciled[0].status, OrderStatus::Unverifiable);
}

/// Owning none of something is the ordinary state for something you are trying to buy.
#[test]
fn a_buy_order_is_never_reconciled() {
    let mut order = sell_order(BRATON_ID, 1, "2026-07-30T10:00:00Z");
    order.kind = OrderKind::Buy;

    let reconciled = reconcile_orders(
        &[order],
        &items(),
        &Collection::default(),
        Some(&snapshot_at("2026-07-31T10:00:00Z")),
    );

    assert_eq!(reconciled[0].status, OrderStatus::Unverifiable);
}

/// An order with no update time cannot be placed relative to the snapshot, so it is not judged.
#[test]
fn an_order_with_no_update_time_is_unverifiable() {
    let mut order = sell_order(BRATON_ID, 1, "2026-07-30T10:00:00Z");
    order.updated_at = None;

    let reconciled = reconcile_orders(
        &[order],
        &items(),
        &collection_holding(vec![(MOD_PATH, 1, None)]),
        Some(&snapshot_at("2026-07-31T10:00:00Z")),
    );

    assert_eq!(reconciled[0].status, OrderStatus::Unverifiable);
}

/// Quantity is summed across the ranked entries of one card. A player holding two unranked and one
/// at rank 5 holds three, and an order for three is not an overshoot.
#[test]
fn owned_quantity_sums_every_rank_of_one_card() {
    let orders = vec![sell_order(MOD_ID, 3, "2026-07-30T10:00:00Z")];

    let reconciled = reconcile_orders(
        &orders,
        &items(),
        &collection_holding(vec![(MOD_PATH, 2, None), (MOD_PATH, 1, Some(5))]),
        Some(&snapshot_at("2026-07-31T10:00:00Z")),
    );

    assert_eq!(reconciled[0].status, OrderStatus::Ok);
}

/// Production snapshot metadata records Unix seconds while orders carry RFC 3339. Compared as
/// text, every digit sorts below every year, so every order would read as unverifiable forever
/// and the feature would ship doing nothing.
#[test]
fn an_epoch_seconds_snapshot_is_compared_against_an_rfc_3339_order() {
    let orders = vec![sell_order(BRATON_ID, 1, "2026-07-30T10:00:00Z")];

    // 2026-07-31T10:00:00Z.
    let reconciled = reconcile_orders(
        &orders,
        &items(),
        &collection_holding(vec![(MOD_PATH, 4, None)]),
        Some(&snapshot_at("1785492000")),
    );

    assert_eq!(reconciled[0].status, OrderStatus::Missing);
}

/// A relic listing is not evidence about the collection. warframe.market publishes one entry per
/// relic carrying the base projection path, with the four refinements as subtypes; the collection
/// stores each refinement as its own suffixed row. The base path is a row it never holds, so
/// comparing against it answers "owned: none" for every relic anyone has ever listed.
#[test]
fn a_relic_order_is_unverifiable_rather_than_missing() {
    let orders = vec![sell_order(RELIC_ID, 1, "2026-07-30T10:00:00Z")];

    let reconciled = reconcile_orders(
        &orders,
        &items(),
        // The refinement the player actually holds, under the name the collection gives it.
        &collection_holding(vec![(
            "/Lotus/Types/Game/Projections/T4VoidProjectionEBronze",
            3,
            None,
        )]),
        Some(&snapshot_at("2026-07-31T10:00:00Z")),
    );

    assert_eq!(
        reconciled[0].status,
        OrderStatus::Unverifiable,
        "a relic listing must never be offered a delete button on the strength of a path the \
         collection cannot carry"
    );
}

/// A set listing names the built item; what the seller holds is the parts. Left uncaught, every
/// set on the account -- the most common thing there is to sell -- reads as missing.
#[test]
fn a_set_order_is_unverifiable_rather_than_missing() {
    let orders = vec![sell_order(SET_ID, 1, "2026-07-30T10:00:00Z")];

    let reconciled = reconcile_orders(
        &orders,
        &items(),
        &collection_holding(vec![(BRATON_PATH, 1, None)]),
        Some(&snapshot_at("2026-07-31T10:00:00Z")),
    );

    assert_eq!(reconciled[0].status, OrderStatus::Unverifiable);
}

/// The badge on a collection card is a join the frontend cannot make on its own: an order names the
/// market's opaque id and a card names a `/Lotus/` row, and the two namespaces share nothing. The
/// reconciliation already holds the item table, so it names the row for the interface -- including
/// for orders it declines to judge, because an unverifiable order is still a live listing whose
/// holding the card should be able to speak about.
#[test]
fn a_reconciled_order_names_the_collection_row_it_belongs_to() {
    let mut ranked = sell_order(MOD_ID, 1, "2026-07-30T10:00:00Z");
    ranked.rank = Some(5);
    let mut refinement = sell_order(RELIC_ID, 1, "2026-07-30T10:00:00Z");
    refinement.subtype = Some("intact".to_owned());
    let orders = vec![
        sell_order(BRATON_ID, 2, "2026-07-30T10:00:00Z"),
        ranked,
        refinement,
    ];

    let reconciled = reconcile_orders(
        &orders,
        &items(),
        &Collection::default(),
        Some(&snapshot_at("2026-07-31T10:00:00Z")),
    );

    assert_eq!(reconciled[0].row_id.as_deref(), Some(BRATON_PATH));
    assert_eq!(
        reconciled[1].row_id.as_deref(),
        Some(&*format!("{MOD_PATH}#5"))
    );
    assert_eq!(
        reconciled[2].row_id.as_deref(),
        Some("/Lotus/Types/Game/Projections/T4VoidProjectionEBronze")
    );
}

/// The row an order cannot name: a set, whose path is the built item rather than the parts held,
/// and a retired item the market publishes no path for. Each carries no row, and the interface
/// offers neither badge nor edit on the strength of a row that does not exist.
#[test]
fn an_order_that_names_no_row_says_so() {
    let orders = vec![
        sell_order(SET_ID, 1, "2026-07-30T10:00:00Z"),
        sell_order(RETIRED_ID, 1, "2026-07-30T10:00:00Z"),
    ];

    let reconciled = reconcile_orders(
        &orders,
        &items(),
        &Collection::default(),
        Some(&snapshot_at("2026-07-31T10:00:00Z")),
    );

    assert_eq!(reconciled[0].row_id, None);
    assert_eq!(reconciled[1].row_id, None);
}
