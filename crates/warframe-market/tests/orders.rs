mod common;

use common::{FakeTransport, ok, ok_with_token, status};
use warframe_market::{
    MarketError, MarketToken, Method, OrderKind, delete_order, list_mine, set_order_quantity,
};

const FAKE_TOKEN: &str = "eyJhbGciOiJub25lIn0.eyJzdWIiOiJ0ZXN0In0.";

/// The shape of `/v2/orders/my`: every order on the account, visible and hidden, in one request.
const ORDERS: &str = r#"{"apiVersion":"0.25.0","data":[
    {"id":"order-one","itemId":"54a73e65e779893a797fff33","type":"sell","platinum":12,
     "quantity":3,"perTrade":1,"visible":true,"updatedAt":"2026-07-30T10:00:00Z"},
    {"id":"order-two","itemId":"54ca39abe7798915c1c11e10","type":"buy","platinum":40,
     "quantity":1,"perTrade":1,"rank":5,"visible":false,"updatedAt":"2026-07-29T08:00:00Z"}
],"error":null}"#;

fn token() -> MarketToken {
    MarketToken::new(FAKE_TOKEN.to_owned())
}

#[test]
fn every_order_on_the_account_is_read() {
    let transport = FakeTransport::new(vec![ok(ORDERS)]);

    let (orders, _) = list_mine(&transport, &token()).expect("orders load");

    assert_eq!(orders.len(), 2);
    assert_eq!(orders[0].id, "order-one");
    assert_eq!(orders[0].kind, OrderKind::Sell);
    assert_eq!(orders[0].platinum, 12);
    assert_eq!(orders[0].quantity, 3);
    assert!(orders[0].visible);
}

/// A hidden order is still an order the player holds, and still reconciles: an invisible listing
/// for something they no longer own becomes visible the moment they toggle it.
#[test]
fn hidden_orders_are_kept() {
    let transport = FakeTransport::new(vec![ok(ORDERS)]);

    let (orders, _) = list_mine(&transport, &token()).expect("orders load");

    assert!(!orders[1].visible);
    assert_eq!(orders[1].kind, OrderKind::Buy);
    assert_eq!(orders[1].rank, Some(5));
}

#[test]
fn listing_sends_the_credential_to_the_account_route() {
    let transport = FakeTransport::new(vec![ok(ORDERS)]);

    list_mine(&transport, &token()).expect("orders load");

    let seen = transport.seen();
    assert!(seen[0].url.ends_with("/v2/orders/my"), "url: {}", seen[0].url);
    assert_eq!(seen[0].token.as_deref(), Some(FAKE_TOKEN));
    assert_eq!(seen[0].method, Method::Get);
}

#[test]
fn a_refused_credential_is_unauthorized_rather_than_an_empty_list() {
    let transport = FakeTransport::new(vec![status(401)]);

    let outcome = list_mine(&transport, &token());

    assert_eq!(outcome.unwrap_err(), MarketError::Unauthorized);
}

#[test]
fn rate_limiting_is_reported_as_itself() {
    let transport = FakeTransport::new(vec![status(429)]);

    assert_eq!(
        list_mine(&transport, &token()).unwrap_err(),
        MarketError::RateLimited
    );
}

#[test]
fn deleting_an_order_names_it_in_the_path() {
    let transport = FakeTransport::new(vec![ok(r#"{"apiVersion":"0.25.0","data":{},"error":null}"#)]);

    delete_order(&transport, &token(), "order-one").expect("delete succeeds");

    let seen = transport.seen();
    assert_eq!(seen[0].method, Method::Delete);
    assert!(seen[0].url.ends_with("/v2/order/order-one"), "url: {}", seen[0].url);
}

#[test]
fn lowering_a_quantity_patches_only_the_quantity() {
    let transport = FakeTransport::new(vec![ok(r#"{"apiVersion":"0.25.0","data":{},"error":null}"#)]);

    set_order_quantity(&transport, &token(), "order-one", 1).expect("patch succeeds");

    let seen = transport.seen();
    assert_eq!(seen[0].method, Method::Patch);
    assert!(seen[0].url.ends_with("/v2/order/order-one"), "url: {}", seen[0].url);
    let body = seen[0].body.as_deref().expect("patch sends a body");
    assert!(body.contains("\"quantity\":1"), "body: {body}");
    // Nothing else is touched. A patch that also sent the price would silently reprice an order
    // the player only asked to shrink.
    assert!(!body.contains("platinum"), "body: {body}");
}

/// A quantity of zero would be a deletion wearing a patch's clothes, and the two are different
/// actions with different buttons. Refused here rather than at the interface, so no caller can
/// route round it.
#[test]
fn a_quantity_of_zero_is_refused() {
    let transport = FakeTransport::new(Vec::new());

    let outcome = set_order_quantity(&transport, &token(), "order-one", 0);

    assert_eq!(outcome.unwrap_err(), MarketError::Rejected);
    assert!(transport.seen().is_empty(), "no request should have been sent");
}

/// An id carrying a `/` would address something other than one order in the interpolated path.
/// These writes are irreversible against a real account, so this is caught before any request.
#[test]
fn deleting_with_a_slash_in_the_id_is_refused() {
    let transport = FakeTransport::new(Vec::new());

    let outcome = delete_order(&transport, &token(), "order/one");

    assert_eq!(outcome.unwrap_err(), MarketError::Rejected);
    assert!(transport.seen().is_empty(), "no request should have been sent");
}

#[test]
fn patching_with_a_slash_in_the_id_is_refused() {
    let transport = FakeTransport::new(Vec::new());

    let outcome = set_order_quantity(&transport, &token(), "order/one", 1);

    assert_eq!(outcome.unwrap_err(), MarketError::Rejected);
    assert!(transport.seen().is_empty(), "no request should have been sent");
}

/// Every authenticated call may reissue the token, and the writes are authenticated calls. Missing
/// it here would expire an account that deletes orders regularly.
#[test]
fn a_write_returns_the_renewed_token() {
    let renewed = "eyJhbGciOiJub25lIn0.eyJzdWIiOiJyZW5ld2VkIn0.";
    let transport = FakeTransport::new(vec![ok_with_token("{}", renewed)]);

    let token = delete_order(&transport, &token(), "order-one").expect("delete succeeds");

    assert_eq!(token.expose(), renewed);
}

#[test]
fn a_malformed_order_list_is_rejected() {
    let transport = FakeTransport::new(vec![ok("not json")]);

    assert_eq!(
        list_mine(&transport, &token()).unwrap_err(),
        MarketError::Malformed
    );
}
