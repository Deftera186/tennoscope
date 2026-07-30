use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use warframe_acquisition::{
    MARKET_MIN_GAP, MarketPriceCache, MarketPriceSource, PriceLookup, WarmOutcome, lowest_sell_top,
    market_slug, slug_and_subtype,
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
    assert_eq!(
        lowest_sell_top(TOP.as_bytes(), None),
        PriceLookup::Priced(19)
    );
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
    assert_eq!(
        lowest_sell_top(body.as_bytes(), None),
        PriceLookup::Priced(25)
    );
}

#[test]
fn an_item_with_no_online_seller_is_distinct_from_a_failure() {
    let body = r#"{"data":{"sell":[
        {"type":"sell","platinum":2,"visible":true,"user":{"status":"offline"}}
    ],"buy":[]}}"#;
    assert_eq!(
        lowest_sell_top(body.as_bytes(), None),
        PriceLookup::NoSellers
    );
}

/// The failure that arrives the day warframe.market widens its payload. As an absent price it
/// would present as "every item is worthless", with nothing anywhere saying otherwise.
#[test]
fn an_unreadable_body_is_reported_rather_than_priced_at_nothing() {
    assert_eq!(
        lowest_sell_top(b"{not json", None),
        PriceLookup::Unavailable
    );
}

/// A relic is one listing with four separately-priced refinement subtypes, so the request has to
/// name the tier. A bare relic name means intact -- the tier warframe.market shows by default, and
/// the one the refined tiers fall back to. Nothing else asks about a subtype at all.
#[test]
fn a_relic_asks_about_its_refinement_and_everything_else_asks_about_none() {
    for (name, slug, subtype) in [
        ("Axi A1 Relic", "axi_a1_relic", Some("intact")),
        ("Axi A1 Relic (Radiant)", "axi_a1_relic", Some("radiant")),
        (
            "Meso B2 Relic (Exceptional)",
            "meso_b2_relic",
            Some("exceptional"),
        ),
        ("Braton Prime Blueprint", "braton_prime_blueprint", None),
        // Brackets that are not a refinement belong to the name, not to a subtype.
        ("Rifle Riven Mod (Veiled)", "rifle_riven_mod_veiled", None),
    ] {
        let (built_slug, built_subtype) = slug_and_subtype(name);
        assert_eq!(built_slug, slug, "slug for {name}");
        assert_eq!(built_subtype.as_deref(), subtype, "subtype for {name}");
    }
}

/// The subtype is filtered client-side as well as in the query string, because warframe.market
/// answers a `/top` request carrying an unrecognised parameter by ignoring it. Trusting the server
/// alone would serve the cheap intact order under a radiant relic's name -- silently, and it is
/// the exact number the whole distinction exists to stop showing.
#[test]
fn a_subtype_the_server_did_not_filter_is_filtered_here() {
    let body = r#"{"data":{"sell":[
        {"type":"sell","platinum":5,"subtype":"intact","visible":true,"user":{"status":"ingame"}},
        {"type":"sell","platinum":90,"perTrade":6,"subtype":"radiant","visible":true,"user":{"status":"ingame"}}
    ],"buy":[]}}"#;
    assert_eq!(
        lowest_sell_top(body.as_bytes(), Some("radiant")),
        PriceLookup::Priced(15),
        "90p for six radiants is 15p each, and the 5p intact order is a different item"
    );
}

/// A tier nobody is selling is not a failure and not the intact price: it is the absent answer the
/// price table's fallback is there to notice.
#[test]
fn a_tier_with_no_seller_of_its_own_reports_no_sellers() {
    let body = r#"{"data":{"sell":[
        {"type":"sell","platinum":5,"subtype":"intact","visible":true,"user":{"status":"ingame"}}
    ],"buy":[]}}"#;
    assert_eq!(
        lowest_sell_top(body.as_bytes(), Some("flawless")),
        PriceLookup::NoSellers
    );
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

    let outcome = cache.warm(
        &market,
        &names(&["Braton Prime Blueprint", "Trumna Prime Barrel"]),
        Duration::ZERO,
    );

    assert_eq!(outcome.stored, 2);
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

/// A source that answers every name the same way.
struct FixedMarket(PriceLookup);

impl MarketPriceSource for FixedMarket {
    fn lowest_sell(&self, _name: &str) -> PriceLookup {
        self.0
    }
}

/// The failure that arrives the day warframe.market widens its payload. It hits every item at
/// once and it does not fix itself, so it must not arrive at the health row wearing the same
/// clothes as a quiet evening where nobody happened to be selling.
#[test]
fn an_oversize_response_reaches_the_caller_apart_from_an_item_nobody_is_selling() {
    let oversize = MarketPriceCache::new().warm(
        &FixedMarket(PriceLookup::Oversize),
        &names(&["Ash Prime Blueprint"]),
        Duration::ZERO,
    );
    let quiet = MarketPriceCache::new().warm(
        &FixedMarket(PriceLookup::NoSellers),
        &names(&["Ash Prime Blueprint"]),
        Duration::ZERO,
    );

    assert_eq!(oversize.oversize, 1);
    assert_eq!(quiet.no_sellers, 1);
    assert!(
        oversize
            .failure()
            .is_some_and(|say| say.contains("size cap")),
        "an oversize response must name itself: {:?}",
        oversize.failure()
    );
    assert_ne!(
        oversize.failure(),
        quiet.failure(),
        "the two failures must not read the same"
    );
}

