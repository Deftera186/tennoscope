//! Live warframe.market pricing for the reward advisor.
//!
//! Ducats alone cannot rank relic rewards: most commons are worth the same 15, so a ducat-ordered
//! advisor calls a coin flip. Platinum is the number that separates them. In one captured run the
//! four cards were 2p, 1p, 3p and 25p while three of them tied on ducats.
//!
//! Prices are fetched off the publication path so a slow or unreachable API delays nothing; the
//! overlay shows the cards immediately and fills prices in when they arrive.

use std::{io::Read, time::Duration};

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
