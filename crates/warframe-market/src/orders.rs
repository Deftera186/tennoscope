//! The account's orders, and the two writes that reduce one.
//!
//! Both writes shrink something the player already published -- taking a listing down, or lowering
//! it to what they hold. Publishing something new is a separate action and is not in this phase.

use serde::{Deserialize, Serialize};

use crate::{
    API_V2, MarketError, MarketRequest, MarketToken, MarketTransport, Method, renewed_token,
};

/// The cap on the order list. An account with a few hundred orders is ordinary and each is a small
/// object; this bounds what one unexpected response can allocate without being tight enough to
/// break a large account.
pub const MAX_ORDERS_BYTES: usize = 4 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OrderKind {
    Sell,
    Buy,
}

#[derive(Deserialize)]
struct OrdersResponse {
    data: Vec<OrderRecord>,
}

fn one() -> u32 {
    1
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OrderRecord {
    id: String,
    item_id: String,
    #[serde(rename = "type")]
    kind: OrderKind,
    platinum: u32,
    #[serde(default = "one")]
    quantity: u32,
    #[serde(default = "one")]
    per_trade: u32,
    #[serde(default)]
    rank: Option<u32>,
    #[serde(default)]
    subtype: Option<String>,
    #[serde(default)]
    visible: bool,
    #[serde(default)]
    updated_at: Option<String>,
}

/// One order on the account.
///
/// Carries no player identity. The list is the player's own, so there is no other party to name,
/// and an account identifier here would be one more field to keep out of every log line.
#[derive(Clone, Debug, Serialize)]
pub struct MarketOrder {
    pub id: String,
    pub item_id: String,
    pub kind: OrderKind,
    pub platinum: u32,
    pub quantity: u32,
    /// How many units one trade moves. A seller listing six relics for 18p is asking 3p each.
    pub per_trade: u32,
    pub rank: Option<u32>,
    pub subtype: Option<String>,
    pub visible: bool,
    /// When warframe.market last saw this order change. Compared against the inventory snapshot's
    /// own timestamp: an order edited after the last snapshot cannot be judged against it.
    pub updated_at: Option<String>,
}

impl From<OrderRecord> for MarketOrder {
    fn from(record: OrderRecord) -> Self {
        Self {
            id: record.id,
            item_id: record.item_id,
            kind: record.kind,
            platinum: record.platinum,
            quantity: record.quantity,
            per_trade: record.per_trade.max(1),
            rank: record.rank,
            subtype: record.subtype,
            visible: record.visible,
            updated_at: record.updated_at,
        }
    }
}

/// Every order on the account, visible and hidden, in one request.
///
/// A hidden order is kept rather than filtered: it is still a listing the player holds, and one
/// for something they no longer own becomes wrong the moment they make it visible again.
pub fn list_mine(
    transport: &dyn MarketTransport,
    token: &MarketToken,
) -> Result<(Vec<MarketOrder>, MarketToken), MarketError> {
    let response = transport.send(MarketRequest {
        method: Method::Get,
        url: format!("{API_V2}/orders/my"),
        token: Some(token.expose().to_owned()),
        body: None,
    })?;
    let renewed = renewed_token(&response, token);
    match response.status {
        200..=299 if response.body.len() <= MAX_ORDERS_BYTES => {
            let parsed = serde_json::from_slice::<OrdersResponse>(&response.body)
                .map_err(|_| MarketError::Malformed)?;
            Ok((
                parsed.data.into_iter().map(MarketOrder::from).collect(),
                renewed,
            ))
        }
        200..=299 => Err(MarketError::Malformed),
        401 | 403 => Err(MarketError::Unauthorized),
        429 => Err(MarketError::RateLimited),
        _ => Err(MarketError::Unreachable),
    }
}

/// Rejects an id that could not name a single existing order: empty, or carrying a `/` that would
/// let it address something other than one order in the path it is interpolated into.
fn validate_order_id(order_id: &str) -> Result<(), MarketError> {
    if order_id.is_empty() || order_id.contains('/') {
        return Err(MarketError::Rejected);
    }
    Ok(())
}

/// Take an order down.
pub fn delete_order(
    transport: &dyn MarketTransport,
    token: &MarketToken,
    order_id: &str,
) -> Result<MarketToken, MarketError> {
    validate_order_id(order_id)?;
    let response = transport.send(MarketRequest {
        method: Method::Delete,
        url: format!("{API_V2}/order/{order_id}"),
        token: Some(token.expose().to_owned()),
        body: None,
    })?;
    write_outcome(response, token)
}

/// Lower an order to the quantity the player actually holds.
///
/// Only the quantity is sent. A patch that also carried the price would silently reprice an order
/// the player asked only to shrink, and they would find out from a buyer.
///
/// This call sends whatever `quantity` it is given -- it has no current quantity to check against,
/// so it cannot enforce that the value is a reduction. Keeping the new quantity at or below what
/// the order already holds is the caller's obligation.
pub fn set_order_quantity(
    transport: &dyn MarketTransport,
    token: &MarketToken,
    order_id: &str,
    quantity: u32,
) -> Result<MarketToken, MarketError> {
    validate_order_id(order_id)?;
    // Zero is a deletion wearing a patch's clothes. The two are different actions behind different
    // buttons, and refusing here means no caller can route round that distinction.
    if quantity == 0 {
        return Err(MarketError::Rejected);
    }
    let response = transport.send(MarketRequest {
        method: Method::Patch,
        url: format!("{API_V2}/order/{order_id}"),
        token: Some(token.expose().to_owned()),
        body: Some(format!("{{\"quantity\":{quantity}}}")),
    })?;
    write_outcome(response, token)
}

/// Publish a sell listing.
///
/// Deliberately narrow. The API's create body carries `perTrade`, `rank`, `charges`, `subtype`,
/// `amberStars` and `cyanStars`, each required for the items that support it and *forbidden* for
/// the ones that do not -- a 400 either way. Rather than model that, callers offer this only for
/// items `MarketItems::comparable` accepts, which is exactly the set whose path names one
/// collection row: no relics, no sets, nothing ranked or subtyped. The guard that keeps
/// reconciliation honest is the guard that keeps this body to four fields.
///
/// `visible` is sent explicitly because the API defaults it to `false`, and a listing nobody can
/// see is not what someone pressing "sell" asked for -- as this account found out the hard way.
pub fn create_order(
    transport: &dyn MarketTransport,
    token: &MarketToken,
    item_id: &str,
    platinum: u32,
    quantity: u32,
    visible: bool,
) -> Result<MarketToken, MarketError> {
    // The API's own bounds, checked before a request is spent finding out. An id with a quote or a
    // backslash would break out of the JSON it is interpolated into, so it is refused rather than
    // escaped: every id this takes comes from the item table, and none of them contain either.
    if item_id.is_empty() || item_id.contains(['"', '\\']) {
        return Err(MarketError::Rejected);
    }
    if !(1..=900_000).contains(&platinum) || !(1..=9_999).contains(&quantity) {
        return Err(MarketError::Rejected);
    }
    let response = transport.send(MarketRequest {
        method: Method::Post,
        url: format!("{API_V2}/order"),
        token: Some(token.expose().to_owned()),
        body: Some(format!(
            r#"{{"itemId":"{item_id}","type":"sell","platinum":{platinum},"quantity":{quantity},"visible":{visible}}}"#
        )),
    })?;
    write_outcome(response, token)
}

fn write_outcome(
    response: crate::MarketResponse,
    token: &MarketToken,
) -> Result<MarketToken, MarketError> {
    let renewed = renewed_token(&response, token);
    match response.status {
        200..=299 => Ok(renewed),
        401 | 403 => Err(MarketError::Unauthorized),
        404 => Err(MarketError::Rejected),
        429 => Err(MarketError::RateLimited),
        _ => Err(MarketError::Unreachable),
    }
}
