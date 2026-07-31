mod common;

use common::{FakeTransport, ok, ok_with_token, status};
use warframe_market::{MarketError, MarketToken, Method, sign_in, verify_token};

/// A fake token. Shaped like a JWT so the parsing is exercised, but signed by nobody and
/// belonging to no account.
const FAKE_TOKEN: &str = "eyJhbGciOiJub25lIn0.eyJzdWIiOiJ0ZXN0In0.";

/// The token arrives in the response header, not in the body. This is the one fact about the v1
/// signin route that a reader would otherwise get wrong, because every other endpoint in this
/// application answers in its body.
#[test]
fn signin_reads_the_token_from_the_response_header() {
    let transport = FakeTransport::new(vec![ok_with_token(
        r#"{"payload":{"user":{"ingame_name":"someone"}}}"#,
        FAKE_TOKEN,
    )]);

    let token = sign_in(&transport, "player@example.invalid", "not-a-real-password")
        .expect("signin succeeds");

    assert_eq!(token.expose(), FAKE_TOKEN);
}

/// The request carries the shape the v1 route requires: `auth_type: header` in the body, and a
/// seed `Authorization: JWT` header. Without either, the route answers with the token in a cookie
/// this client does not read, or refuses outright.
#[test]
fn signin_asks_for_a_header_token() {
    let transport = FakeTransport::new(vec![ok_with_token("{}", FAKE_TOKEN)]);

    sign_in(&transport, "player@example.invalid", "not-a-real-password").expect("signin succeeds");

    let seen = transport.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].method, Method::Post);
    assert!(
        seen[0].url.ends_with("/v1/auth/signin"),
        "signin must use the v1 route: {}",
        seen[0].url
    );
    let body = seen[0].body.as_deref().expect("signin sends a body");
    assert!(body.contains("\"auth_type\":\"header\""), "body: {body}");
}

#[test]
fn rejected_credentials_are_reported_as_a_rejection() {
    let transport = FakeTransport::new(vec![status(401)]);

    let outcome = sign_in(&transport, "player@example.invalid", "not-a-real-password");

    assert_eq!(outcome.unwrap_err(), MarketError::Rejected);
}

/// A 404 from the signin route means the route itself is gone, which is a different problem from
/// a wrong password and wants a different answer from the interface: the paste-token path still
/// works, and telling the player their password was wrong would send them to change it.
#[test]
fn a_missing_signin_route_is_not_reported_as_a_bad_password() {
    let transport = FakeTransport::new(vec![status(404)]);

    let outcome = sign_in(&transport, "player@example.invalid", "not-a-real-password");

    assert_eq!(outcome.unwrap_err(), MarketError::SigninUnavailable);
}

/// A signin that answers 200 with no token is malformed rather than successful. Treating it as
/// success would store an empty credential and report the account linked.
#[test]
fn a_signin_without_a_token_is_malformed() {
    let transport = FakeTransport::new(vec![ok("{}")]);

    let outcome = sign_in(&transport, "player@example.invalid", "not-a-real-password");

    assert_eq!(outcome.unwrap_err(), MarketError::Malformed);
}

/// A pasted token is checked before it is stored, so a bad paste fails where the player can see
/// the paste box rather than at the next action.
#[test]
fn a_pasted_token_is_verified_against_the_account_route() {
    let transport = FakeTransport::new(vec![ok(r#"{"apiVersion":"0.25.0","data":{},"error":null}"#)]);

    let token = verify_token(&transport, &MarketToken::new(FAKE_TOKEN.to_owned()))
        .expect("a valid token verifies");

    assert_eq!(token.expose(), FAKE_TOKEN);
    let seen = transport.seen();
    assert!(seen[0].url.ends_with("/v2/me"), "url: {}", seen[0].url);
    assert_eq!(seen[0].token.as_deref(), Some(FAKE_TOKEN));
}

#[test]
fn a_refused_token_is_unauthorized() {
    let transport = FakeTransport::new(vec![status(401)]);

    let outcome = verify_token(&transport, &MarketToken::new(FAKE_TOKEN.to_owned()));

    assert_eq!(outcome.unwrap_err(), MarketError::Unauthorized);
}

/// Every authenticated call may hand back a renewed token, so an account in regular use never
/// expires. Verification takes that path too.
#[test]
fn a_renewed_token_replaces_the_one_that_was_sent() {
    let renewed = "eyJhbGciOiJub25lIn0.eyJzdWIiOiJyZW5ld2VkIn0.";
    let transport = FakeTransport::new(vec![ok_with_token("{}", renewed)]);

    let token = verify_token(&transport, &MarketToken::new(FAKE_TOKEN.to_owned()))
        .expect("verification succeeds");

    assert_eq!(token.expose(), renewed);
}

/// The rule that keeps this feature honest, asserted rather than commented.
///
/// A token in a `Debug` rendering reaches the log the first time anyone writes `{:?}` on a struct
/// that holds one, which is exactly the kind of edit nobody reviews closely. The password is
/// checked in the same test because it travels the same path and has the same consequence.
#[test]
fn neither_the_token_nor_the_password_survives_into_a_rendering() {
    let password = "not-a-real-password";
    let token = MarketToken::new(FAKE_TOKEN.to_owned());

    let rendered = format!("{token:?}");
    assert!(!rendered.contains(FAKE_TOKEN), "token leaked: {rendered}");
    assert!(rendered.contains("redacted"), "unhelpful rendering: {rendered}");

    let transport = FakeTransport::new(vec![status(401)]);
    let error = sign_in(&transport, "player@example.invalid", password).unwrap_err();
    let rendered = format!("{error:?} {error}");
    assert!(!rendered.contains(password), "password leaked: {rendered}");
    assert!(
        !rendered.contains("player@example.invalid"),
        "address leaked: {rendered}"
    );
}
