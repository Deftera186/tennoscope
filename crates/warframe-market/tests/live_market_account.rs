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
use warframe_market::{MarketHttp, MarketItems, MarketToken, list_mine, verify_token};

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
}
