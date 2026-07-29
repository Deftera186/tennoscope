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
/// The top orders endpoint returns at most five buy and five sell orders, measured at 4.9 KB
/// against 184 KB for the same item's full book. The cap is generous against that so a widened
/// payload does not silently stop every price.
const MAX_ORDERS_BYTES: usize = 256 * 1024;
const USER_AGENT: &str = concat!(
    "TennoScope/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Deftera186/tennoscope)"
);

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

/// Why an item has no price, kept distinct because the four reasons want different responses.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PriceLookup {
    Priced(u32),
    /// The item is listed, but nobody selling it is online.
    NoSellers,
    Unavailable,
    /// The response exceeded the size cap. Its own outcome so diagnostics can name it.
    Oversize,
}

pub trait MarketPriceSource {
    fn lowest_sell(&self, name: &str) -> PriceLookup;
}

#[derive(Deserialize)]
struct TopResponse {
    data: TopOrders,
}

#[derive(Deserialize)]
struct TopOrders {
    #[serde(default)]
    sell: Vec<Order>,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Order {
    platinum: u32,
    #[serde(default)]
    visible: bool,
    user: OrderUser,
    /// How many units one trade moves. `platinum` is the price of that whole trade, so a seller
    /// listing six relics for 18p is asking 3p each, not 18p each.
    #[serde(default = "one")]
    per_trade: u32,
}

#[derive(Deserialize)]
struct OrderUser {
    #[serde(default)]
    status: String,
}

pub fn lowest_sell_top(body: &[u8]) -> PriceLookup {
    let Ok(response) = serde_json::from_slice::<TopResponse>(body) else {
        return PriceLookup::Unavailable;
    };
    response
        .data
        .sell
        .into_iter()
        .filter(|order| order.visible && order.user.status == "ingame")
        .map(|order| {
            // `platinum` buys `perTrade` units, so the per-unit price is the quotient. Rounded to
            // nearest rather than truncated: 12p for five is 2.4p each, and truncation would quote
            // 2p and understate every bulk seller. A malformed count of zero is treated as one
            // rather than dividing by it. Floored at 1p because a bulk listing that rounds to zero
            // -- 1p for six -- renders as "0p", which reads as free rather than as cheap.
            let per_trade = order.per_trade.max(1);
            ((order.platinum + per_trade / 2) / per_trade).max(1)
        })
        .min()
        .map_or(PriceLookup::NoSellers, PriceLookup::Priced)
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
    fn lowest_sell(&self, name: &str) -> PriceLookup {
        let slug = market_slug(name);
        if slug.is_empty() {
            return PriceLookup::Unavailable;
        }
        let Ok(response) = self.client.get(format!("{ORDERS_URL}{slug}/top")).send() else {
            return PriceLookup::Unavailable;
        };
        let Ok(response) = response.error_for_status() else {
            return PriceLookup::Unavailable;
        };
        let mut body = Vec::new();
        if response
            .take((MAX_ORDERS_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .is_err()
        {
            return PriceLookup::Unavailable;
        }
        if body.len() > MAX_ORDERS_BYTES {
            return PriceLookup::Oversize;
        }
        lowest_sell_top(&body)
    }
}

/// How long a fetched price stays usable. Relic part prices drift over hours, not over one
/// mission, so this only exists to stop a session left running overnight from quoting yesterday.
const PRICE_TTL: Duration = Duration::from_secs(15 * 60);

/// The closest together two requests may leave this process, whatever the caller asked for.
///
/// warframe.market documents three per second for the public API. That is a limit on the client,
/// not on one caller: the relic pool warm, a collection page refresh and a reward screen fill all
/// share this cache and can run at once, and three callers each politely pacing themselves at
/// 334ms is still nine requests a second arriving at the API.
pub const MARKET_MIN_GAP: Duration = Duration::from_millis(334);

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
    /// When the last request left this process, shared by every caller so the rate limit is the
    /// client's rather than each caller's own.
    last_request: Arc<Mutex<Option<Instant>>>,
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

    /// Block until a request may be sent, then claim that slot.
    ///
    /// The claim is made under the lock and before the request is sent, so two threads arriving
    /// together are spaced rather than both cleared: the second one waits out the first's slot.
    fn take_request_slot(&self, gap: Duration) {
        let gap = gap.max(MARKET_MIN_GAP);
        let Ok(mut last) = self.last_request.lock() else {
            // A poisoned lock is not a licence to flood the API.
            std::thread::sleep(gap);
            return;
        };
        if let Some(previous) = *last
            && let Some(remaining) = gap.checked_sub(previous.elapsed())
        {
            std::thread::sleep(remaining);
        }
        *last = Some(Instant::now());
    }

    /// Price every name not already held, keeping requests at least `gap` apart.
    ///
    /// The gap is what keeps a pool of two dozen names from arriving at warframe.market as a burst.
    /// There is no hurry -- the whole point is that this runs minutes early -- so it is cheap to be
    /// polite. A caller in a hurry may pass `Duration::ZERO` to skip its own extra politeness, but
    /// `MARKET_MIN_GAP` still applies across every caller.
    pub fn warm(
        &self,
        source: &dyn MarketPriceSource,
        names: &[String],
        gap: Duration,
    ) -> WarmOutcome {
        let mut outcome = WarmOutcome::default();
        for name in names {
            if self.get(name).is_some() {
                continue;
            }
            self.take_request_slot(gap);
            match source.lowest_sell(name) {
                PriceLookup::Priced(price) => {
                    self.insert(name, price);
                    outcome.stored += 1;
                }
                PriceLookup::NoSellers => outcome.no_sellers += 1,
                PriceLookup::Unavailable => outcome.unavailable += 1,
                PriceLookup::Oversize => outcome.oversize += 1,
            }
        }
        outcome
    }
}

/// What a warm pass achieved, and what it ran into.
///
/// The counts exist so a caller can tell the diagnostics row something true. Without them every
/// failure arrives as the same absent price, and "warframe.market is sending us more than we will
/// read" is indistinguishable from "nobody is selling this" -- which is the difference between a
/// client that needs fixing and an ordinary quiet evening.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WarmOutcome {
    pub stored: usize,
    pub no_sellers: usize,
    pub unavailable: usize,
    pub oversize: usize,
}

impl WarmOutcome {
    /// What the market health row should say, when there is anything worth saying.
    ///
    /// Oversize outranks everything: it does not fix itself, it hits every item at once, and as an
    /// absent price it presents as "the whole collection is worthless" with nothing saying
    /// otherwise. An unreachable endpoint is reported whether or not the pass priced anything --
    /// a 65-relic sweep that lost half its prices to an outage is not a healthy pass, and reading
    /// Ready off it is how a player concludes those relics are simply worthless. A pass that priced
    /// something and merely found an item unsold is not news.
    pub fn failure(self) -> Option<&'static str> {
        if self.oversize > 0 {
            return Some("warframe.market responses are over the size cap; no price can be read");
        }
        if self.unavailable > 0 {
            return Some(if self.stored > 0 {
                "warframe.market answered for only some items; the rest are unpriced"
            } else {
                "warframe.market could not be reached"
            });
        }
        if self.stored > 0 {
            return None;
        }
        (self.no_sellers > 0).then_some("No live warframe.market sellers for these items")
    }
}
