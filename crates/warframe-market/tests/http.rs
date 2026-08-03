use warframe_acquisition::RequestPacer;
use warframe_market::{MarketHttp, USER_AGENT};

/// The identifying user agent, so warframe.market can attribute this client's traffic and ask its
/// author to stop if it misbehaves. An anonymous client is one they can only block.
#[test]
fn the_user_agent_names_this_application_and_its_home() {
    assert!(USER_AGENT.starts_with("TennoScope/"), "{USER_AGENT}");
    assert!(
        USER_AGENT.contains("github.com/Deftera186/tennoscope"),
        "{USER_AGENT}"
    );
}

/// Constructing must not perform a request, so a launch with no network still starts the
/// application rather than blocking on a client build that reaches out.
#[test]
fn a_transport_builds_without_touching_the_network() {
    assert!(MarketHttp::new(RequestPacer::new()).is_ok());
}
