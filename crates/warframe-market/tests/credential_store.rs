use std::sync::Mutex;

use warframe_market::{CredentialBacking, CredentialStore, MarketError, MarketToken};

const FAKE_TOKEN: &str = "eyJhbGciOiJub25lIn0.eyJzdWIiOiJ0ZXN0In0.";

/// A store that holds one token in memory, standing in for whichever backend won.
///
/// The point of the trait is that the caller does not care which one answered, and this proves the
/// caller can be written against it.
#[derive(Default)]
struct MemoryStore {
    held: Mutex<Option<String>>,
    backing: Option<CredentialBacking>,
}

impl CredentialStore for MemoryStore {
    fn load(&self) -> Result<Option<MarketToken>, MarketError> {
        Ok(self
            .held
            .lock()
            .map_err(|_| MarketError::CredentialUnavailable)?
            .clone()
            .map(MarketToken::new))
    }

    fn store(&self, token: &MarketToken) -> Result<(), MarketError> {
        *self
            .held
            .lock()
            .map_err(|_| MarketError::CredentialUnavailable)? = Some(token.expose().to_owned());
        Ok(())
    }

    fn clear(&self) -> Result<(), MarketError> {
        *self
            .held
            .lock()
            .map_err(|_| MarketError::CredentialUnavailable)? = None;
        Ok(())
    }

    fn backing(&self) -> CredentialBacking {
        self.backing.unwrap_or(CredentialBacking::Database)
    }
}

#[test]
fn a_stored_token_comes_back() {
    let store = MemoryStore::default();

    store
        .store(&MarketToken::new(FAKE_TOKEN.to_owned()))
        .expect("store succeeds");

    let loaded = store.load().expect("load succeeds").expect("a token is held");
    assert_eq!(loaded.expose(), FAKE_TOKEN);
}

#[test]
fn an_empty_store_holds_nothing() {
    assert!(MemoryStore::default().load().expect("load succeeds").is_none());
}

/// Unlinking must actually remove the credential, not merely stop using it. A token left behind
/// after the player unlinked is a credential they believe they revoked.
#[test]
fn clearing_removes_the_token() {
    let store = MemoryStore::default();
    store
        .store(&MarketToken::new(FAKE_TOKEN.to_owned()))
        .expect("store succeeds");

    store.clear().expect("clear succeeds");

    assert!(store.load().expect("load succeeds").is_none());
}

/// Which backend answered is observable, because the health panel states it and the two are not
/// equally strong: a database file is readable by anything running as the user.
#[test]
fn the_backing_is_reported() {
    let store = MemoryStore {
        backing: Some(CredentialBacking::Keyring),
        ..MemoryStore::default()
    };

    assert_eq!(store.backing(), CredentialBacking::Keyring);
}

/// The keyring is probed rather than assumed. A Linux session with no secret service running is
/// ordinary -- minimal window managers frequently have none -- and the fallback is what makes the
/// feature work there at all.
///
/// Which answer comes back depends on the machine, so neither is asserted. What is asserted is
/// that probing is cheap and repeatable: it must not block startup waiting on a dbus timeout, and
/// it must not answer differently between two calls a moment apart, because the credential store
/// is chosen once from this answer and a store that changed identity mid-session would look for a
/// token where it never put one.
#[test]
fn probing_the_keyring_is_cheap_and_repeatable() {
    let started = std::time::Instant::now();
    let first = warframe_market::KeyringStore::available().is_some();
    let elapsed = started.elapsed();
    let second = warframe_market::KeyringStore::available().is_some();

    assert_eq!(
        first, second,
        "the keyring probe must not change its answer between calls"
    );
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "the keyring probe blocked startup for {elapsed:?}"
    );
}
