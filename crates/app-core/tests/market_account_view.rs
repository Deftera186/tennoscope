use app_core::{AppCore, HealthState, LinkState, MarketAccountView, OrderStatus, ReconciledOrder};
use warframe_market::{CredentialBacking, MarketOrder, OrderKind};

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
            reconciled(order("three", 12, 3, true), OrderStatus::Overshoot { owned: 1 }),
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
