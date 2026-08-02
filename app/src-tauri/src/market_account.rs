//! The linked warframe.market account, as the application holds it.
//!
//! Keeps the credential and the item table in one place so the commands stay thin, and so the
//! token is read from its store at the moment it is used rather than being carried around in the
//! runtime where every future field addition would have to be careful of it.
#![forbid(unsafe_code)]

use app_core::{MarketAccountView, reconcile_orders};
use local_store::SnapshotMeta;
use warframe_domain::Collection;
use warframe_market::{
    CredentialBacking, CredentialStore, MarketError, MarketItems, MarketToken, MarketTransport,
    list_mine,
};

pub struct MarketSession {
    store: Box<dyn CredentialStore + Send + Sync>,
    /// warframe.market's item table. 1.61 MB and one request, so it is fetched once per launch
    /// and reused: re-fetching it on every order refresh would make opening the section the most
    /// expensive thing the application does.
    items: Option<MarketItems>,
}

impl MarketSession {
    pub fn new(store: Box<dyn CredentialStore + Send + Sync>) -> Self {
        Self { store, items: None }
    }

    pub fn backing(&self) -> CredentialBacking {
        self.store.backing()
    }

    pub fn token(&self) -> Result<Option<MarketToken>, MarketError> {
        self.store.load()
    }

    /// Keep a token, including one a response renewed.
    pub fn adopt(&mut self, token: MarketToken) -> Result<(), MarketError> {
        self.store.store(&token)
    }

    pub fn forget(&mut self) -> Result<(), MarketError> {
        self.items = None;
        self.store.clear()
    }

    pub fn items(&mut self, transport: &dyn MarketTransport) -> Result<&MarketItems, MarketError> {
        if self.items.is_none() {
            self.items = Some(MarketItems::fetch(transport)?);
        }
        self.items.as_ref().ok_or(MarketError::Malformed)
    }
}

/// The account section's state, fetched and reconciled.
///
/// An unlinked session asks for nothing: a request made on behalf of an account with no token
/// spends a slot to be told 401.
pub fn account_view(
    session: &mut MarketSession,
    transport: &dyn MarketTransport,
    collection: &Collection,
    snapshot: Option<&SnapshotMeta>,
    now: &str,
) -> Result<MarketAccountView, MarketError> {
    let Some(token) = session.token()? else {
        return Ok(MarketAccountView::unlinked());
    };
    let backing = session.backing();
    let items = session.items(transport)?.clone();
    match list_mine(transport, &token) {
        Ok((orders, renewed)) => {
            // Stored before the view is built, so a renewal survives even if something later in
            // this function fails.
            session.adopt(renewed)?;
            let reconciled = reconcile_orders(&orders, &items, collection, snapshot);
            Ok(MarketAccountView::linked(
                backing,
                reconciled,
                now.to_owned(),
            ))
        }
        // A refused credential is the account's own state rather than a failed request, and the
        // interface has a repair for it.
        Err(MarketError::Unauthorized) => Ok(MarketAccountView::needs_relink()),
        Err(error) => Err(error),
    }
}

/// What the interface says about a failure.
///
/// Every one of these is shown to a player, so none names a credential value or an address. The
/// signin route being gone gets its own wording pointing at the path that still works: the route
/// is undocumented and can be withdrawn, and a player told their password failed will change a
/// password that was never the problem.
pub const fn failure_message(error: MarketError) -> &'static str {
    match error {
        MarketError::Unreachable => "warframe.market could not be reached",
        MarketError::RateLimited => "warframe.market is limiting requests; try again shortly",
        MarketError::Unauthorized => "The linked account needs signing in again",
        MarketError::Rejected => "warframe.market did not accept those sign-in details",
        MarketError::SigninUnavailable => {
            "Signing in from the app is unavailable; link with a pasted token instead"
        }
        MarketError::Malformed => "warframe.market sent a response this version cannot read",
        MarketError::CredentialUnavailable => "The credential could not be saved on this machine",
    }
}
