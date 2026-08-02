//! The one file in this crate that reaches the network.
//!
//! Deliberately thin. Everything worth testing -- what a signin body looks like, which status
//! means an expired credential, how an order parses -- lives in a module that takes a transport,
//! so the tests never depend on warframe.market being up or on the orders behind a real account.

use std::{io::Read, time::Duration};

use reqwest::{
    blocking::{Client, RequestBuilder},
    redirect::Policy,
};
use warframe_acquisition::{MARKET_MIN_GAP, RequestPacer};

use crate::{MarketError, MarketRequest, MarketResponse, MarketTransport, Method};

pub const USER_AGENT: &str = concat!(
    "TennoScope/",
    env!("CARGO_PKG_VERSION"),
    " (+https://github.com/Deftera186/tennoscope)"
);

const CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Longer than the price path's eight seconds because the item table is 1.61 MB against a few
/// kilobytes, and a slow connection reading it is not a failure.
const TOTAL_TIMEOUT: Duration = Duration::from_secs(30);
/// The cap on any response this transport will read.
const MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

pub struct MarketHttp {
    client: Client,
    pacer: RequestPacer,
}

impl MarketHttp {
    /// A transport pacing against `pacer`.
    ///
    /// The pacer is passed in rather than created here because the anonymous price path is the
    /// other caller against the same three-per-second budget, and two clocks would jointly spend
    /// six.
    pub fn new(pacer: RequestPacer) -> Result<Self, MarketError> {
        let client = Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(TOTAL_TIMEOUT)
            .user_agent(USER_AGENT)
            // Every URL this transport calls is a known API endpoint under a fixed origin, so a
            // redirect is a signal something is wrong rather than something to follow.
            .redirect(Policy::none())
            .build()
            .map_err(|_| MarketError::Unreachable)?;
        Ok(Self { client, pacer })
    }
}

impl MarketTransport for MarketHttp {
    fn send(&self, request: MarketRequest) -> Result<MarketResponse, MarketError> {
        self.pacer.take_slot(MARKET_MIN_GAP);
        let builder = match request.method {
            Method::Get => self.client.get(&request.url),
            Method::Post => self.client.post(&request.url),
            Method::Patch => self.client.patch(&request.url),
            Method::Delete => self.client.delete(&request.url),
        };
        let builder = apply_token(builder, request.token.as_deref(), &request.url);
        let builder = match request.body {
            Some(body) => builder
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body),
            None => builder,
        };
        let response = builder.send().map_err(|_| MarketError::Unreachable)?;
        let status = response.status().as_u16();
        let authorization = response
            .headers()
            .get(reqwest::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        let mut body = Vec::new();
        response
            .take((MAX_RESPONSE_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|_| MarketError::Unreachable)?;
        if body.len() > MAX_RESPONSE_BYTES {
            return Err(MarketError::Malformed);
        }
        Ok(MarketResponse {
            status,
            authorization,
            body,
        })
    }
}

/// Label the credential with the scheme the route being called expects.
///
/// One token authenticates both versions, but under different names: v1 issued it and calls it
/// `JWT`, and v2 wants `Bearer`. Sending the wrong label is a 401 that looks exactly like an
/// expired credential, which would send the player to re-link an account that was fine.
fn apply_token(builder: RequestBuilder, token: Option<&str>, url: &str) -> RequestBuilder {
    let Some(token) = token else {
        return builder;
    };
    let scheme = if url.contains("/v1/") { "JWT" } else { "Bearer" };
    builder.header(
        reqwest::header::AUTHORIZATION,
        format!("{scheme} {token}").trim().to_owned(),
    )
}
