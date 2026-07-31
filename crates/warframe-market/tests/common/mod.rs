use std::sync::Mutex;

use warframe_market::{MarketError, MarketRequest, MarketResponse, MarketTransport};

/// Records what it was asked for and answers with what it was told to.
///
/// Every test in this crate goes through one of these. A test that reached the real API would be
/// a test of warframe.market's uptime, and would post orders from whoever ran it.
pub struct FakeTransport {
    replies: Mutex<Vec<Result<MarketResponse, MarketError>>>,
    seen: Mutex<Vec<MarketRequest>>,
}

impl FakeTransport {
    pub fn new(replies: Vec<Result<MarketResponse, MarketError>>) -> Self {
        Self {
            replies: Mutex::new(replies),
            seen: Mutex::new(Vec::new()),
        }
    }

    pub fn seen(&self) -> Vec<MarketRequest> {
        self.seen.lock().expect("fake transport lock").clone()
    }
}

impl MarketTransport for FakeTransport {
    fn send(&self, request: MarketRequest) -> Result<MarketResponse, MarketError> {
        self.seen.lock().expect("fake transport lock").push(request);
        let mut replies = self.replies.lock().expect("fake transport lock");
        if replies.is_empty() {
            return Err(MarketError::Unreachable);
        }
        replies.remove(0)
    }
}

/// A 200 carrying `body`, with no reissued token.
pub fn ok(body: &str) -> Result<MarketResponse, MarketError> {
    Ok(MarketResponse {
        status: 200,
        authorization: None,
        body: body.as_bytes().to_vec(),
    })
}

/// A 200 carrying `body` and a reissued token in the header warframe.market puts it in.
pub fn ok_with_token(body: &str, token: &str) -> Result<MarketResponse, MarketError> {
    Ok(MarketResponse {
        status: 200,
        authorization: Some(format!("JWT {token}")),
        body: body.as_bytes().to_vec(),
    })
}

/// A response carrying only a status, for the error paths.
pub fn status(code: u16) -> Result<MarketResponse, MarketError> {
    Ok(MarketResponse {
        status: code,
        authorization: None,
        body: b"{\"apiVersion\":\"0.25.0\",\"data\":null,\"error\":{}}".to_vec(),
    })
}
