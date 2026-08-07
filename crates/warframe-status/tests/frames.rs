//! The frames, checked against what the server accepts.
//!
//! There is no way to assert against the live socket without changing a real account's presence,
//! so what is testable here is exactly the wire shape: the routes, the payload keys, and reading
//! back what the server says it committed.
use warframe_status::{
    Presence, committed_status, is_sign_in_success, is_signin_refusal, set_status_frame,
    sign_in_frame,
};

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
fn the_committed_status_is_read_from_the_answers_own_echo() {
    // Observed against the live server: every set is answered with the command echo, carrying the
    // status it just committed. It is the only confirmation later sets in a connection receive --
    // the event above arrives once, announcing the status held when the connection opened.
    assert_eq!(
        committed_status(
            r#"{"route":"@wfm|cmd/status/set:ok","payload":{"status":"ingame","statusSetAt":"2026-08-07T12:44:32Z"},"meta":{"stream":"status:abc","revision":2}}"#
        ),
        Some(Presence::Ingame)
    );
    assert_eq!(
        committed_status(r#"{"route":"@wfm|cmd/status/set:ok","payload":{"status":"online"}}"#),
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

#[test]
fn an_accepted_credential_is_recognised_from_its_own_frame() {
    // The counterpart of the refusal: the server answers a good token on the same route, with
    // `:ok` where the refusal reads `:error`.
    assert!(is_sign_in_success(
        r#"{"route":"@wfm|cmd/auth/signIn:ok","payload":{}}"#
    ));
    assert!(!is_sign_in_success(
        r#"{"route":"@wfm|cmd/auth/signIn:error","payload":"app.jwt.invalid"}"#
    ));
    // The status event is a different message and must not be mistaken for the acceptance: a
    // client that took it for the go-ahead would start speaking before the credential was in.
    assert!(!is_sign_in_success(
        r#"{"route":"@wfm|event/status/set","payload":{"status":"online"}}"#
    ));
    assert!(!is_sign_in_success("not json"));
}

#[test]
fn a_refused_credential_is_recognised_from_its_own_frame() {
    // Observed against the live server: a bad token is answered with this, and the connection is
    // then left open. Nothing about the socket says the sign-in failed, so the frame has to.
    assert!(is_signin_refusal(
        r#"{"route":"@wfm|cmd/auth/signIn:error","payload":"app.jwt.invalid"}"#
    ));
    assert!(!is_signin_refusal(
        r#"{"route":"@wfm|event/reports/online","payload":{"connections":38870}}"#
    ));
    assert!(!is_signin_refusal("not json"));
}
