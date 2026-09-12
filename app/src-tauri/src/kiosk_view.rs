//! What the overlay draws once the screen has been read.
//!
//! Recognition answers "which slot holds which name"; this module answers "what is that worth".
//! Every number the chips show -- platinum, ducats, owned counts, the basket total -- comes from
//! joins against data the app already holds, never from the screen: the spec's no-digit-OCR rule
//! keeps the game's own numerals out of the pipeline entirely.

use serde::Serialize;
use warframe_acquisition::RewardCatalogEntry;

use crate::kiosk_ocr::{BasketRow, GridCell};

/// One grid tile's corner chip. Cells the price table cannot price render nothing at all, per the
/// spec, so a chip existing already says its platinum resolved.
#[derive(Clone, Debug, Serialize)]
pub struct CellChip {
    pub col: u32,
    pub row: u32,
    pub name: String,
    pub platinum: Option<u32>,
}

/// One basket row's platinum value. The game already draws each row's ducats; this only adds
/// what it does not show. A row the price table cannot price stays listed -- the game still
/// drew it, and the frontend decides what a missing price looks like -- but contributes nothing
/// to the total.
#[derive(Clone, Debug, Serialize)]
pub struct BasketChip {
    pub index: u32,
    pub name: String,
    pub platinum: Option<u32>,
}

/// One poller epoch's whole overlay payload.
#[derive(Clone, Debug, Default, Serialize)]
pub struct KioskView {
    /// Monotonic kiosk-visit identity. Frontend event handlers reject queued scroll verdicts from
    /// an earlier visit after a close/reopen.
    pub session: u64,
    pub epoch: u64,
    pub cells: Vec<CellChip>,
    pub basket: Vec<BasketChip>,
    pub total_plat: u64,
    /// The grid's scroll offset from the calibration rows, in design pixels: the reads were
    /// taken with the label bands shifted by exactly this, so the chips belong this far from
    /// their unscrolled positions. The basket pane never scrolls and needs no offset.
    pub scroll_dy: i32,
}

/// The active kiosk visit and its latest published view. Session ids make every worker output
/// conditional on still owning the current visit; the mutex is the single ordering point shared
/// by open, close, publication and event emission.
#[derive(Default)]
struct KioskSlot {
    next_session: u64,
    active_session: Option<u64>,
    view: Option<KioskView>,
}

#[derive(Default)]
pub struct KioskState(std::sync::Mutex<KioskSlot>);

impl KioskState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Start a new visit, invalidating all outputs from previous workers and clearing their view.
    pub fn begin_session(&self) -> u64 {
        let Ok(mut slot) = self.0.lock() else {
            return 0;
        };
        slot.next_session = slot.next_session.wrapping_add(1).max(1);
        slot.active_session = Some(slot.next_session);
        slot.view = None;
        slot.next_session
    }

    /// End this visit if it is still current. A delayed close from an older visit is harmless.
    pub fn end_session(&self, session: u64) {
        if let Ok(mut slot) = self.0.lock()
            && slot.active_session == Some(session)
        {
            slot.active_session = None;
            slot.view = None;
        }
    }

    /// Exposed for lifecycle tests and for attaching a worker to the visit just opened.
    pub fn active_session(&self) -> Option<u64> {
        self.0.lock().ok().and_then(|slot| slot.active_session)
    }

    /// Publish a fresh epoch's view; retained for state-cell callers that do not own a session.
    pub fn set(&self, view: KioskView) {
        if let Ok(mut slot) = self.0.lock() {
            slot.view = Some(view);
        }
    }

    /// Publish and announce a fresh epoch only while the worker still owns the active kiosk
    /// visit. Both operations share the lifecycle mutex: close/reopen cannot begin after the
    /// state write but before its session-bearing frontend event. The state stamps the accepted
    /// session into the payload so queued frontend events obey the same boundary.
    pub fn set_if_current(
        &self,
        session: u64,
        mut view: KioskView,
        announce: impl FnOnce(),
    ) -> bool {
        let Ok(mut slot) = self.0.lock() else {
            return false;
        };
        if slot.active_session != Some(session) {
            return false;
        }
        view.session = session;
        slot.view = Some(view);
        announce();
        true
    }

    /// Run a non-view side effect only while its worker still owns the active visit.
    pub fn run_if_current(&self, session: u64, action: impl FnOnce()) -> bool {
        let Ok(slot) = self.0.lock() else {
            return false;
        };
        if slot.active_session != Some(session) {
            return false;
        }
        action();
        true
    }

    /// The latest view, or `None` when nothing has been published (or the kiosk has closed).
    pub fn get(&self) -> Option<KioskView> {
        self.0.lock().ok().and_then(|slot| slot.view.clone())
    }

    /// Unconditional reset used by process teardown and state-cell tests.
    pub fn clear(&self) {
        if let Ok(mut slot) = self.0.lock() {
            slot.active_session = None;
            slot.view = None;
        }
    }
}

