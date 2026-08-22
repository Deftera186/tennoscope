//! The linked warframe.market account, as the application holds it.
//!
//! Keeps the credential and the item table in one place so the commands stay thin, and so the
//! token is read from its store at the moment it is used rather than being carried around in the
//! runtime where every future field addition would have to be careful of it.
#![forbid(unsafe_code)]

use std::sync::Arc;

use app_core::{MarketAccountView, OrderStatus, ReconciledOrder, reconcile_orders};
use local_store::SnapshotMeta;
use warframe_domain::Collection;
use warframe_market::{
    CredentialBacking, CredentialStore, Listing, MarketError, MarketItems, MarketToken,
    MarketTransport, list_mine,
};

pub struct MarketSession {
    store: Box<dyn CredentialStore + Send + Sync>,
    /// warframe.market's item table. 1.61 MB and one request, so it is fetched once per launch
    /// and reused: re-fetching it on every order refresh would make opening the section the most
    /// expensive thing the application does. Held behind an `Arc` so a caller can take its own
    /// handle and let go of `&mut MarketSession` before doing anything slow with it, rather than
    /// cloning the whole table out on every refresh.
    items: Option<Arc<MarketItems>>,
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
        self.store
            .store(&token)
            .inspect(|_| log::info!("market: sign in ok"))
            .inspect_err(|error| log::warn!("market: sign in failed: {error}"))
    }

    pub fn forget(&mut self) -> Result<(), MarketError> {
        self.items = None;
        self.store
            .clear()
            .inspect(|_| log::info!("market: sign out ok"))
            .inspect_err(|error| log::warn!("market: sign out failed: {error}"))
    }

    /// The item table already held, if a fetch has happened since launch.
    ///
    /// For a caller that wants to fetch unlocked: it can take this handle, decide for itself
    /// whether a fetch is needed, and only come back to store the result -- rather than holding
    /// whatever lock guards this session for the whole network round trip.
    pub fn cached_items(&self) -> Option<Arc<MarketItems>> {
        self.items.clone()
    }

    /// Keep an item table a caller fetched on its own.
    pub fn set_items(&mut self, items: Arc<MarketItems>) {
        self.items = Some(items);
    }

    pub fn items(
        &mut self,
        transport: &dyn MarketTransport,
    ) -> Result<Arc<MarketItems>, MarketError> {
        if self.items.is_none() {
            self.items = Some(Arc::new(MarketItems::fetch(transport)?));
        }
        // The `None` branch just above either returned on failure or filled this in, so the field
        // is always occupied here. Reporting `Malformed` on a miss would blame warframe.market for
        // a local bookkeeping mistake it had no part in.
        Ok(Arc::clone(
            self.items.as_ref().expect("populated immediately above"),
        ))
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
    let items = session.items(transport)?;
    match list_mine(transport, &token) {
        Ok((orders, renewed)) => {
            // Stored before the view is built, so a renewal survives even if something later in
            // this function fails.
            session.adopt(renewed)?;
            let reconciled = reconcile_orders(&orders, &items, collection, snapshot);
            Ok(
                MarketAccountView::linked(backing, reconciled, now.to_owned())
                    .with_listable(&items, collection),
            )
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

/// The order named is not one the held view currently lists.
///
/// Shared by both writing commands: a stale or fabricated id must be refused before it reaches
/// the transport, since a delete or a quantity write acts irreversibly on a real account.
pub const ORDER_NOT_HELD: &str = "That order is not on the currently held list";

/// A quantity write was asked for on an order the reconciliation has not flagged as an overshoot.
///
/// The only quantity this command will ever send is the collection's own count, taken from
/// `OrderStatus::Overshoot { owned }`. An order that is not an overshoot has no such count to send.
pub const ORDER_NOT_OVERSHOOT: &str = "That order is not currently flagged as oversold";

/// Find one order by id in the account view currently held, so a write can be checked against
/// what the player is actually looking at rather than against whatever a frontend call supplies.
pub fn find_order<'a>(view: &'a MarketAccountView, order_id: &str) -> Option<&'a ReconciledOrder> {
    view.orders.iter().find(|entry| entry.order.id == order_id)
}

/// The quantity a write is allowed to send for this order: the collection's own count, and only
/// when the reconciliation has flagged the order as overselling it. Anything else -- `Ok`,
/// `Missing`, `Unverifiable`, or an id absent from the view -- has no quantity this command may
/// derive, and the caller must refuse rather than fall back to a frontend-supplied number.
pub fn overshoot_quantity(view: &MarketAccountView, order_id: &str) -> Option<u32> {
    match find_order(view, order_id)?.status {
        OrderStatus::Overshoot { owned } => Some(owned),
        _ => None,
    }
}

/// A sell was asked for on something this device does not hold.
pub const ITEM_NOT_OWNED: &str = "This device's collection does not hold that item";

/// A sell was asked for on a row whose listing this application cannot name honestly.
///
/// A copy held part-way up its ranks -- warframe.market quotes a card at rank 0 and at its ceiling
/// only -- an Ayatan sculpture whose socketed stars no row knows, a set whose market entry names
/// the built item rather than the parts actually held. The refusal is backend-side because the
/// request would be refused by warframe.market anyway, after the request.
pub const ITEM_NOT_LISTABLE: &str = "That item cannot be listed from TennoScope: warframe.market needs details this app does not ask for";

/// The listing a sell may publish, or the reason it is refused.
///
/// Takes a collection row id rather than a market id for the same reason `set_order_quantity`
/// derives its own quantity: a market id supplied by the frontend is a value nothing checked, and
/// this one addresses which item a real listing gets published for. The row is checked against the
/// collection first -- offering to sell what the player does not have is the mirror of the
/// `missing` flag this screen exists to raise -- and then resolved through the item table, which
/// answers with the rank, subtype and per-trade size the row's own identity implies. Every
/// contextual field of the create body comes from here; none is ever taken from the caller.
pub fn authorize_sell<'a>(
    items: &'a MarketItems,
    collection: &Collection,
    collection_id: &str,
) -> Result<Listing<'a>, &'static str> {
    let held = collection
        .entries()
        .find(|entry| entry.item.id.as_str() == collection_id)
        .ok_or(ITEM_NOT_OWNED)?;
    items
        .listing_for(collection_id, held.at_max_rank().unwrap_or(false))
        .ok_or(ITEM_NOT_LISTABLE)
}

/// Whether a delete may proceed: only for an id the held view actually lists. Checked before any
/// transport is built, since a stale or fabricated id must never reach a delete call.
pub fn authorize_removal(view: &MarketAccountView, order_id: &str) -> Result<(), &'static str> {
    find_order(view, order_id).map(|_| ()).ok_or(ORDER_NOT_HELD)
}

/// The quantity a quantity write may send, or the reason it is refused. Checked before any
/// transport is built: the only value ever sent is the collection's own count on an order
/// currently flagged as an overshoot, never a number a caller supplied.
pub fn authorize_quantity_write(
    view: &MarketAccountView,
    order_id: &str,
) -> Result<u32, &'static str> {
    overshoot_quantity(view, order_id).ok_or(ORDER_NOT_OVERSHOOT)
}
