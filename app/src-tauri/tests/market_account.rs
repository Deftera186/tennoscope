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

/// The shape of `/v2/items`, in the cases selling cares about: an ordinary part whose path names
/// one collection row, a mod whose listing must carry a rank, and a relic whose one entry stands
/// for four refinements. Shapes verbatim from the live table.
const SELLABLE_ITEMS: &str = r#"{"apiVersion":"0.25.0","data":[
    {"id":"54a73e65e779893a797fff33","slug":"braton_prime_blueprint",
     "gameRef":"/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint",
     "i18n":{"en":{"name":"Braton Prime Blueprint"}}},
    {"id":"54ca39abe7798915c1c11e10","slug":"creeping_bullseye",
     "gameRef":"/Lotus/Upgrades/Mods/Pistol/DualStat/CorruptedCritChanceFireRatePistol",
     "maxRank":5,"i18n":{"en":{"name":"Creeping Bullseye"}}},
    {"id":"6054dd685221e30057500f63","slug":"axi_a1_relic",
     "gameRef":"/Lotus/Types/Game/Projections/T4VoidProjectionE","bulkTradable":true,
     "subtypes":["intact","exceptional","flawless","radiant"],
     "i18n":{"en":{"name":"Axi A1 Relic"}}}
],"error":null}"#;

const MOD_PATH: &str = "/Lotus/Upgrades/Mods/Pistol/DualStat/CorruptedCritChanceFireRatePistol";
const RELIC_BASE: &str = "/Lotus/Types/Game/Projections/T4VoidProjectionE";

fn entry_of(path: &str, quantity: u32, rank: Option<u32>, max_rank: Option<u32>) -> InventoryEntry {
    let id = ItemId::new(path).expect("item id");
    let item = CatalogItem::new(id, "Item", Category::PrimePart).expect("catalog item");
    let entry = InventoryEntry::new(item, quantity);
    match rank {
        Some(rank) => entry.with_rank(rank, max_rank),
        None => entry,
    }
}

fn collection_holding(entries: &[InventoryEntry]) -> Collection {
    let mut collection = Collection::default();
    collection.replace(InventorySnapshot::coherent(entries.to_vec()).expect("snapshot"));
    collection
}

#[test]
fn a_sell_resolves_the_collections_row_to_the_listing_it_will_be_published_as() {
    let items = MarketItems::from_response(SELLABLE_ITEMS.as_bytes()).expect("items parse");
    let collection = collection_holding(&[entry_of(
        "/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint",
        3,
        None,
        None,
    )]);

    let listing = authorize_sell(
        &items,
        &collection,
        "/Lotus/Types/Recipes/Weapons/BratonPrimeBlueprint",
    )
    .expect("an owned, comparable item can be listed");

    assert_eq!(listing.item_id, "54a73e65e779893a797fff33");
    assert_eq!(listing.rank, None);
    assert_eq!(listing.subtype, None);
    assert_eq!(listing.per_trade, None);
}

/// The row is what gets sold, and the guard is the row id rather than the path: a card held only
/// maxed has no unranked stack to list, and a path with no such row is not held at all. The old
/// path match would have offered the unranked stack's listing against copies that are ranked.
#[test]
fn a_sell_names_one_row_not_the_cards_whole_path() {
    let items = MarketItems::from_response(SELLABLE_ITEMS.as_bytes()).expect("items parse");
    let collection = collection_holding(&[entry_of(&format!("{MOD_PATH}#5"), 1, Some(5), Some(5))]);

    assert_eq!(
        authorize_sell(&items, &collection, MOD_PATH),
        Err(ITEM_NOT_OWNED)
    );
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

/// A maxed copy lists at its ceiling and a relic refinement at its subtype, both resolved from the
/// row itself: this is the guard that decides what the create body will carry, and neither value
/// is ever taken from the frontend.
#[test]
fn a_maxed_copy_and_a_relic_refinement_resolve_their_own_context() {
    let items = MarketItems::from_response(SELLABLE_ITEMS.as_bytes()).expect("items parse");
    let collection = collection_holding(&[
        entry_of(&format!("{MOD_PATH}#5"), 1, Some(5), Some(5)),
        entry_of(&format!("{RELIC_BASE}Bronze"), 6, None, None),
    ]);

    let maxed = authorize_sell(&items, &collection, &format!("{MOD_PATH}#5"))
        .expect("a maxed copy is one of the two ranks the market quotes");
    assert_eq!(maxed.item_id, "54ca39abe7798915c1c11e10");
    assert_eq!(maxed.rank, Some(5));

    let relic = authorize_sell(&items, &collection, &format!("{RELIC_BASE}Bronze"))
        .expect("a refinement is one of the four rows a relic entry stands for");
    assert_eq!(relic.item_id, "6054dd685221e30057500f63");
    assert_eq!(relic.subtype, Some("intact"));
    assert_eq!(relic.per_trade, Some(1));
}

/// Refused on the backend and not merely hidden in the interface. A part-ranked copy has no rank
/// the market would accept -- it quotes a card at rank 0 and at its ceiling only -- so the
/// refusal belongs here, where a caller cannot route around it.
#[test]
fn a_sell_is_refused_for_a_copy_held_part_way_up() {
    let items = MarketItems::from_response(SELLABLE_ITEMS.as_bytes()).expect("items parse");
    let collection = collection_holding(&[entry_of(&format!("{MOD_PATH}#3"), 1, Some(3), Some(5))]);

    assert_eq!(
        authorize_sell(&items, &collection, &format!("{MOD_PATH}#3")),
        Err(ITEM_NOT_LISTABLE)
    );
}
