use std::sync::Mutex;

use app_core::{LinkState, MarketAccountView, OrderStatus, ReconciledOrder};
use app_lib::market_account::{
    ITEM_NOT_LISTABLE, ITEM_NOT_OWNED, MarketSession, account_view, authorize_quantity_write,
    authorize_removal, authorize_sell, failure_message,
};
use warframe_domain::{
    CatalogItem, Category, Collection, InventoryEntry, InventorySnapshot, ItemId,
};
use warframe_market::{
    CredentialBacking, CredentialStore, MarketError, MarketItems, MarketOrder, MarketRequest,
    MarketResponse, MarketToken, MarketTransport, OrderKind,
};

/// A credential store that holds one token, so the session can be exercised without a keyring.
#[derive(Default)]
struct MemoryStore {
    held: Mutex<Option<String>>,
}

impl CredentialStore for MemoryStore {
    fn load(&self) -> Result<Option<MarketToken>, MarketError> {
        Ok(self
            .held
            .lock()
            .expect("lock")
            .clone()
            .map(MarketToken::new))
    }
    fn store(&self, token: &MarketToken) -> Result<(), MarketError> {
        *self.held.lock().expect("lock") = Some(token.expose().to_owned());
        Ok(())
    }
    fn clear(&self) -> Result<(), MarketError> {
        *self.held.lock().expect("lock") = None;
        Ok(())
    }
    fn backing(&self) -> CredentialBacking {
        CredentialBacking::Database
    }
}

struct ScriptedTransport {
    replies: Mutex<Vec<Result<MarketResponse, MarketError>>>,
}

impl ScriptedTransport {
    fn new(replies: Vec<Result<MarketResponse, MarketError>>) -> Self {
        Self {
            replies: Mutex::new(replies),
        }
    }
}

impl MarketTransport for ScriptedTransport {
    fn send(&self, _request: MarketRequest) -> Result<MarketResponse, MarketError> {
        let mut replies = self.replies.lock().expect("lock");
        if replies.is_empty() {
            return Err(MarketError::Unreachable);
        }
        replies.remove(0)
    }
}

fn ok(body: &str) -> Result<MarketResponse, MarketError> {
    Ok(MarketResponse {
        status: 200,
        authorization: None,
        body: body.as_bytes().to_vec(),
    })
}

const ITEMS: &str = r#"{"apiVersion":"0.25.0","data":[
    {"id":"item-one","gameRef":"/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint",
     "i18n":{"en":{"name":"Braton Prime Blueprint"}}}
],"error":null}"#;

const ORDERS: &str = r#"{"apiVersion":"0.25.0","data":[
    {"id":"order-one","itemId":"item-one","type":"sell","platinum":12,"quantity":1,
     "perTrade":1,"visible":true,"updatedAt":"2026-07-30T10:00:00Z"}
],"error":null}"#;

/// With no stored credential the view is unlinked, and nothing is requested. A session that asked
/// the API about an account it has no token for would spend a request to be told 401.
#[test]
fn an_unlinked_session_asks_for_nothing() {
    let mut session = MarketSession::new(Box::new(MemoryStore::default()));
    let transport = ScriptedTransport::new(Vec::new());

    let view = account_view(
        &mut session,
        &transport,
        &Collection::default(),
        None,
        "2026-07-31T12:00:00Z",
    )
    .expect("view builds");

    assert_eq!(view.link, LinkState::Unlinked);
    assert!(view.orders.is_empty());
}

#[test]
fn a_linked_session_lists_and_reconciles() {
    let store = MemoryStore::default();
    store
        .store(&MarketToken::new("fake-token".to_owned()))
        .expect("token stores");
    let mut session = MarketSession::new(Box::new(store));
    let transport = ScriptedTransport::new(vec![ok(ITEMS), ok(ORDERS)]);

    let view = account_view(
        &mut session,
        &transport,
        &Collection::default(),
        None,
        "2026-07-31T12:00:00Z",
    )
    .expect("view builds");

    assert_eq!(view.link, LinkState::Linked);
    assert_eq!(view.orders.len(), 1);
    // No snapshot, so nothing is claimed.
    assert_eq!(view.orders[0].status, OrderStatus::Unverifiable);
    assert_eq!(view.listed_platinum, 12);
}

