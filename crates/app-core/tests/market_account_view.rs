use app_core::{AppCore, HealthState, LinkState, MarketAccountView, OrderStatus, ReconciledOrder};
use warframe_domain::{
    CatalogItem, Category, Collection, InventoryEntry, InventorySnapshot, ItemId,
};
use warframe_market::{CredentialBacking, MarketItems, MarketOrder, OrderKind};

/// The market table the listable rule is measured against: a plain item, a ranked mod, and a
/// relic publishing its base path with the four refinements as subtypes. Shapes verbatim from
/// `GET /v2/items`.
const ITEMS: &str = r#"{"apiVersion":"0.25.0","data":[
    {"id":"54a73e65e779893a797fff33",
     "gameRef":"/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint",
     "i18n":{"en":{"name":"Braton Prime Blueprint"}}},
    {"id":"54ca39abe7798915c1c11e10",
     "gameRef":"/Lotus/Upgrades/Mods/Pistol/DualStat/CorruptedCritChanceFireRatePistol",
     "maxRank":5,"i18n":{"en":{"name":"Creeping Bullseye"}}},
    {"id":"56783f24cbfa8f0432dd89a2",
     "gameRef":"/Lotus/Types/Game/Projections/T4VoidProjectionE","bulkTradable":true,
     "subtypes":["intact","exceptional","flawless","radiant"],
     "i18n":{"en":{"name":"Axi A1 Relic"}}}
],"error":null}"#;

const MOD_PATH: &str = "/Lotus/Upgrades/Mods/Pistol/DualStat/CorruptedCritChanceFireRatePistol";
const RELIC_BASE: &str = "/Lotus/Types/Game/Projections/T4VoidProjectionE";

fn items() -> MarketItems {
    MarketItems::from_response(ITEMS.as_bytes()).expect("items parse")
}

fn row(id: &str, quantity: u32, rank: Option<u32>, max_rank: Option<u32>) -> InventoryEntry {
    let item = CatalogItem::new(ItemId::new(id).expect("item id"), "Item", Category::Mod)
        .expect("catalog item");
    let entry = InventoryEntry::new(item, quantity);
    match rank {
        Some(rank) => entry.with_rank(rank, max_rank),
        None => entry,
    }
}

fn collection_of(entries: Vec<InventoryEntry>) -> Collection {
    let mut collection = Collection::default();
    collection.replace(InventorySnapshot::coherent(entries).expect("snapshot"));
    collection
}

fn order(id: &str, platinum: u32, quantity: u32, visible: bool) -> MarketOrder {
    MarketOrder {
        id: id.to_owned(),
        item_id: "54a73e65e779893a797fff33".to_owned(),
        kind: OrderKind::Sell,
        platinum,
        quantity,
        per_trade: 1,
        rank: None,
        subtype: None,
        visible,
        updated_at: Some("2026-07-30T10:00:00Z".to_owned()),
    }
}

fn reconciled(order: MarketOrder, status: OrderStatus) -> ReconciledOrder {
    ReconciledOrder {
        order,
        name: Some("Braton Prime Blueprint".to_owned()),
        status,
    }
}

/// A fresh application has no account, and says so without pretending anything failed.
#[test]
fn a_new_core_reports_no_linked_account() {
    let core = AppCore::in_memory().expect("core opens");

    assert_eq!(core.market_account().link, LinkState::Unlinked);
    assert!(core.market_account().orders.is_empty());
    assert_eq!(
        core.health().market_account().state(),
        HealthState::Idle,
        "an unlinked account is not a fault: nothing is wrong and nothing was asked for"
    );
}

#[test]
fn a_linked_account_reports_its_orders_and_backing() {
    let mut core = AppCore::in_memory().expect("core opens");

    let view = core
        .set_market_account(MarketAccountView::linked(
            CredentialBacking::Keyring,
            vec![reconciled(order("one", 12, 2, true), OrderStatus::Ok)],
            "2026-07-31T12:00:00Z".to_owned(),
        ))
        .expect("account sets");

    assert_eq!(view.market_account().link, LinkState::Linked);
    assert_eq!(
        view.market_account().backing,
        Some(CredentialBacking::Keyring)
    );
    assert_eq!(view.market_account().orders.len(), 1);
    assert_eq!(
        view.market_account().fetched_at.as_deref(),
        Some("2026-07-31T12:00:00Z")
    );
}

/// The header figure. Only visible sell orders count: a hidden listing is not offered to anybody,
/// and a buy order is money going out rather than value held.
#[test]
fn listed_value_counts_only_what_is_actually_offered() {
    let mut hidden = order("hidden", 100, 1, false);
    hidden.visible = false;
    let mut buying = order("buying", 50, 1, true);
    buying.kind = OrderKind::Buy;

    let view = MarketAccountView::linked(
        CredentialBacking::Database,
        vec![
            reconciled(order("one", 12, 2, true), OrderStatus::Ok),
            reconciled(hidden, OrderStatus::Ok),
            reconciled(buying, OrderStatus::Unverifiable),
        ],
        "2026-07-31T12:00:00Z".to_owned(),
    );

    assert_eq!(view.listed_platinum, 24);
}

