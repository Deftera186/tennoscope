//! What the overlay draws once the screen has been read.
//!
//! Recognition answers "which slot holds which name"; this module answers "what is that worth".
//! A chip carries a name and a platinum figure and nothing else, so both are joins against data
//! the app already holds: reading the game's own ducat or platinum numerals is a spec non-goal,
//! so they never enter this pipeline. The owned count a player sees is the game's.

use serde::Serialize;
use warframe_domain::KioskMastery;

use crate::kiosk_ocr::{BasketRow, GridCell};

/// One grid tile's corner chip, kept when its price, its mastery or both resolve. A mastery chip
/// is drawn as a strip that carries the price as well, or a dash when the tile has none.
#[derive(Clone, Debug, Serialize)]
pub struct CellChip {
    pub col: u32,
    pub row: u32,
    pub name: String,
    pub platinum: Option<u32>,
    pub mastery: Option<KioskMastery>,
}

/// Whether the kiosk payload may carry mastery strips. Strips need a live inventory; a saved
/// collection can only say what is mastered, which no strip needs to repeat.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MasteryStatus {
    #[default]
    Off,
    Live,
    Unavailable,
}

/// One basket row's platinum value. The game already draws each row's ducats; this only adds
/// what it does not show. A row the price table cannot price stays listed (the game still
/// drew it, and the frontend decides what a missing price looks like) but contributes nothing
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
    /// Published in the overlay's design-pixel space (the capture-space measurement is
    /// normalized at publication; see `kiosk_scroll::to_design_px`).
    pub scroll_dy: i32,
    /// Whether this payload may carry mastery strips. The frontend draws its note from this,
    /// not from the cells.
    pub mastery_status: MasteryStatus,
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

/// Join recognized slots against the price table and the mastery lookup; a cell survives when
/// either hits. Both are closures so tests and the poller can supply whichever source is live.
pub fn build_view(
    epoch: u64,
    cells: &[GridCell],
    basket: &[BasketRow],
    price: impl Fn(&str) -> Option<u32>,
    mastery: impl Fn(&str) -> Option<KioskMastery>,
    mastery_status: MasteryStatus,
) -> KioskView {
    let cells = cells
        .iter()
        .filter_map(|cell| {
            let platinum = price(&cell.name);
            let cell_mastery = mastery(&cell.name);
            if platinum.is_none() && cell_mastery.is_none() {
                return None;
            }
            Some(CellChip {
                col: cell.col as u32,
                row: cell.row as u32,
                name: cell.name.clone(),
                platinum,
                mastery: cell_mastery,
            })
        })
        .collect();

    // Chips carry the unit price, the number a marketplace listing shows for one copy, so
    // the basket row and the grid tile agree per name, whatever the stack size. Only the
    // total multiplies: that is the one place "what is this basket worth" means the pile.
    let basket_chips: Vec<BasketChip> = basket
        .iter()
        .map(|row| BasketChip {
            index: row.index as u32,
            name: row.name.clone(),
            platinum: price(&row.name),
        })
        .collect();

    let total_plat = basket_chips
        .iter()
        .zip(basket)
        .filter_map(|(chip, row)| {
            chip.platinum
                .map(|unit| u64::from(unit) * u64::from(row.quantity))
        })
        .sum();

    KioskView {
        session: 0,
        epoch,
        scroll_dy: 0,
        cells,
        basket: basket_chips,
        total_plat,
        mastery_status,
    }
}

