mod common;

use common::FakeTransport;
use warframe_market::{MarketError, MarketRequest, MarketResponse, MarketTransport, Method};

#[test]
fn the_transport_records_what_it_was_asked_for() {
    let transport = FakeTransport::new(vec![Ok(MarketResponse {
        status: 200,
        authorization: None,
        body: b"{}".to_vec(),
    })]);

    let response = transport
        .send(MarketRequest {
            method: Method::Get,
            url: "https://api.warframe.market/v2/orders/my".to_owned(),
            token: Some("fake-token".to_owned()),
            body: None,
        })
        .expect("fake transport answers");

    assert_eq!(response.status, 200);
    assert_eq!(transport.seen().len(), 1);
    assert_eq!(transport.seen()[0].method, Method::Get);
}

/// A transport with nothing left to say reports an unreachable API rather than panicking, so a
/// test that makes one request too many fails as a behaviour rather than as a crash.
#[test]
fn an_exhausted_transport_reports_the_api_unreachable() {
    let transport = FakeTransport::new(Vec::new());

    let outcome = transport.send(MarketRequest {
        method: Method::Get,
        url: "https://api.warframe.market/v2/me".to_owned(),
        token: None,
        body: None,
    });

    assert!(matches!(outcome, Err(MarketError::Unreachable)));
}