/// Join recognized slots against the catalogue, the price table and the collection.
///
/// `price` and `owned` are closures so tests (and later the poller, which layers the market cache
/// under the daily dump) can supply whatever sources are live without this module knowing them.
pub fn build_view(
    epoch: u64,
    cells: &[GridCell],
    basket: &[BasketRow],
    _catalog: &[RewardCatalogEntry],
    price: impl Fn(&str) -> Option<u32>,
) -> KioskView {
    let cells = cells
        .iter()
        .filter_map(|cell| {
            let platinum = price(&cell.name)?;
            Some(CellChip {
                col: cell.col as u32,
                row: cell.row as u32,
                name: cell.name.clone(),
                platinum: Some(platinum),
            })
        })
        .collect();

    let basket: Vec<BasketChip> = basket
        .iter()
        .map(|row| BasketChip {
            index: row.index as u32,
            name: row.name.clone(),
            platinum: price(&row.name).and_then(|unit_price| unit_price.checked_mul(row.quantity)),
        })
        .collect();

    let total_plat = basket
        .iter()
        .filter_map(|chip| chip.platinum)
        .map(u64::from)
        .sum();

    KioskView {
        session: 0,
        epoch,
        scroll_dy: 0,
        cells,
        basket,
        total_plat,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Vec<RewardCatalogEntry> {
        [("Tiberon Prime Barrel", 45)]
            .into_iter()
            .map(|(name, ducats)| RewardCatalogEntry {
                name: name.to_owned(),
                ducats,
            })
            .collect()
    }

    fn cell(col: usize, row: usize, name: &str) -> GridCell {
        GridCell {
            col,
            row,
            name: name.to_owned(),
            score: 0.9,
        }
    }

    fn basket_row(index: usize, name: &str) -> BasketRow {
        BasketRow {
            index,
            name: name.to_owned(),
            score: 0.9,
            quantity: 1,
        }
    }

    #[test]
    fn priced_cells_become_chips() {
        let cells = [
            cell(0, 0, "Tiberon Prime Barrel"),
            cell(1, 0, "Atlas Prime Chassis Blueprint"),
        ];
        let view = build_view(3, &cells, &[], &catalog(), |name| {
            (name == "Tiberon Prime Barrel").then_some(12)
        });
        assert_eq!(view.cells.len(), 1);
        assert_eq!(view.cells[0].platinum, Some(12));
    }

    #[test]
    fn basket_rows_carry_platinum_and_the_total_sums_only_priced_rows() {
        let basket = [
            basket_row(0, "Afentis Prime Blade"),
            basket_row(1, "Fulmin Prime Receiver"),
        ];
        let view = build_view(1, &[], &basket, &catalog(), |name| match name {
            "Afentis Prime Blade" => Some(6),
            "Fulmin Prime Receiver" => Some(20),
            _ => None,
        });
        assert_eq!(view.basket.len(), 2);
        assert_eq!(view.basket[0].platinum, Some(6));
        assert_eq!(view.basket[1].platinum, Some(20));
        assert_eq!(view.total_plat, 26);
    }

    #[test]
    fn basket_rows_price_every_selected_copy() {
        let basket = [BasketRow {
            quantity: 2,
            ..basket_row(0, "Kompressa Prime Barrel")
        }];
        let view = build_view(1, &[], &basket, &catalog(), |name| {
            (name == "Kompressa Prime Barrel").then_some(7)
        });

        assert_eq!(view.basket[0].platinum, Some(14));
        assert_eq!(view.total_plat, 14);
    }

    #[test]
    fn a_single_basket_row_keeps_unit_price() {
        let basket = [basket_row(0, "Kompressa Prime Barrel")];
        let view = build_view(1, &[], &basket, &catalog(), |name| {
            (name == "Kompressa Prime Barrel").then_some(7)
        });

        assert_eq!(view.basket[0].platinum, Some(7));
        assert_eq!(view.total_plat, 7);
    }

    #[test]
    fn the_epoch_passes_through_for_the_frontend_transform_reset() {
        let view = build_view(42, &[], &[], &catalog(), |_| None);
        assert_eq!(view.epoch, 42);
        assert_eq!(view.total_plat, 0);
    }
}
