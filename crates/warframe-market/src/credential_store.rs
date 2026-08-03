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
    /// Probed by reading, not merely by constructing an entry. Constructing one succeeds against a
    /// keyring that cannot actually answer -- a locked collection is the ordinary case, since a
    /// session started without the login keyring unlocked has one. That store then reports itself
    /// as the backing, every `load` fails, and the screen says the account is unlinked while
    /// insisting the credential is held in the keyring. The token is not gone; it is unreachable,
    /// which is worse, because nothing on screen says so and re-linking writes to the same place.
    ///
    /// A read that comes back `NoEntry` is a working keyring with nothing in it yet, which is what
    /// a first launch looks like. Anything else means this application cannot rely on it, and the
    /// database fallback exists precisely for that.
    pub fn available() -> Option<Self> {
        let entry = keyring::Entry::new(KEYRING_SERVICE, KEYRING_ENTRY).ok()?;
        match entry.get_password() {
            Ok(_) | Err(keyring::Error::NoEntry) => Some(Self { entry }),
            Err(_) => None,
        }
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

use std::path::PathBuf;

use local_store::SqliteStore;

/// The fallback: the token in the application's own database, which is file-permissioned but
/// readable by anything running as this user.
///
/// Holds a path rather than a connection. `AppCore` owns its `SqliteStore` privately and does not
/// lend it out, and threading an `Arc<Mutex<_>>` through it to reach one row would restructure who
/// owns the database for the sake of a value read at startup and written when a token renews.
/// Opening per operation costs a file open on a path already in the page cache.
#[derive(Clone)]
pub struct DatabaseStore {
    path: PathBuf,
}

impl DatabaseStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    fn open(&self) -> Result<SqliteStore, MarketError> {
        SqliteStore::open(&self.path).map_err(|_| MarketError::CredentialUnavailable)
    }
}

impl CredentialStore for DatabaseStore {
    fn load(&self) -> Result<Option<MarketToken>, MarketError> {
        self.open()?
            .market_credential()
            .map(|held| held.map(MarketToken::new))
            .map_err(|_| MarketError::CredentialUnavailable)
    }

    fn store(&self, token: &MarketToken) -> Result<(), MarketError> {
        self.open()?
            .set_market_credential(token.expose())
            .map_err(|_| MarketError::CredentialUnavailable)
    }

    fn clear(&self) -> Result<(), MarketError> {
        self.open()?
            .clear_market_credential()
            .map_err(|_| MarketError::CredentialUnavailable)
    }

    fn backing(&self) -> CredentialBacking {
        CredentialBacking::Database
    }
}

/// The best credential store this machine offers, with the other one still readable.
///
/// The keyring is preferred and the database is the fallback rather than the other way round,
/// because a credential in a keyring survives a stolen backup and one in a database file does not.
/// This never fails: a machine with no keyring still links, which is the reason the fallback
/// exists at all.
pub fn open_credential_store(database: PathBuf) -> Box<dyn CredentialStore + Send + Sync> {
    let database = DatabaseStore::new(database);
    KeyringStore::available().map_or_else(
        || Box::new(database.clone()) as Box<dyn CredentialStore + Send + Sync>,
        |keyring| {
            Box::new(FallbackReadStore {
                primary: keyring,
                secondary: database.clone(),
            }) as Box<dyn CredentialStore + Send + Sync>
        },
    )
}

/// The keyring, plus whatever an earlier launch left in the database.
///
/// Which store this application picks is decided at startup, and the answer can differ between
/// launches: a session whose keyring daemon is not up yet links to the database, and the next
/// launch finds a working keyring and reads an empty one. Neither store lost anything, but the
/// account appears unlinked on every other start, which is indistinguishable from the credential
/// deleting itself.
///
/// So a miss on the keyring falls through to the database before concluding there is no account,
/// and anything found there is promoted -- read once from the weaker store, written to the
/// stronger, and cleared from the weaker so the credential is not left in two places.
struct FallbackReadStore {
    primary: KeyringStore,
    secondary: DatabaseStore,
}

impl CredentialStore for FallbackReadStore {
    fn load(&self) -> Result<Option<MarketToken>, MarketError> {
        // A keyring that errors rather than reporting an empty entry is one this launch cannot
        // trust, and the database may still hold the token from a launch that could not either.
        if let Ok(Some(token)) = self.primary.load() {
            return Ok(Some(token));
        }
        let Some(token) = self.secondary.load()? else {
            return Ok(None);
        };
        // Best effort: failing to promote is not a reason to refuse a credential that was found.
        // The next launch simply finds it in the same place and tries again.
        if self.primary.store(&token).is_ok() {
            let _ = self.secondary.clear();
        }
        Ok(Some(token))
    }

    fn store(&self, token: &MarketToken) -> Result<(), MarketError> {
        self.primary.store(token)
    }

    /// Both, so unlinking cannot leave a copy behind in the store this launch is not using.
    fn clear(&self) -> Result<(), MarketError> {
        let primary = self.primary.clear();
        let secondary = self.secondary.clear();
        primary.and(secondary)
    }

    fn backing(&self) -> CredentialBacking {
        CredentialBacking::Keyring
    }
}
