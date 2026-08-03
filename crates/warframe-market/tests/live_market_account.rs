//! One opt-in test against the real API.
//!
//! Read-only, and deliberately so. There is no way to exercise a real deletion against a real
//! account without destroying a listing its owner wanted, so the two writes are covered against a
//! fake transport instead and are not attempted here.
//!
//! Reads its token from `TENNOSCOPE_LIVE_MARKET_TOKEN`, so nothing about an account reaches this
//! repository. Output is counts and states only: no order, item, price, or account identifier.

use std::env;

use warframe_acquisition::RequestPacer;
use warframe_market::{MarketHttp, MarketItems, MarketToken, OrderKind, list_mine, verify_token};

#[test]
#[ignore = "requires a warframe.market token in TENNOSCOPE_LIVE_MARKET_TOKEN and network access"]
fn live_market_account_reads_without_changing_anything() {
    let Ok(raw) = env::var("TENNOSCOPE_LIVE_MARKET_TOKEN") else {
        panic!("set TENNOSCOPE_LIVE_MARKET_TOKEN to run this test");
    };
    let token = MarketToken::new(raw);
    let transport = MarketHttp::new(RequestPacer::new()).expect("transport builds");

    let token = verify_token(&transport, &token).expect("the token should be accepted");
    println!("credential=accepted");

    let items = MarketItems::fetch(&transport).expect("the item table should load");
    println!("item_table_entries={}", items.len());
    assert!(
        items.len() > 3_000,
        "the item table should carry thousands of entries, got {}",
        items.len()
    );

    let (orders, _) = list_mine(&transport, &token).expect("the order list should load");
    println!("orders={}", orders.len());
    let resolvable = orders
        .iter()
        .filter(|order| items.comparable(&order.item_id))
        .count();
    // The measurement that matters: how much of a real account's order list can be reconciled at
    // all. Counted by `comparable` rather than by whether a path exists, because a relic and a set
    // both publish a path that names a row the collection never carries -- counting those would
    // report as judgeable exactly the orders reconciliation declines to judge. Anything left over
    // is reported as unverifiable rather than flagged, so a low number is a quiet feature rather
    // than a wrong one -- but it is worth knowing.
    println!("orders_resolvable_to_a_collection_item={resolvable}");

    // What the header figure is built from, reported as its own parts. A total reading zero on an
    // account that plainly has listings could be any of these three predicates, and a single
    // summed number cannot say which -- so each is counted where the failure would show.
    let visible = orders.iter().filter(|order| order.visible).count();
    let selling = orders
        .iter()
        .filter(|order| order.kind == OrderKind::Sell)
        .count();
    let counted: Vec<_> = orders
        .iter()
        .filter(|order| order.visible && order.kind == OrderKind::Sell)
        .collect();
    println!("orders_visible={visible}");
    println!("orders_sell={selling}");
    println!("orders_counted_towards_listed_value={}", counted.len());

    let mut per_trade: Vec<u32> = orders.iter().map(|order| order.per_trade).collect();
    per_trade.sort_unstable();
    per_trade.dedup();
    // A price is a price per trade, so a `perTrade` this client read as something other than what
    // the account carries divides the total wrongly. The distinct values are not account data in
    // any meaningful sense -- there are six of them possible -- and they say at a glance whether
    // the field arrived at all.
    println!("per_trade_values_seen={per_trade:?}");

    let listed_platinum: u32 = counted
        .iter()
        .map(|order| {
            order
                .platinum
                .saturating_mul(order.quantity / order.per_trade.max(1))
        })
        .sum();
    // The one figure that is a price. Printed because the whole point of this run is to compare it
    // against what the screen shows, and an account's own total is not a secret from its owner.
    println!("listed_platinum={listed_platinum}");
}
