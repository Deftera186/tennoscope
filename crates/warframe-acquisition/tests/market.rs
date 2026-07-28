use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use warframe_acquisition::{
    MARKET_MIN_GAP, MarketPriceCache, MarketPriceSource, PriceLookup, lowest_sell_top, market_slug,
};

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

/// The shape of `/v2/orders/item/{slug}/top`: top buy and sell orders, already filtered to sellers
/// who are online, at 4.9 KB against 184 KB for the full order book.
const TOP: &str = r#"{"apiVersion":"0.25.0","data":{
    "sell":[
        {"type":"sell","platinum":19,"visible":true,"user":{"status":"ingame"}},
        {"type":"sell","platinum":20,"visible":true,"user":{"status":"ingame"}}
    ],
    "buy":[{"type":"buy","platinum":12,"visible":true,"user":{"status":"ingame"}}]
},"error":null}"#;

#[test]
fn the_cheapest_online_seller_sets_the_price() {
    assert_eq!(lowest_sell_top(TOP.as_bytes()), PriceLookup::Priced(19));
}

/// An offline seller's price is a number nobody can trade at. Counting them makes every item look
/// cheaper than it is, which would push the advisor toward the wrong card.
#[test]
fn an_offline_or_hidden_seller_is_not_quotable() {
    let body = r#"{"data":{"sell":[
        {"type":"sell","platinum":2,"visible":true,"user":{"status":"offline"}},
        {"type":"sell","platinum":3,"visible":false,"user":{"status":"ingame"}},
        {"type":"sell","platinum":25,"visible":true,"user":{"status":"ingame"}}
    ],"buy":[]}}"#;
    assert_eq!(lowest_sell_top(body.as_bytes()), PriceLookup::Priced(25));
}

#[test]
fn an_item_with_no_online_seller_is_distinct_from_a_failure() {
    let body = r#"{"data":{"sell":[
        {"type":"sell","platinum":2,"visible":true,"user":{"status":"offline"}}
    ],"buy":[]}}"#;
    assert_eq!(lowest_sell_top(body.as_bytes()), PriceLookup::NoSellers);
}

/// The failure that arrives the day warframe.market widens its payload. As an absent price it
/// would present as "every item is worthless", with nothing anywhere saying otherwise.
#[test]
fn an_unreadable_body_is_reported_rather_than_priced_at_nothing() {
    assert_eq!(lowest_sell_top(b"{not json"), PriceLookup::Unavailable);
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
    fn lowest_sell(&self, name: &str) -> PriceLookup {
        self.asked.lock().unwrap().push(name.to_owned());
        self.priced
            .get(name)
            .copied()
            .map_or(PriceLookup::NoSellers, PriceLookup::Priced)
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

/// Three call paths share one cache -- the relic pool warm, a collection page refresh and the
/// reward screen's fill -- and any two can run at once. Pacing each caller separately puts twice
/// the documented rate on the API, so the floor has to belong to the cache, not to the caller.
/// The reward fill asks for no gap of its own and must still not be able to breach it.
#[test]
fn two_concurrent_warms_cannot_out_pace_the_shared_floor() {
    let market = Arc::new(StampingMarket::default());
    let cache = MarketPriceCache::new();

    std::thread::scope(|scope| {
        for names in [
            names(&["Ash Prime Blueprint", "Nikana Prime Blade"]),
            names(&["Volt Prime Chassis Blueprint", "Soma Prime Barrel"]),
        ] {
            let market = Arc::clone(&market);
            let cache = cache.clone();
            scope.spawn(move || {
                cache.warm(market.as_ref(), &names, Duration::ZERO);
            });
        }
    });

    let stamps = market.stamps.lock().unwrap().clone();
    assert_eq!(stamps.len(), 4);
    for pair in stamps.windows(2) {
        let apart = pair[1].duration_since(pair[0]);
        assert!(
            apart >= MARKET_MIN_GAP,
            "requests {apart:?} apart, under the {MARKET_MIN_GAP:?} floor"
        );
    }
}

/// Records when each request was made, so a test can measure the spacing rather than trust it.
#[derive(Default)]
struct StampingMarket {
    stamps: std::sync::Mutex<Vec<Instant>>,
}

impl MarketPriceSource for StampingMarket {
    fn lowest_sell(&self, _name: &str) -> PriceLookup {
        self.stamps.lock().unwrap().push(Instant::now());
        PriceLookup::NoSellers
    }
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