/// Paint every published chip out of a captured frame.
///
/// Monitor-scoped capture (the portal/KWin rungs every native Wayland session uses) frames
/// the composite: our own overlay is in the picture. Chips sitting on the captured kiosk
/// read straight back into the pipeline, so before any profile or OCR the pipeline
/// overwrites exactly those boxes with the frame's own background.
///
/// Each box is filled with the median luma of its border ring: the label background the game
/// drew behind the chip, which is inert to both the edge-count profile and the per-crop OCR
/// normalisation. Nothing under a chip was captured anyway, since the chip occludes it on
/// screen, so masking loses no information about the game, only about ourselves.
/// `mask_dy` is where the chips actually sit in design pixels right now: the published
/// phase plus any scroll deltas streamed since. Tiles must shift the same distance the
/// frontend slides them, or the moment after a scroll the previous epoch's chips re-enter
/// the capture above their masks (the self-readback failure class, one scroll earlier).
pub fn mask_published_chips(frame: &mut image::DynamicImage, view: &KioskView, mask_dy: i32) {
    let (width, height) = (frame.width(), frame.height());
    let mut rects = Vec::with_capacity(view.cells.len() + view.basket.len() + 1);
    for cell in &view.cells {
        // A strip replaces its chip and is wider than it: mask the strip's own width, or
        // the strip's text reads back into the scroll locator and the quantity OCR.
        let chip_w_1080 = cell
            .mastery
            .map_or(crate::kiosk_geometry::PRICE_CHIP_W_1080, |mastery| {
                crate::kiosk_geometry::strip_mask_width_1080(mastery.held)
            });
        if let Some(rect) = crate::kiosk_geometry::grid_chip_mask(
            width,
            height,
            cell.col as usize,
            cell.row as usize,
            mask_dy,
            chip_w_1080,
        ) {
            rects.push(rect);
        }
    }
    for row in &view.basket {
        if let Some(rect) =
            crate::kiosk_geometry::basket_chip_mask(width, height, row.index as usize)
        {
            rects.push(rect);
        }
    }
    if !view.basket.is_empty() {
        rects.push(crate::kiosk_geometry::total_chip_mask(width, height));
    }
    if rects.is_empty() {
        return;
    }
    let mut rgb = frame.to_rgb8();
    for (x, y, w, h) in rects {
        paint_over(&mut rgb, x, y, w, h);
    }
    *frame = image::DynamicImage::ImageRgb8(rgb);
}

