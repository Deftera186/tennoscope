//! Authenticated warframe.market account access.
//!
//! Separate from `warframe-acquisition::market`, which prices items anonymously, because this
//! carries a credential and fails for unrelated reasons: an unreachable API stops a price, an
//! expired token stops an account. They are diagnosed apart and reported apart.
//!
//! Every call goes through [`MarketTransport`], so the tests run offline against recorded bodies
//! rather than against the live API with somebody's real orders behind it.
#![forbid(unsafe_code)]

use thiserror::Error;

/// The version that still issues tokens to third parties. `/v2/auth/signin` is first-party only:
/// it requires a Firebase App Check header no third party can produce, and warframe.market's own
/// documentation directs integrations here until OAuth registration opens.
pub const API_V1: &str = "https://api.warframe.market/v1";
pub const API_V2: &str = "https://api.warframe.market/v2";

mod auth;
pub use auth::{MarketToken, renewed_token, sign_in, verify_token};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Method {
    Get,
    Post,
    Patch,
    Delete,
}

/// One request, with the token kept apart from the URL and body.
///
/// Separate so a transport cannot accidentally log a formatted request that carries the
/// credential, and so a test can assert on the URL without the token appearing in the failure.
#[derive(Clone)]
pub struct MarketRequest {
    pub method: Method,
    pub url: String,
    pub token: Option<String>,
    pub body: Option<String>,
}

/// Hand-written so a stray `{request:?}` -- in a log line, a panic message, an `.expect(&format!())`
/// -- prints the method and url and nothing that could be a credential. `token` is the account
/// token; `body` is, for signin, the serialized password.
impl std::fmt::Debug for MarketRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MarketRequest")
            .field("method", &self.method)
            .field("url", &self.url)
            .field("token", &self.token.as_ref().map(|_| "<redacted>"))
            .field("body", &self.body.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// The response, with the `Authorization` header kept because that is where a renewed token
/// arrives -- warframe.market reissues on use rather than only at signin.
#[derive(Clone)]
pub struct MarketResponse {
    pub status: u16,
    pub authorization: Option<String>,
    pub body: Vec<u8>,
}

/// Hand-written: `authorization` carries a renewed token, and `body` on an account route
/// describes the player's orders, so neither belongs in a rendering. The status and body length
/// are kept because they are what a diagnostic actually needs.
impl std::fmt::Debug for MarketResponse {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("MarketResponse")
            .field("status", &self.status)
            .field(
                "authorization",
                &self.authorization.as_ref().map(|_| "<redacted>"),
            )
            .field("body_len", &self.body.len())
            .finish()
    }
}

pub trait MarketTransport {
    fn send(&self, request: MarketRequest) -> Result<MarketResponse, MarketError>;
}

/// Why a market call did not produce what was asked for.
///
/// The variants are distinct because each wants a different response from the interface, and
/// collapsing them produces the failure this feature exists to avoid: an expired credential and a
/// network blip look identical, so the application either retries a token that cannot work or
/// tells the player to sign in again because their wifi dropped.
///
/// No variant carries a token, a password, or an account identifier. `Rejected` deliberately does
/// not say which field was wrong: that is a detail the API knows and this application has no use
/// for, and carrying it means carrying it into every log line that renders the error.
#[derive(Clone, Debug, Eq, Error, PartialEq)]
pub enum MarketError {
    #[error("warframe.market could not be reached")]
    Unreachable,
    #[error("warframe.market is rate limiting this client")]
    RateLimited,
    #[error("the stored warframe.market credential was refused")]
    Unauthorized,
    #[error("warframe.market rejected the sign-in")]
    Rejected,
    #[error("the warframe.market sign-in route is no longer available")]
    SigninUnavailable,
    #[error("warframe.market sent a response this client cannot read")]
    Malformed,
    #[error("no credential store is available")]
    CredentialUnavailable,
}
