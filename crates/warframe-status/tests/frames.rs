//! The frames, checked against what the server accepts.
//!
//! There is no way to assert against the live socket without changing a real account's presence,
//! so what is testable here is exactly the wire shape: the routes, the payload keys, and reading
//! back what the server says it committed.
use warframe_status::{Presence, committed_status, set_status_frame, sign_in_frame};

#[test]
fn sign_in_carries_the_token_on_the_auth_route() {
    let frame: serde_json::Value = serde_json::from_str(&sign_in_frame("jwt-value")).unwrap();

    assert_eq!(frame["route"], "@wfm|cmd/auth/signIn");
    assert_eq!(frame["payload"]["token"], "jwt-value");
}

#[test]
fn a_token_with_a_quote_stays_one_json_string() {
    let frame: serde_json::Value = serde_json::from_str(&sign_in_frame(r#"a"b\c"#)).unwrap();

    assert_eq!(frame["payload"]["token"], r#"a"b\c"#);
}

#[test]
fn set_status_names_the_status_and_nothing_else() {
    let frame: serde_json::Value =
        serde_json::from_str(&set_status_frame(Presence::Ingame)).unwrap();

    assert_eq!(frame["route"], "@wfm|cmd/status/set");
    assert_eq!(frame["payload"]["status"], "ingame");
    // A duration would make the status expire while the application is still running.
    assert!(frame["payload"].get("duration").is_none());
    assert!(frame["payload"].get("activity").is_none());
}

#[test]
fn the_committed_status_is_read_from_the_servers_own_event() {
    assert_eq!(
        committed_status(r#"{"route":"@wfm|event/status/set","payload":{"status":"invisible"}}"#),
        Some(Presence::Invisible)
    );
    assert_eq!(
        committed_status(r#"{"route":"@wfm|event/status/set","payload":"online"}"#),
        Some(Presence::Online)
    );
}

#[test]
fn nothing_is_read_from_a_frame_that_is_not_that_event() {
    // Every other route on this socket -- order events, chat, subscriptions -- goes past this
    // function, and a status read out of one of them would be a presence nobody set.
    assert_eq!(
        committed_status(r#"{"route":"@wfm|event/orders/new","payload":{"status":"ingame"}}"#),
        None
    );
    assert_eq!(committed_status("not json"), None);
    assert_eq!(
        committed_status(r#"{"route":"@wfm|event/status/set","payload":{"status":"offline"}}"#),
        None,
        "offline is observed on the profile, not a value this client models as settable"
    );
}
