//! The market token must never appear in the log channel, no matter what path
//! carries it.

use std::io::Read;

use app_lib::market_account;
use warframe_market::{CredentialBacking, CredentialStore, MarketError, MarketToken};

struct MemoryStore {
    token: std::sync::Mutex<Option<String>>,
}

impl CredentialStore for MemoryStore {
    fn load(&self) -> Result<Option<MarketToken>, MarketError> {
        Ok(self.token.lock().expect("lock").clone().map(MarketToken::new))
    }
    fn store(&self, token: &MarketToken) -> Result<(), MarketError> {
        *self.token.lock().expect("lock") = Some(token.expose().to_owned());
        Ok(())
    }
    fn clear(&self) -> Result<(), MarketError> {
        *self.token.lock().expect("lock") = None;
        Ok(())
    }
    fn backing(&self) -> CredentialBacking {
        CredentialBacking::Database
    }
}

mod common;

#[test]
fn token_never_reaches_the_log() {
    common::isolate_debug_log();
    let store = MemoryStore {
        token: std::sync::Mutex::new(None),
    };
    let mut session = market_account::MarketSession::new(Box::new(store));
    let secret = "a1b2c3d4-fake-market-token-0001";
    session
        .adopt(MarketToken::new(secret.to_owned()))
        .expect("token adopts");
    session.forget().expect("forget clears");

    let path = std::env::var_os("TENNOSCOPE_DEBUG_LOG")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("tennoscope-test.log"));
    let mut log_text = String::new();
    std::fs::File::open(&path)
        .expect("test log exists")
        .read_to_string(&mut log_text)
        .expect("test log reads");
    assert!(
        !log_text.contains(secret),
        "the market token must never appear in the log channel"
    );
}
