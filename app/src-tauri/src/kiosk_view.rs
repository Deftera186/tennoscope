//! What the overlay draws once the screen has been read.
//!
//! Recognition answers "which slot holds which name"; this module answers "what is that worth".
//! Every number the chips show -- platinum, ducats, owned counts, the basket total -- comes from
//! joins against data the app already holds, never from the screen: the spec's no-digit-OCR rule
//! keeps the game's own numerals out of the pipeline entirely.

use serde::Serialize;
use warframe_acquisition::{RewardCatalogEntry, reward_name_matches};

use crate::kiosk_ocr::{BasketRow, GridCell};

/// One grid tile's corner chip. Cells the price table cannot price render nothing at all, per the
/// spec, so a chip existing already says its platinum resolved.
#[derive(Clone, Debug, Serialize)]
pub struct CellChip {
    pub col: u32,
    pub row: u32,
    pub name: String,
    pub platinum: Option<u32>,
    pub owned: u32,
}

/// One basket row's pair beside the game's own ducat number. A row the price table cannot price
/// stays listed -- the game still drew it, and the frontend decides what a missing price looks
/// like -- but it contributes nothing to the total.
#[derive(Clone, Debug, Serialize)]
pub struct BasketChip {
    pub index: u32,
    pub name: String,
    pub platinum: Option<u32>,
    pub ducats: u32,
}

/// One poller epoch's whole overlay payload.
#[derive(Clone, Debug, Default, Serialize)]
pub struct KioskView {
    pub epoch: u64,
    pub cells: Vec<CellChip>,
    pub basket: Vec<BasketChip>,
    pub total_plat: u64,
}

/// The poller's latest published epoch, shared with the `/kiosk` window's `get_kiosk_view`
/// command.
///
/// A lock-poisoned cell can only mean a panic while publishing; the degradation that matters is
/// that the overlay hides (reads come back empty) rather than that the app dies, so every method
/// degrades instead of propagating.
#[derive(Default)]
pub struct KioskState(std::sync::Mutex<Option<KioskView>>);

impl KioskState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish a fresh epoch's view; readers hold a copy until the next publish or a clear.
    pub fn set(&self, view: KioskView) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = Some(view);
        }
    }

    /// The latest view, or `None` when nothing has been published (or the kiosk has closed).
    pub fn get(&self) -> Option<KioskView> {
        self.0.lock().ok().and_then(|slot| slot.clone())
    }

    /// Close semantics: nothing is drawn over whatever the game shows next.
    pub fn clear(&self) {
        if let Ok(mut slot) = self.0.lock() {
            *slot = None;
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
    catalog: &[RewardCatalogEntry],
    price: impl Fn(&str) -> Option<u32>,
    owned: impl Fn(&str) -> u32,
) -> KioskView {
    let ducats_for = |name: &str| {
        catalog
            .iter()
            .find(|entry| reward_name_matches(&entry.name, name))
            .map_or(0, |entry| entry.ducats)
    };

    let cells = cells
        .iter()
        .filter_map(|cell| {
            let platinum = price(&cell.name)?;
            Some(CellChip {
                col: cell.col as u32,
                row: cell.row as u32,
                name: cell.name.clone(),
                platinum: Some(platinum),
                owned: owned(&cell.name),
            })
        })
        .collect();

    let basket: Vec<BasketChip> = basket
        .iter()
        .map(|row| BasketChip {
            index: row.index as u32,
            name: row.name.clone(),
            platinum: price(&row.name),
            ducats: ducats_for(&row.name),
        })
        .collect();

    let total_plat = basket
        .iter()
        .filter_map(|chip| chip.platinum)
        .map(u64::from)
        .sum();

    KioskView {
        epoch,
        cells,
        basket,
        total_plat,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Vec<RewardCatalogEntry> {
        [
            ("Tiberon Prime Barrel", 45),
            ("Titania Prime Systems Blueprint", 100),
            ("Atlas Prime Chassis Blueprint", 30),
            ("Afentis Prime Blade", 15),
            ("Fulmin Prime Receiver", 75),
        ]
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
        }
    }

    #[test]
    fn priced_cells_become_chips_with_owned_counts() {
        let cells = [
            cell(0, 0, "Titania Prime Systems Blueprint"),
            cell(1, 0, "Atlas Prime Chassis Blueprint"),
        ];
        let view = build_view(
            3,
            &cells,
            &[],
            &catalog(),
            |name| {
                Some(if name == "Titania Prime Systems Blueprint" {
                    30
                } else {
                    8
                })
            },
            |name| usize::from(name == "Atlas Prime Chassis Blueprint") as u32 + 1,
        );
        let chips: Vec<_> = view
            .cells
            .iter()
            .map(|chip| (chip.name.as_str(), chip.platinum, chip.owned))
            .collect();
        assert_eq!(
            chips,
            [
                ("Titania Prime Systems Blueprint", Some(30), 1),
                ("Atlas Prime Chassis Blueprint", Some(8), 2),
            ]
        );
    }

    #[test]
    fn unpriced_cells_render_nothing() {
        let cells = [
            cell(0, 0, "Tiberon Prime Barrel"),
            cell(1, 0, "Atlas Prime Chassis Blueprint"),
        ];
        let view = build_view(
            0,
            &cells,
            &[],
            &catalog(),
            |name| (name == "Tiberon Prime Barrel").then_some(12),
            |_| 0,
        );
        assert_eq!(view.cells.len(), 1);
        assert_eq!(view.cells[0].name, "Tiberon Prime Barrel");
        assert_eq!(view.cells[0].platinum, Some(12));
    }

    #[test]
    fn basket_rows_carry_ducats_and_the_total_sums_only_priced_rows() {
        let basket = [
            basket_row(0, "Afentis Prime Blade"),
            basket_row(1, "Fulmin Prime Receiver"),
            basket_row(2, "Atlas Prime Chassis Blueprint"),
        ];
        let view = build_view(
            1,
            &[],
            &basket,
            &catalog(),
            |name| match name {
                "Afentis Prime Blade" => Some(6),
                "Fulmin Prime Receiver" => Some(20),
                _ => None,
            },
            |_| 0,
        );
        assert_eq!(view.basket.len(), 3);
        assert_eq!(view.basket[0].ducats, 15);
        assert_eq!(view.basket[1].ducats, 75);
        assert_eq!(view.basket[2].platinum, None);
        assert_eq!(view.total_plat, 26);
    }

    #[test]
    fn the_epoch_passes_through_for_the_frontend_transform_reset() {
        let view = build_view(42, &[], &[], &catalog(), |_| None, |_| 0);
        assert_eq!(view.epoch, 42);
        assert_eq!(view.total_plat, 0);
    }
}