/// What the section badge counts. An unverifiable order is not a problem and must not be counted
/// as one -- a badge reading "9 problems" on a machine that simply has not read the game yet is
/// the exact false alarm the unverifiable state exists to prevent.
#[test]
fn only_claims_are_counted_as_flagged() {
    let view = MarketAccountView::linked(
        CredentialBacking::Keyring,
        vec![
            reconciled(order("one", 12, 1, true), OrderStatus::Ok),
            reconciled(order("two", 12, 1, true), OrderStatus::Missing),
            reconciled(
                order("three", 12, 3, true),
                OrderStatus::Overshoot { owned: 1 },
            ),
            reconciled(order("four", 12, 1, true), OrderStatus::Unverifiable),
        ],
        "2026-07-31T12:00:00Z".to_owned(),
    );

    assert_eq!(view.flagged, 2);
}

/// A refused credential stops the feature and says so, rather than presenting as an empty account.
#[test]
fn a_relink_is_reported_as_its_own_state() {
    let mut core = AppCore::in_memory().expect("core opens");

    let view = core
        .set_market_account(MarketAccountView::needs_relink())
        .expect("account sets");

    assert_eq!(view.market_account().link, LinkState::NeedsRelink);
    assert_eq!(
        view.health().market_account().state(),
        HealthState::Degraded
    );
}

/// A failed fetch keeps the orders already held. The list is still the truth as of when it was
/// fetched, and its age is on the screen -- discarding it would replace a slightly old answer with
/// no answer.
#[test]
fn a_failed_fetch_keeps_the_orders_already_held() {
    let mut core = AppCore::in_memory().expect("core opens");
    core.set_market_account(MarketAccountView::linked(
        CredentialBacking::Keyring,
        vec![reconciled(order("one", 12, 2, true), OrderStatus::Ok)],
        "2026-07-31T12:00:00Z".to_owned(),
    ))
    .expect("account sets");

    let view = core
        .record_market_account_failure("warframe.market could not be reached")
        .expect("failure records");

    assert_eq!(view.market_account().orders.len(), 1);
    assert_eq!(view.market_account().link, LinkState::Linked);
    assert_eq!(
        view.health().market_account().state(),
        HealthState::Degraded
    );
}

/// Unlinking empties the view. Leaving orders on screen after unlinking would show account data
/// the player just disconnected.
#[test]
fn unlinking_clears_the_orders() {
    let mut core = AppCore::in_memory().expect("core opens");
    core.set_market_account(MarketAccountView::linked(
        CredentialBacking::Keyring,
        vec![reconciled(order("one", 12, 2, true), OrderStatus::Ok)],
        "2026-07-31T12:00:00Z".to_owned(),
    ))
    .expect("account sets");

    let view = core
        .set_market_account(MarketAccountView::unlinked())
        .expect("account clears");

    assert!(view.market_account().orders.is_empty());
    assert_eq!(view.market_account().link, LinkState::Unlinked);
    assert_eq!(view.market_account().backing, None);
    // The health row clears with the view. A success time left on a row that reads "no account
    // linked" describes a fetch for an account the player disconnected.
    assert_eq!(view.health().market_account().last_success(), None);
}

/// A bulk listing prices one trade, not one unit. Measured against the live API, where a traded
/// relic carries `perTrade: 6` on roughly a third of its orders -- 300 listed at 18p per six is
/// asking 900p, and multiplying the two figures together would put 5,400p at the top of the
/// screen.
#[test]
fn listed_value_prices_a_trade_rather_than_a_unit() {
    let mut bulk = order("bulk", 18, 300, true);
    bulk.per_trade = 6;

    let view = MarketAccountView::linked(
        CredentialBacking::Keyring,
        vec![reconciled(bulk, OrderStatus::Ok)],
        "2026-07-31T12:00:00Z".to_owned(),
    );

    assert_eq!(view.listed_platinum, 900);
}

/// The listable set names rows, not paths, because the rows are what differ. A card held unranked
/// and held maxed is two holdings with two listings -- rank 0 and rank 5 -- while the part-ranked
/// copy between them has no rank the market would accept and so no offer. A relic refinement row
/// resolves through its tier suffix. The order is the collection's own: sorted by row id.
#[test]
fn the_listable_set_names_the_rows_the_market_would_accept() {
    let collection = collection_of(vec![
        row(
            "/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint",
            2,
            None,
            None,
        ),
        row(MOD_PATH, 4, None, None),
        row(&format!("{MOD_PATH}#3"), 1, Some(3), Some(5)),
        row(&format!("{MOD_PATH}#5"), 2, Some(5), Some(5)),
        row(&format!("{RELIC_BASE}Bronze"), 6, None, None),
    ]);

    let view = MarketAccountView::linked(
        CredentialBacking::Keyring,
        Vec::new(),
        "2026-08-22T12:00:00Z".to_owned(),
    )
    .with_listable(&items(), &collection);

    assert_eq!(
        view.listable,
        vec![
            format!("{RELIC_BASE}Bronze"),
            "/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint".to_owned(),
            MOD_PATH.to_owned(),
            format!("{MOD_PATH}#5"),
        ],
        "the part-ranked row is the one refusal in here"
    );
}