/// A pass that priced something and merely found one item unsold is not a health problem.
#[test]
fn a_pass_that_priced_something_reports_nothing_to_the_health_row() {
    let cache = MarketPriceCache::new();
    let outcome = cache.warm(
        &FixedMarket(PriceLookup::Priced(12)),
        &names(&["Ash Prime Blueprint"]),
        Duration::ZERO,
    );

    assert_eq!(outcome.failure(), None);
}

/// An outage partway through a 65-relic sweep leaves most of the collection priced and the rest
/// silently unpriced. Reporting Ready because *something* was stored tells the player those relics
/// are worthless, when the truth is that nobody could ask.
#[test]
fn a_pass_that_reached_the_api_for_only_some_items_still_reports_it() {
    let partial = WarmOutcome {
        stored: 30,
        unavailable: 35,
        ..WarmOutcome::default()
    };

    assert!(
        partial.failure().is_some(),
        "half a sweep lost to an outage is not a healthy pass"
    );
    assert_ne!(
        partial.failure(),
        WarmOutcome {
            unavailable: 1,
            ..WarmOutcome::default()
        }
        .failure(),
        "priced some and priced none must not read the same"
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

/// `platinum` is the price for a whole trade of `perTrade` units, not for one unit. Relic sellers
/// routinely list six at a time, so comparing a six-pack's total against a single's price ranks
/// two different quantities as if they were the same thing.
#[test]
fn a_bulk_listing_is_quoted_per_unit_not_per_trade() {
    let body = r#"{"data":{"sell":[
        {"platinum":20,"perTrade":1,"visible":true,"user":{"status":"ingame"}},
        {"platinum":18,"perTrade":6,"visible":true,"user":{"status":"ingame"}}
    ],"buy":[]}}"#;
    assert_eq!(
        lowest_sell_top(body.as_bytes(), None),
        PriceLookup::Priced(3)
    );
}

/// A listing with no `perTrade` field is a single, not a free item.
#[test]
fn a_listing_without_a_per_trade_count_is_one_unit() {
    let body = r#"{"data":{"sell":[
        {"platinum":25,"visible":true,"user":{"status":"ingame"}}
    ],"buy":[]}}"#;
    assert_eq!(
        lowest_sell_top(body.as_bytes(), None),
        PriceLookup::Priced(25)
    );
}

/// Integer division would report a 5-for-12 listing at 2p and understate every bulk seller.
#[test]
fn a_per_unit_price_rounds_rather_than_truncating() {
    let body = r#"{"data":{"sell":[
        {"platinum":12,"perTrade":5,"visible":true,"user":{"status":"ingame"}}
    ],"buy":[]}}"#;
    assert_eq!(
        lowest_sell_top(body.as_bytes(), None),
        PriceLookup::Priced(2)
    );
}

/// A malformed `perTrade` of zero must not divide by zero or price the item at nothing.
#[test]
fn a_zero_per_trade_count_is_treated_as_one() {
    let body = r#"{"data":{"sell":[
        {"platinum":30,"perTrade":0,"visible":true,"user":{"status":"ingame"}}
    ],"buy":[]}}"#;
    assert_eq!(
        lowest_sell_top(body.as_bytes(), None),
        PriceLookup::Priced(30)
    );
}

/// The cheapest thing on the market still costs something. 1p for six rounds to nothing, and "0p"
/// on a card reads as free rather than as cheap -- a price of zero is the one number this app uses
/// to mean "worthless", so the cheapest real listing must not borrow it.
#[test]
fn a_bulk_listing_too_cheap_to_divide_is_still_worth_a_platinum() {
    let body = r#"{"data":{"sell":[
        {"platinum":1,"perTrade":6,"visible":true,"user":{"status":"ingame"}}
    ],"buy":[]}}"#;
    assert_eq!(
        lowest_sell_top(body.as_bytes(), None),
        PriceLookup::Priced(1)
    );
}

/// A source that answers each name differently, so a per-name pass can be checked for attributing
/// each verdict to the name that produced it.
struct PerName;

impl MarketPriceSource for PerName {
    fn lowest_sell(&self, name: &str) -> PriceLookup {
        match name {
            "Sold" => PriceLookup::Priced(19),
            "Quiet" => PriceLookup::NoSellers,
            _ => PriceLookup::Unavailable,
        }
    }
}

/// The startup sweep walks its names one at a time so it can record *which* one nobody was
/// selling: that answer is what stops the sweep re-asking about the same relic on every inventory
/// sync. A summed outcome cannot do it, so this pins the per-name accounting the loop reads.
///
/// The `Unavailable` half is the one that matters most. An unreachable endpoint must stay
/// distinguishable from an empty order book, because recording an outage as an answer would
/// blacklist a relic until the next day's dump over a router that rebooted mid-sweep.
#[test]
fn a_per_name_pass_can_tell_an_empty_order_book_from_an_unreachable_one() {
    let cache = MarketPriceCache::new();
    let verdicts: Vec<(&str, WarmOutcome)> = ["Sold", "Quiet", "Offline"]
        .into_iter()
        .map(|name| (name, cache.warm(&PerName, &names(&[name]), Duration::ZERO)))
        .collect();

    assert_eq!(verdicts[0].1.stored, 1);
    assert_eq!(verdicts[0].1.no_sellers, 0);
    assert_eq!(
        verdicts[1].1.no_sellers, 1,
        "the empty book is attributed to Quiet"
    );
    assert_eq!(verdicts[1].1.unavailable, 0);
    assert_eq!(
        verdicts[2].1.unavailable, 1,
        "an outage is its own verdict, and must not be recorded as an answer"
    );
    assert_eq!(verdicts[2].1.no_sellers, 0);
}
