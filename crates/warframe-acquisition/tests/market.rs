use warframe_acquisition::{lowest_sell_price, market_slug};

/// Verified against the live warframe.market v2 API for every reward name observed in a real run.
#[test]
fn reward_names_become_market_slugs() {
    for (name, slug) in [
        ("Braton Prime Blueprint", "braton_prime_blueprint"),
        ("Burston Prime Stock", "burston_prime_stock"),
        ("Paris Prime Lower Limb", "paris_prime_lower_limb"),
        (
            "Xaku Prime Neuroptics Blueprint",
            "xaku_prime_neuroptics_blueprint",
        ),
        (
            "Titania Prime Systems Blueprint",
            "titania_prime_systems_blueprint",
        ),
        // Forma is not tradeable, so this slug has no market entry and the price stays unknown.
        ("2X Forma Blueprint", "2x_forma_blueprint"),
        ("Cobra & Crane Prime Hilt", "cobra_crane_prime_hilt"),
    ] {
        assert_eq!(market_slug(name), slug, "for {name}");
    }
}

#[test]
fn a_name_with_nothing_quotable_produces_no_slug() {
    assert_eq!(market_slug("   "), "");
    assert_eq!(market_slug("&&&"), "");
}

fn orders(body: &str) -> Option<u32> {
    lowest_sell_price(body.as_bytes())
}

/// An offline seller's price is a number nobody can trade at. Counting them makes every item look
/// cheaper than it is, which would push the advisor toward the wrong card.
#[test]
fn only_visible_sell_orders_from_sellers_in_game_are_quoted() {
    let body = r#"{"data":[
        {"type":"buy","platinum":1,"visible":true,"user":{"status":"ingame"}},
        {"type":"sell","platinum":2,"visible":true,"user":{"status":"offline"}},
        {"type":"sell","platinum":3,"visible":false,"user":{"status":"ingame"}},
        {"type":"sell","platinum":25,"visible":true,"user":{"status":"ingame"}},
        {"type":"sell","platinum":40,"visible":true,"user":{"status":"ingame"}}
    ]}"#;
    assert_eq!(orders(body), Some(25));
}

#[test]
fn an_item_with_no_live_sellers_has_no_price() {
    let body = r#"{"data":[
        {"type":"buy","platinum":1,"visible":true,"user":{"status":"ingame"}},
        {"type":"sell","platinum":2,"visible":true,"user":{"status":"offline"}}
    ]}"#;
    assert_eq!(orders(body), None);
}

#[test]
fn an_unreadable_or_empty_response_has_no_price() {
    assert_eq!(orders("not json"), None);
    assert_eq!(orders(r#"{"data":[]}"#), None);
    assert_eq!(orders(r#"{"error":"not found"}"#), None);
}
