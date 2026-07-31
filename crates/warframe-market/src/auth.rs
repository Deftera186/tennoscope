//! The credential lifecycle: obtaining a token, checking one, and keeping it out of everything
//! that renders.
#![forbid(unsafe_code)]

use serde::Serialize;
use zeroize::{Zeroize, Zeroizing};

use crate::{API_V1, API_V2, MarketError, MarketRequest, MarketResponse, MarketTransport, Method};

/// An account token, which is a credential: whoever holds it can post and delete orders on the
/// account. Wrapped rather than passed as a `String` so it cannot be printed by accident.
///
/// `Debug` is written by hand. A derived one would put the token in the first log line that
/// renders any struct holding it, and that edit looks harmless in review.
#[derive(Clone, Eq, PartialEq)]
pub struct MarketToken(String);

impl MarketToken {
    pub fn new(value: String) -> Self {
        Self(value)
    }

    /// The token itself, for the one caller that has to put it on a request.
    ///
    /// Named `expose` rather than `as_str` so that reaching for it reads as a decision at every
    /// call site.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.trim().is_empty()
    }
}

impl std::fmt::Debug for MarketToken {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MarketToken(redacted)")
    }
}

impl Drop for MarketToken {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// What the v1 signin route expects.
///
/// `auth_type: "header"` is what makes the route answer with the token in a response header
/// instead of setting a cookie this client would have to keep a jar for.
#[derive(Serialize)]
struct SigninBody<'a> {
    email: &'a str,
    password: &'a str,
    auth_type: &'a str,
}

/// Exchange an email and password for an account token.
///
/// The password is borrowed, used once, and never stored. The body carrying it is held in a
/// `Zeroizing` buffer so the serialized copy does not outlive the call in a freed allocation.
pub fn sign_in(
    transport: &dyn MarketTransport,
    email: &str,
    password: &str,
) -> Result<MarketToken, MarketError> {
    let body = Zeroizing::new(
        serde_json::to_string(&SigninBody {
            email,
            password,
            auth_type: "header",
        })
        .map_err(|_| MarketError::Malformed)?,
    );
    let response = transport.send(MarketRequest {
        method: Method::Post,
        url: format!("{API_V1}/auth/signin"),
        // The seed value the route requires. Not a credential: it names the scheme the client
        // wants its token issued under.
        token: Some(String::new()),
        body: Some(body.to_string()),
    })?;
    match response.status {
        200..=299 => token_from(&response).ok_or(MarketError::Malformed),
        // The route answering 404 or 410 means the route is gone, which the interface must not
        // present as a wrong password: the paste-token path still works, and a player told their
        // password failed will go and change a password that was never the problem.
        404 | 410 => Err(MarketError::SigninUnavailable),
        429 => Err(MarketError::RateLimited),
        401 | 403 | 400 => Err(MarketError::Rejected),
        _ => Err(MarketError::Unreachable),
    }
}

/// Check a token against the account route, and take whatever token comes back.
///
/// Used for a pasted token before it is stored, so a bad paste fails at the paste box.
pub fn verify_token(
    transport: &dyn MarketTransport,
    token: &MarketToken,
) -> Result<MarketToken, MarketError> {
    if token.is_empty() {
        return Err(MarketError::Rejected);
    }
    let response = transport.send(MarketRequest {
        method: Method::Get,
        url: format!("{API_V2}/me"),
        token: Some(token.expose().to_owned()),
        body: None,
    })?;
    match response.status {
        200..=299 => Ok(renewed_token(&response, token)),
        401 | 403 => Err(MarketError::Unauthorized),
        429 => Err(MarketError::RateLimited),
        _ => Err(MarketError::Unreachable),
    }
}

/// The token a response reissued, or the one that was sent if it reissued none.
///
/// warframe.market renews on use rather than only at signin, so an account used regularly never
/// reaches the roughly sixty-day expiry. Missing the renewal would expire an account that was in
/// daily use, which reads as the feature randomly logging itself out.
pub fn renewed_token(response: &MarketResponse, current: &MarketToken) -> MarketToken {
    token_from(response).unwrap_or_else(|| current.clone())
}

/// The token out of an `Authorization` header, whichever scheme labels it.
///
/// v1 issues `JWT <token>` and v2 issues `Bearer <token>`; one client sees both, because the
/// token from v1 signin is the one sent to v2 routes.
fn token_from(response: &MarketResponse) -> Option<MarketToken> {
    let header = response.authorization.as_deref()?;
    let value = header
        .strip_prefix("JWT ")
        .or_else(|| header.strip_prefix("Bearer "))
        .unwrap_or(header)
        .trim();
    (!value.is_empty()).then(|| MarketToken::new(value.to_owned()))
}