/// The item table is fetched once and reused. It is 1.61 MB, and re-fetching it on every refresh
/// would make opening the section the most expensive thing the application does.
#[test]
fn the_item_table_is_fetched_once() {
    let store = MemoryStore::default();
    store
        .store(&MarketToken::new("fake-token".to_owned()))
        .expect("token stores");
    let mut session = MarketSession::new(Box::new(store));
    // Two refreshes, but only three replies: items once, then orders twice.
    let transport = ScriptedTransport::new(vec![ok(ITEMS), ok(ORDERS), ok(ORDERS)]);

    for _ in 0..2 {
        account_view(
            &mut session,
            &transport,
            &Collection::default(),
            None,
            "2026-07-31T12:00:00Z",
        )
        .expect("view builds");
    }
}

/// A refused credential produces a relink rather than an error the caller has to interpret.
#[test]
fn a_refused_credential_becomes_a_relink() {
    let store = MemoryStore::default();
    store
        .store(&MarketToken::new("fake-token".to_owned()))
        .expect("token stores");
    let mut session = MarketSession::new(Box::new(store));
    let transport = ScriptedTransport::new(vec![
        ok(ITEMS),
        Ok(MarketResponse {
            status: 401,
            authorization: None,
            body: Vec::new(),
        }),
    ]);

    let view = account_view(
        &mut session,
        &transport,
        &Collection::default(),
        None,
        "2026-07-31T12:00:00Z",
    )
    .expect("view builds");

    assert_eq!(view.link, LinkState::NeedsRelink);
}

/// Every message the interface can show is checked for the two things that must never be in one.
///
/// The check is for a credential *value* rather than for the word "token": one message names the
/// paste-token path on purpose, because that is the path that still works when signin does not.
#[test]
fn no_failure_message_carries_a_credential_or_an_address() {
    for error in [
        MarketError::Unreachable,
        MarketError::RateLimited,
        MarketError::Unauthorized,
        MarketError::Rejected,
        MarketError::SigninUnavailable,
        MarketError::Malformed,
        MarketError::CredentialUnavailable,
    ] {
        let message = failure_message(error);
        assert!(!message.contains('@'), "address-shaped message: {message}");
        assert!(
            !message.contains("eyJ"),
            "a token value in a message: {message}"
        );
        assert!(!message.trim().is_empty(), "blank message for {error:?}");
    }
}

/// The signin route going away must not read as a wrong password: it points at the path that
/// still works.
#[test]
fn a_missing_signin_route_points_at_the_paste_path() {
    let message = failure_message(MarketError::SigninUnavailable);

    assert!(message.contains("token"), "unhelpful message: {message}");
}

fn order(id: &str, status: OrderStatus) -> ReconciledOrder {
    ReconciledOrder {
        order: MarketOrder {
            id: id.to_owned(),
            item_id: "item-one".to_owned(),
            kind: OrderKind::Sell,
            platinum: 10,
            quantity: 5,
            per_trade: 1,
            rank: None,
            subtype: None,
            visible: true,
            updated_at: None,
        },
        name: Some("Braton Prime Blueprint".to_owned()),
        status,
    }
}

/// A quantity write is refused, without ever reaching the transport, for an order the
/// reconciliation has not flagged as an overshoot -- the caller supplying a lower or higher number
/// than warframe.market currently lists is not this command's business to arbitrate; only an
/// overshoot has a quantity this command may derive and send.
#[test]
fn a_quantity_write_is_refused_for_an_order_that_is_not_an_overshoot() {
    let view = MarketAccountView::linked(
        CredentialBacking::Database,
        vec![order("order-one", OrderStatus::Ok)],
        "2026-07-31T12:00:00Z".to_owned(),
    );

    let result = authorize_quantity_write(&view, "order-one");

    assert!(result.is_err(), "an Ok order has no overshoot quantity");
}

/// The only quantity ever authorized is the collection's own count from `Overshoot { owned }` --
/// never a number the caller supplies. A caller asking to raise a two-unit overshoot to nine
/// still only gets back the two the collection holds.
#[test]
fn a_quantity_write_derives_the_owned_count_from_the_overshoot() {
    let view = MarketAccountView::linked(
        CredentialBacking::Database,
        vec![order("order-one", OrderStatus::Overshoot { owned: 2 })],
        "2026-07-31T12:00:00Z".to_owned(),
    );

    let quantity = authorize_quantity_write(&view, "order-one").expect("overshoot authorizes");

    assert_eq!(
        quantity, 2,
        "the caller's own desired value never enters this path"
    );
}

