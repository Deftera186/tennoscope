//! Live warframe.market pricing for the reward advisor.
//!
//! Ducats alone cannot rank relic rewards: most commons are worth the same 15, so a ducat-ordered
//! advisor calls a coin flip. Platinum is the number that separates them. In one captured run the
//! four cards were 2p, 1p, 3p and 25p while three of them tied on ducats.
//!
//! Prices are fetched off the publication path so a slow or unreachable API delays nothing; the
//! overlay shows the cards immediately and fills prices in when they arrive.

use std::{
    collections::HashMap,
    io::Read,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use reqwest::blocking::Client;
use serde::Deserialize;

const ORDERS_URL: &str = "https://api.warframe.market/v2/orders/item/";
const MAX_ORDERS_BYTES: usize = 8 * 1024 * 1024;
const USER_AGENT: &str = concat!("TennoScope/", env!("CARGO_PKG_VERSION"));

/// warframe.market's slug for a reward name: lowercase, non-alphanumerics collapsed to underscore.
///
/// Verified against the live API for every reward name observed so far. Items that are not
/// tradeable, Forma among them, simply have no entry and price as unknown.
pub fn market_slug(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    for character in name.chars() {
        if character.is_ascii_alphanumeric() {
            slug.extend(character.to_lowercase());
        } else if !slug.ends_with('_') {
            slug.push('_');
        }
    }
    slug.trim_matches('_').to_owned()
}

pub trait MarketPriceSource {
    /// Lowest visible sell price from a seller who is in game, or `None` when the item is not
    /// traded or the API cannot be reached.
    fn lowest_sell(&self, name: &str) -> Option<u32>;
}

#[derive(Deserialize)]
struct OrdersResponse {
    data: Vec<Order>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Order {
    #[serde(rename = "type")]
    order_type: String,
    platinum: u32,
    #[serde(default)]
    visible: bool,
    user: OrderUser,
}

#[derive(Deserialize)]
struct OrderUser {
    #[serde(default)]
    status: String,
}

/// Only orders from a seller who is actually in game are quotable. An offline seller's price is a
/// number nobody can trade at, and including them makes every item look cheaper than it is.
pub fn lowest_sell_price(body: &[u8]) -> Option<u32> {
    serde_json::from_slice::<OrdersResponse>(body)
        .ok()?
        .data
        .into_iter()
        .filter(|order| {
            order.order_type == "sell" && order.visible && order.user.status == "ingame"
        })
        .map(|order| order.platinum)
        .min()
}

pub struct WarframeMarketHttp {
    client: Client,
}

impl WarframeMarketHttp {
    pub fn new() -> Option<Self> {
        Client::builder()
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(8))
            .user_agent(USER_AGENT)
            .build()
            .ok()
            .map(|client| Self { client })
    }
}

impl MarketPriceSource for WarframeMarketHttp {
    fn lowest_sell(&self, name: &str) -> Option<u32> {
        let slug = market_slug(name);
        if slug.is_empty() {
            return None;
        }
        let response = self
            .client
            .get(format!("{ORDERS_URL}{slug}"))
            .send()
            .ok()?
            .error_for_status()
            .ok()?;
        let mut body = Vec::new();
        response
            .take((MAX_ORDERS_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .ok()?;
        if body.len() > MAX_ORDERS_BYTES {
            return None;
        }
        lowest_sell_price(&body)
    }
}

/// How long a fetched price stays usable. Relic part prices drift over hours, not over one
/// mission, so this only exists to stop a session left running overnight from quoting yesterday.
const PRICE_TTL: Duration = Duration::from_secs(15 * 60);

/// Prices already looked up, so the reward screen does not wait on requests that could have been
/// made minutes earlier.
///
/// EE.log names the squad's relics when they are *loaded*, which is a long way ahead of the reward
/// screen -- 125s in the replayed run, against a screen that lives for fifteen seconds. Pricing
/// only started when the cards were already on screen, so every card showed a dash until the
/// requests came back, which is exactly the moment the player is deciding. This fills that window:
/// everything the pool can drop is priced while the mission is still being played, and by the time
/// four of them are on screen the numbers are local.
///
/// Only successful lookups are stored. A miss is either an untradeable item or an unreachable API
/// and the two are indistinguishable from here, so caching misses would turn one failed request
/// into a card that stays unpriced for the rest of the session.
#[derive(Clone, Default)]
pub struct MarketPriceCache {
    entries: Arc<Mutex<HashMap<String, (u32, Instant)>>>,
}

impl MarketPriceCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, name: &str) -> Option<u32> {
        let entries = self.entries.lock().ok()?;
        let (price, fetched) = entries.get(name)?;
        (fetched.elapsed() < PRICE_TTL).then_some(*price)
    }

    pub fn insert(&self, name: &str, price: u32) {
        if let Ok(mut entries) = self.entries.lock() {
            entries.insert(name.to_owned(), (price, Instant::now()));
        }
    }

    /// Price every name not already held, pausing `gap` between requests.
    ///
    /// The gap is what keeps a pool of two dozen names from arriving at warframe.market as a burst.
    /// There is no hurry -- the whole point is that this runs minutes early -- so it is cheap to be
    /// polite. Returns how many new prices were stored.
    pub fn warm(&self, source: &dyn MarketPriceSource, names: &[String], gap: Duration) -> usize {
        let mut stored = 0;
        let mut requested = false;
        for name in names {
            if self.get(name).is_some() {
                continue;
            }
            if requested && !gap.is_zero() {
                std::thread::sleep(gap);
            }
            requested = true;
            if let Some(price) = source.lowest_sell(name) {
                self.insert(name, price);
                stored += 1;
            }
        }
        stored
    }
}
