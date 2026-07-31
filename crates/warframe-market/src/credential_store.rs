//! Where the account token lives between launches.
//!
//! Two backends, because neither alone covers the machines this runs on. The OS keyring is the
//! right place for a credential, and a Linux session without a running secret service is ordinary
//! rather than exceptional -- so a keyring-only design would simply not work on a large share of
//! the target platform, and a database-only one would be weaker than necessary everywhere else.
#![forbid(unsafe_code)]

use serde::Serialize;

use crate::{MarketError, MarketToken};

/// The keyring service name. Matches the application identifier already used for the local data
/// directory, so a player looking through their keyring finds it under the name they installed.
pub const KEYRING_SERVICE: &str = "io.github.deftera186.tennoscope";
/// The entry name. Not the player's email: the keyring is a place a credential can be read from,
/// and putting an address in the key would store an identifier that nothing needs.
pub const KEYRING_ENTRY: &str = "warframe-market-token";

/// Which backend holds the credential.
///
/// Observable because the health panel says so. The difference is real rather than cosmetic: a
/// database file is readable by anything running as the user and is swept up by backup tools, and
/// a player who would rather not have that can install a secret service or decline to link.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialBacking {
    Keyring,
    Database,
}

pub trait CredentialStore {
    fn load(&self) -> Result<Option<MarketToken>, MarketError>;
    fn store(&self, token: &MarketToken) -> Result<(), MarketError>;
    fn clear(&self) -> Result<(), MarketError>;
    fn backing(&self) -> CredentialBacking;
}

/// The OS keyring: Secret Service on Linux, Credential Manager on Windows.
pub struct KeyringStore {
    entry: keyring::Entry,
}

impl KeyringStore {
    /// A keyring store, if this machine has a working keyring.
    ///
    /// Probed by constructing an entry rather than by asking the session what it supports: the
    /// answer that matters is whether this application can store a credential, and the ways that
    /// fails (no service, a locked collection, a sandbox) are not enumerable from here.
    pub fn available() -> Option<Self> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_ENTRY)
            .ok()
            .map(|entry| Self { entry })
    }
}

impl CredentialStore for KeyringStore {
    fn load(&self) -> Result<Option<MarketToken>, MarketError> {
        match self.entry.get_password() {
            Ok(value) => Ok(Some(MarketToken::new(value))),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(MarketError::CredentialUnavailable),
        }
    }

    fn store(&self, token: &MarketToken) -> Result<(), MarketError> {
        self.entry
            .set_password(token.expose())
            .map_err(|_| MarketError::CredentialUnavailable)
    }

    fn clear(&self) -> Result<(), MarketError> {
        match self.entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(MarketError::CredentialUnavailable),
        }
    }

    fn backing(&self) -> CredentialBacking {
        CredentialBacking::Keyring
    }
}