/// An id absent from the currently held view is refused for a quantity write, the same as an id
/// present but not overshooting.
#[test]
fn a_quantity_write_is_refused_for_an_id_not_on_the_held_list() {
    let view = MarketAccountView::linked(
        CredentialBacking::Database,
        vec![order("order-one", OrderStatus::Overshoot { owned: 2 })],
        "2026-07-31T12:00:00Z".to_owned(),
    );

    let result = authorize_quantity_write(&view, "order-two");

    assert!(result.is_err(), "an unknown id has nothing to authorize");
}

/// A delete is refused for an id absent from the account view currently held. The check happens
/// before any transport is built, so a stale or fabricated id never reaches a real delete call.
#[test]
fn a_removal_is_refused_for_an_id_not_on_the_held_list() {
    let view = MarketAccountView::linked(
        CredentialBacking::Database,
        vec![order("order-one", OrderStatus::Ok)],
        "2026-07-31T12:00:00Z".to_owned(),
    );

    let result = authorize_removal(&view, "order-missing");

    assert!(result.is_err(), "an id not on the list must be refused");
}

/// A delete for an id that is on the list is authorized, so a legitimate removal is not blocked by
/// the same guard that protects against a stale one.
#[test]
fn a_removal_is_authorized_for_an_id_on_the_held_list() {
    let view = MarketAccountView::linked(
        CredentialBacking::Database,
        vec![order("order-one", OrderStatus::Ok)],
        "2026-07-31T12:00:00Z".to_owned(),
    );

    let result = authorize_removal(&view, "order-one");

    assert!(result.is_ok(), "a held id must be authorized");
}

/// The shape of `/v2/items`, in the two cases selling cares about: an ordinary part whose path
/// names one collection row, and a relic whose one entry stands for four refinements.
const SELLABLE_ITEMS: &str = r#"{"apiVersion":"0.25.0","data":[
    {"id":"54a73e65e779893a797fff33","slug":"braton_prime_blueprint",
     "gameRef":"/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint",
     "i18n":{"en":{"name":"Braton Prime Blueprint"}}},
    {"id":"relic-id","slug":"lith_a1_relic",
     "gameRef":"/Lotus/Types/Game/Projections/T1VoidProjectionBratonPrimeDBronze",
     "subtypes":["intact","exceptional","flawless","radiant"],
     "i18n":{"en":{"name":"Lith A1 Relic"}}}
],"error":null}"#;

fn collection_holding(paths: &[&str]) -> Collection {
    let entries = paths
        .iter()
        .map(|path| {
            let id = ItemId::new(*path).expect("item id");
            let item = CatalogItem::new(id, "Item", Category::PrimePart).expect("catalog item");
            InventoryEntry::new(item, 3)
        })
        .collect();
    let mut collection = Collection::default();
    collection.replace(InventorySnapshot::coherent(entries).expect("snapshot"));
    collection
}

#[test]
fn a_sell_resolves_the_collections_path_to_the_market_id_it_will_be_listed_against() {
    let items = MarketItems::from_response(SELLABLE_ITEMS.as_bytes()).expect("items parse");
    let collection = collection_holding(&["/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint"]);

    let item_id = authorize_sell(
        &items,
        &collection,
        "/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint",
    )
    .expect("an owned, comparable item can be listed");

    assert_eq!(item_id, "54a73e65e779893a797fff33");
}

/// Offering to sell what the device does not hold is the mirror of the `missing` flag this whole
/// screen exists to raise.
#[test]
fn a_sell_is_refused_for_something_this_device_does_not_hold() {
    let items = MarketItems::from_response(SELLABLE_ITEMS.as_bytes()).expect("items parse");

    let result = authorize_sell(
        &items,
        &Collection::default(),
        "/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint",
    );

    assert_eq!(result, Err(ITEM_NOT_OWNED));
}

/// Refused on the backend and not merely hidden in the interface. A relic's create body needs a
/// `subtype` this application never collects, so the request would come back 400 -- but the
/// refusal belongs here, where a caller cannot route around it.
#[test]
fn a_sell_is_refused_for_an_item_whose_identity_names_no_single_row() {
    let items = MarketItems::from_response(SELLABLE_ITEMS.as_bytes()).expect("items parse");
    let path = "/Lotus/Types/Game/Projections/T1VoidProjectionBratonPrimeDBronze";
    let collection = collection_holding(&[path]);

    assert_eq!(
        authorize_sell(&items, &collection, path),
        Err(ITEM_NOT_LISTABLE)
    );
}
