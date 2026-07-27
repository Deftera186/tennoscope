use std::time::Duration;

use warframe_acquisition::{MarketPriceCache, MarketPriceSource, lowest_sell_price, market_slug};

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

/// A source that records every name it is asked for, so a test can prove the cache stopped a
/// request rather than merely returning the right number.
struct CountingMarket {
    priced: std::collections::HashMap<String, u32>,
    asked: std::sync::Mutex<Vec<String>>,
}

impl CountingMarket {
    fn new(priced: &[(&str, u32)]) -> Self {
        Self {
            priced: priced
                .iter()
                .map(|(name, price)| ((*name).to_owned(), *price))
                .collect(),
            asked: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn asked(&self) -> Vec<String> {
        self.asked.lock().unwrap().clone()
    }
}

impl MarketPriceSource for CountingMarket {
    fn lowest_sell(&self, name: &str) -> Option<u32> {
        self.asked.lock().unwrap().push(name.to_owned());
        self.priced.get(name).copied()
    }
}

fn names(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

/// The relic pool is known when the relics load, minutes before the reward screen exists. Warming
/// it then is the whole point: by the time four of those rewards are on screen the prices are
/// already local, instead of the player watching dashes during the fifteen seconds they have to
/// decide.
#[test]
fn warming_the_pool_prices_it_before_the_reward_screen_asks() {
    let market =
        CountingMarket::new(&[("Braton Prime Blueprint", 12), ("Trumna Prime Barrel", 45)]);
    let cache = MarketPriceCache::new();

    let stored = cache.warm(
        &market,
        &names(&["Braton Prime Blueprint", "Trumna Prime Barrel"]),
        Duration::ZERO,
    );

    assert_eq!(stored, 2);
    assert_eq!(cache.get("Braton Prime Blueprint"), Some(12));
    assert_eq!(cache.get("Trumna Prime Barrel"), Some(45));
}

#[test]
fn a_warmed_price_is_not_requested_again() {
    let market = CountingMarket::new(&[("Braton Prime Blueprint", 12)]);
    let cache = MarketPriceCache::new();
    cache.warm(&market, &names(&["Braton Prime Blueprint"]), Duration::ZERO);

    cache.warm(
        &market,
        &names(&["Braton Prime Blueprint", "Trumna Prime Barrel"]),
        Duration::ZERO,
    );

    assert_eq!(
        market.asked(),
        vec!["Braton Prime Blueprint", "Trumna Prime Barrel"],
        "the cached name must not be requested a second time"
    );
}

/// Forma is untradeable and an unreachable API looks identical from here. Storing either as a
/// price would leave the card permanently unpriced, so a miss stays a miss and is retried.
#[test]
fn an_unpriced_name_is_retried_rather_than_cached_as_a_failure() {
    let market = CountingMarket::new(&[]);
    let cache = MarketPriceCache::new();

    cache.warm(&market, &names(&["Forma Blueprint"]), Duration::ZERO);
    cache.warm(&market, &names(&["Forma Blueprint"]), Duration::ZERO);

    assert_eq!(cache.get("Forma Blueprint"), None);
    assert_eq!(market.asked().len(), 2, "a miss must not be cached");
}