/// Fill `[x, y, w, h)` with the median luma of its one-pixel border ring, clamped to the
/// image. A degenerate rect or one pasted clean off screen paints nothing.
fn paint_over(image: &mut image::RgbImage, x: u32, y: u32, w: u32, h: u32) {
    let (width, height) = image.dimensions();
    let (x1, y1) = (x.min(width), y.min(height));
    let (x2, y2) = (
        (x.saturating_add(w)).min(width),
        (y.saturating_add(h)).min(height),
    );
    if x2 - x1 < 3 || y2 - y1 < 3 {
        return;
    }
    let mut ring = Vec::new();
    let push = |px: u32, py: u32, ring: &mut Vec<u8>| {
        if px < width && py < height {
            let p = image.get_pixel(px, py).0;
            ring.push(
                ((299 * u32::from(p[0]) + 587 * u32::from(p[1]) + 114 * u32::from(p[2])) / 1000)
                    as u8,
            );
        }
    };
    for px in x1.saturating_sub(1)..=(x2).min(width.saturating_sub(1)) {
        push(px, y1.saturating_sub(1), &mut ring);
        push(px, (y2).min(height.saturating_sub(1)), &mut ring);
    }
    for py in y1.saturating_sub(1)..=(y2).min(height.saturating_sub(1)) {
        push(x1.saturating_sub(1), py, &mut ring);
        push((x2).min(width.saturating_sub(1)), py, &mut ring);
    }
    if ring.is_empty() {
        return;
    }
    ring.sort_unstable();
    let fill = ring[ring.len() / 2];
    for py in y1..y2 {
        for px in x1..x2 {
            image.put_pixel(px, py, image::Rgb([fill, fill, fill]));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let view = build_view(
            3,
            &cells,
            &[],
            |name| (name == "Tiberon Prime Barrel").then_some(12),
            |_| None,
            MasteryStatus::Off,
        );
        assert_eq!(view.cells.len(), 1);
        assert_eq!(view.cells[0].platinum, Some(12));
    }

    #[test]
    fn basket_rows_carry_platinum_and_the_total_sums_only_priced_rows() {
        let basket = [
            basket_row(0, "Afentis Prime Blade"),
            basket_row(1, "Fulmin Prime Receiver"),
        ];
        let view = build_view(
            1,
            &[],
            &basket,
            |name| match name {
                "Afentis Prime Blade" => Some(6),
                "Fulmin Prime Receiver" => Some(20),
                _ => None,
            },
            |_| None,
            MasteryStatus::Off,
        );
        assert_eq!(view.basket.len(), 2);
        assert_eq!(view.basket[0].platinum, Some(6));
        assert_eq!(view.basket[1].platinum, Some(20));
        assert_eq!(view.total_plat, 26);
    }

    /// The row chip is the unit price, what one copy sells for and what the grid tile of
    /// the same item shows, while the total alone multiplies: a 7p barrel counted twice is
    /// worth 14 but lists for 7.
    #[test]
    fn basket_rows_show_the_unit_price_and_the_total_sums_copies() {
        let basket = [BasketRow {
            quantity: 2,
            ..basket_row(0, "Kompressa Prime Barrel")
        }];
        let view = build_view(
            1,
            &[],
            &basket,
            |name| (name == "Kompressa Prime Barrel").then_some(7),
            |_| None,
            MasteryStatus::Off,
        );

        assert_eq!(view.basket[0].platinum, Some(7));
        assert_eq!(view.total_plat, 14);
    }

    #[test]
    fn a_single_basket_row_keeps_unit_price() {
        let basket = [basket_row(0, "Kompressa Prime Barrel")];
        let view = build_view(
            1,
            &[],
            &basket,
            |name| (name == "Kompressa Prime Barrel").then_some(7),
            |_| None,
            MasteryStatus::Off,
        );

        assert_eq!(view.basket[0].platinum, Some(7));
        assert_eq!(view.total_plat, 7);
    }

    /// Both lanes price by the same name through the same table, so a card's corner chip
    /// is its basket row's chip, and the marketplace's per-copy number is what both say.
    #[test]
    fn the_same_name_prices_identically_on_the_card_and_in_the_basket() {
        let cells = [cell(0, 0, "Kompressa Prime Barrel")];
        let mut stacked = basket_row(0, "Kompressa Prime Barrel");
        stacked.quantity = 3;
        let view = build_view(
            0,
            &cells,
            &[stacked],
            |name| match name {
                "Kompressa Prime Barrel" => Some(18),
                _ => None,
            },
            |_| None,
            MasteryStatus::Off,
        );
        assert_eq!(view.cells[0].platinum, Some(18));
        assert_eq!(
            view.basket[0].platinum,
            Some(18),
            "the basket shows the unit price"
        );
        assert_eq!(view.total_plat, 54, "only the total multiplies copies");
    }

    #[test]
    fn the_epoch_passes_through_for_the_frontend_transform_reset() {
        let view = build_view(42, &[], &[], |_| None, |_| None, MasteryStatus::Off);
        assert_eq!(view.epoch, 42);
        assert_eq!(view.total_plat, 0);
    }

    #[test]
    fn a_cell_with_mastery_but_no_price_still_becomes_a_chip() {
        let cells = [cell(0, 0, "Guandao Prime Blade")];
        let view = build_view(
            7,
            &cells,
            &[],
            |_| None,
            |_| Some(KioskMastery { held: 1, uses: 2 }),
            MasteryStatus::Live,
        );
        let chip = &view.cells[0];
        assert_eq!(
            (chip.platinum, chip.mastery),
            (None, Some(KioskMastery { held: 1, uses: 2 }))
        );
        assert_eq!(view.mastery_status, MasteryStatus::Live);
    }

    #[test]
    fn a_cell_with_neither_price_nor_mastery_is_still_dropped() {
        let cells = [cell(0, 0, "Forma Blueprint")];
        let view = build_view(7, &cells, &[], |_| None, |_| None, MasteryStatus::Live);
        assert!(view.cells.is_empty());
    }
}
