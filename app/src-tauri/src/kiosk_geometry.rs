//! Kiosk screen geometry, calibrated against a labelled 1920x1080 capture
//! (`tests/fixtures/kiosk/kiosk-open.png`).
//!
//! Same convention as `reward_ocr.rs`: Warframe scales its HUD with window *height* and centres
//! it horizontally, so every constant below is a fraction of height: horizontal positions as a
//! signed offset of that fraction from the window's horizontal centre. Fractions of width would
//! agree at 16:9 and silently drift everywhere else.
//!
//! Measured on the fixture (2026-08-23, structural pass): six tile cards on a 207.5px pitch
//! whose right borders sit at x=265.95+207.55k (cards 190px wide, first left edge x=75.7),
//! three card rows visible at rest on an exact 222px pitch (tops y=199/421/643), with a fourth
//! row entering through the pane's bottom edge during scroll (next top y=865), the item label
//! band below each thumbnail, a sell-basket list whose ducat digits sit on baselines
//! y=243+38 1/3*k right-aligned near x=1790, and the TOTAL row's digits (16px tall, baseline
//! y=875) with its gold ducat glyph at x~1730..1749.
//!
//! The rendering anchors (`tile_anchor`, `total_row_pair`, the chip/digit sizes) have no Rust
//! consumer: the `/kiosk` window positions its chips in CSS. They stay because they are the
//! calibration of record, fixture-exact and test-asserted, and the TypeScript constants are
//! mirrors of these, not a second measurement.
#![allow(dead_code)]

pub const GRID_COLS: usize = 6;
/// Maximum card bands visible in the clipped pane while scrolling. Three fit at rest; a fourth
/// enters through the bottom edge before the first leaves through the top.
pub const GRID_ROWS: usize = 4;
/// Basket rows the pane shows at once. Row `i`'s digits sit on baseline 243+38 1/3*i and
/// the TOTAL row's digits sit on 875, so indexes 0..=15 fit (row 15's baseline is 818.0, its
/// band ends at 823 against the TOTAL band's top at ~859); a seventeenth row would print
/// into the TOTAL row itself.
pub const BASKET_ROWS: usize = 16;

/// Left edge of the basket pane at 1920x1080. Name OCR must not cross into grid column six.
const BASKET_NAME_LEFT_1080P: f32 = 1256.0;

const CAL: f32 = 1080.0;
/// The calibration's width. Only the vertical anchors are read off it, but [`grid_strip`] takes
/// a whole frame size.
const CAL_W: u32 = 1920;

/// Horizontal positions are `(pixel_at_1920 - 960) / 1080`; vertical are `pixel / 1080`.
const fn fx(px: f32) -> f32 {
    px / CAL
}

const fn fcx(px: f32) -> f32 {
    (px - 960.0) / CAL
}

const COL_PITCH: f32 = fx(207.5);
const TILE_W: f32 = fx(190.0);
const GRID_LEFT: f32 = fcx(76.0);
const ROW_TOPS: [f32; GRID_ROWS] = [fx(199.0), fx(421.0), fx(643.0), fx(865.0)];
/// The grid pane's clip edge in design pixels, measured on a live 1080p frame: card-column
/// mean luma drops 77 to 48 between y=982 and y=985, so 983 is where the pane ends. A label
/// band that would land at y=1009 is never rendered. The game clips before drawing it.
const PANE_BOTTOM: f32 = fx(983.0);
/// The strip's height in design pixels: `grid_strip`'s y extent (`193..983` at the
/// calibration). Vertical offsets measured in the strip's own rows rebase onto this span
/// for the overlay (`kiosk_scroll::to_design_px`).
pub const GRID_STRIP_H_1080: i32 = 790;

/// The grid's vertical period in design pixels: one row's card top to the next's
/// (`ROW_TOPS` differences, 421-199 and 643-421). The scroll locator folds the pane's row
/// profile over this, because a grid scrolled by any amount repeats itself on it.
pub const ROW_PITCH_1080: i32 = 222;
/// Where the unscrolled grid's first label band starts, and how tall the band is. These are
/// the locator's anchors, not the OCR crop's: the crop is deliberately taller (three-line
/// labels stack upward out of a two-line box), and a locator window that grew with it would
/// average the label's brightness away against the artwork above it.
pub const LABEL_BAND_TOP_1080: i32 = 343;
pub const LABEL_BAND_H_1080: i32 = 46;
/// Label crop top relative to its row's card top: grown upward from the two-line band (144)
/// because three-line labels stack upward out of it. *Styanax Prime Neuroptics Blueprint*
/// renders its first line at card top +124, 20px above the old crop, and went unpriced for
/// it (measured live, 2026-08-23). The bottom edge stays where it was.
const LABEL_DY: f32 = fx(122.0);
/// Label crop height: the two-line geometry's bottom edge (+190) plus the upward growth.
const LABEL_H: f32 = fx(68.0);
/// Each card border stroke's position in design pixels, measured off a live 1080p capture
/// (the lit columns were 264/265, 471/472, 679/680, 1094/1095, 1302/1303; col3 interpolated
/// on the 207.6px pitch the others confirm). A chip's right edge sits on these, flush with
/// its card's top-right corner; see `tile_anchor`.
const COL_RIGHT_1080: [f32; 6] = [264.0, 471.5, 679.5, 887.0, 1094.5, 1302.5];

/// Digit baseline of the first basket row; rows follow on a 38 1/3px pitch.
const BASKET_FIRST_BASELINE: f32 = fx(243.0);
const BASKET_PITCH: f32 = fx(115.0 / 3.0);
/// Right edge where our `[icon][digits]` pair ends in a basket row: 9px left of the game's
/// earliest digit (the pane border sits at x~1814, so a left anchor there overflows).
const ROW_PAIR_RIGHT: f32 = fcx(1750.0);

/// Right edge of our `[icon][digits]` pair in the TOTAL row. The game's gold glyph slides
/// left as its ducat count widens, so the pair ends at 1700: ~13px clear of a five-digit
/// total, still clear at six.
const TOTAL_PAIR_RIGHT: f32 = fcx(1700.0);
const TOTAL_BASELINE: f32 = fx(875.0);

/// Overlay digit/icon sizes, matching the game's own (see spec).
pub const DIGIT_H_TOTAL: f32 = fx(16.0);
pub const DIGIT_H_ROW: f32 = fx(15.0);
pub const ICON_H_TOTAL: f32 = fx(19.0);
pub const ICON_H_ROW: f32 = fx(18.0);
/// Chip text size for grid corner chips.
pub const CHIP_TEXT: f32 = fx(17.0);

/// Column `col`'s left edge in pixels.
fn col_left(width: u32, height: u32, col: usize) -> f32 {
    width as f32 / 2.0 + (GRID_LEFT + COL_PITCH * col as f32) * height as f32
}

/// The OCR crop over one visible tile label: `(x, y, w, h)`, or `None` outside the clipped pane.
///
/// `dy` locates the topmost rendered band relative to the calibration. Four row positions are
/// enumerated because a scroll phase can expose the next band through the pane's bottom edge.
pub fn grid_label_rect(
    width: u32,
    height: u32,
    col: usize,
    row: usize,
    dy: i32,
) -> Option<(u32, u32, u32, u32)> {
    if col >= GRID_COLS || row >= GRID_ROWS {
        return None;
    }
    let x = col_left(width, height, col).round() as u32;
    let y_f = (ROW_TOPS[row] + LABEL_DY) * height as f32 + dy as f32;
    if y_f < 0.0 {
        return None;
    }
    let y = y_f.round() as u32;
    if y_f + (LABEL_H * height as f32) > PANE_BOTTOM * height as f32 {
        return None;
    }
    Some((
        x,
        y,
        (TILE_W * height as f32).round() as u32,
        (LABEL_H * height as f32).round() as u32,
    ))
}

/// The price chip's width in 1080p design pixels. A mastery strip replaces the chip and is
/// wider, so the width travels with the call.
pub const PRICE_CHIP_W_1080: f32 = 100.0;
/// The game's owned badge ends about 47 design px into a tile with one digit and 59 with two,
/// so the strip stops 51 or 63 px short of the tile's left edge.
pub const BADGE_CLEARANCE_1080: f32 = 51.0;
pub const BADGE_CLEARANCE_TWO_DIGITS_1080: f32 = 63.0;

/// A mastery strip's width in 1080p design pixels: the tile minus the badge clearance for
/// the count the game is showing.
pub fn strip_mask_width_1080(held: u32) -> f32 {
    190.0
        - if held >= 10 {
            BADGE_CLEARANCE_TWO_DIGITS_1080
        } else {
            BADGE_CLEARANCE_1080
        }
}

/// Where a grid chip's top-right corner sits, in pixels.
///
/// The corner is the card border stroke's own position, measured per column off a live
/// capture (2026-08-24): the 207.5px column pitch rasterizes each border on a different
/// half-pixel, and a pitch formula rounded once per chip drifted up to 5px by the last
/// column. The player asked for pixel-for-pixel and the formula could not deliver it.
pub fn tile_anchor(width: u32, height: u32, col: usize, row: usize) -> Option<(f32, f32)> {
    if col >= GRID_COLS || row >= GRID_ROWS {
        return None;
    }
    // An edge on the stroke's centre renders as its floor: the stroke's last full pixel.
    let scale = height as f32 / 1080.0;
    let x = (width as f32 / 2.0 + (COL_RIGHT_1080[col] - 960.0) * scale).floor();
    let y = ROW_TOPS[row] * height as f32;
    Some((x, y))
}

/// A basket row's pair right edge and digit baseline, in pixels: `(right_x, baseline_y)`.
/// The overlay right-aligns its `[icon][digits]` pair onto `right_x` with `translateX(-100%)`.
pub fn basket_row_pair(width: u32, height: u32, row: usize) -> Option<(f32, f32)> {
    if row >= BASKET_ROWS {
        return None;
    }
    let right = width as f32 / 2.0 + ROW_PAIR_RIGHT * height as f32;
    let baseline = (BASKET_FIRST_BASELINE + BASKET_PITCH * row as f32) * height as f32;
    Some((right, baseline))
}

/// The screen area one published grid chip occupies, in pixels: the chip's top-right corner
/// sits on `tile_anchor(col, row)` and extends left. Capture rectangles of every monitor
/// include our own overlay (portal captures the composited monitor), so the kiosk pipeline
/// masks exactly these boxes out of a frame before profiles and OCR. An unread chip cannot
/// fold the locator onto the card-top row or feed digits to a quantity read.
///
/// The box is the chip's own geometry grown by a few pixels of antialias fringe; text behind
/// it is occluded on screen anyway, so nothing readable is masked out.
pub fn grid_chip_mask(
    width: u32,
    height: u32,
    col: usize,
    row: usize,
    dy: i32,
    chip_w_1080: f32,
) -> Option<(u32, u32, u32, u32)> {
    let (right, top) = tile_anchor(width, height, col, row)?;
    let scale = height as f32;
    // `dy` arrives in the overlay's design pixels (the published scroll offset); tile_anchor
    // works in capture pixels. Pane and frame scale together, so one height factor converts.
    let top = top + dy as f32 / CAL * scale;
    if top < ROW_TOPS_MIN_CLIP * scale || top > PANE_BOTTOM * scale {
        return None;
    }
    Some(mask_box(
        right,
        top,
        chip_w_1080,
        34.0,
        scale,
        width,
        height,
    ))
}

/// The mask box for one basket row's chip ([icon][digits] right-aligned on the row's pair
/// edge). Drawn even for an unpriced row (the em dash), so it is masked regardless.
pub fn basket_chip_mask(width: u32, height: u32, row: usize) -> Option<(u32, u32, u32, u32)> {
    let (right, baseline) = basket_row_pair(width, height, row)?;
    let scale = height as f32;
    // The chip's bottom edge sits on the baseline plus its descent; it grows upward.
    let bottom = baseline + fx(PAIR_DESCENT_PX + 2.0) * scale;
    Some(mask_box(
        right,
        bottom - fx(36.0) * scale,
        112.0,
        36.0,
        scale,
        width,
        height,
    ))
}

/// The basket TOTAL row's chip mask, same construction.
pub fn total_chip_mask(width: u32, height: u32) -> (u32, u32, u32, u32) {
    let (right, baseline) = total_row_pair(width, height);
    let scale = height as f32;
    let bottom = baseline + fx(PAIR_DESCENT_PX + 2.0) * scale;
    mask_box(
        right,
        bottom - fx(40.0) * scale,
        140.0,
        40.0,
        scale,
        width,
        height,
    )
}

/// Digits' descender drop under a baseline, in 1080p design pixels (mirrors the overlay's).
const PAIR_DESCENT_PX: f32 = 3.0;
/// The grid pane's top clip as a height fraction: chips scrolled above it are not drawn.
const ROW_TOPS_MIN_CLIP: f32 = fx(193.0);

/// Clip `[right-w_1080*scale, top) x (w_1080, h_1080)*scale` into the frame.
fn mask_box(
    right: f32,
    top: f32,
    w_1080: f32,
    h_1080: f32,
    scale: f32,
    width: u32,
    height: u32,
) -> (u32, u32, u32, u32) {
    let w = (fx(w_1080) * scale).round() as u32;
    let h = (fx(h_1080) * scale).round() as u32;
    let right = (right + fx(3.0) * scale).round().clamp(0.0, width as f32) as u32;
    let top = (top - fx(3.0) * scale).round().clamp(0.0, height as f32) as u32;
    (
        right.saturating_sub(w).min(width),
        top,
        w.min(width),
        h.min(height),
    )
}

/// The TOTAL row pair's right edge and digit baseline, in pixels: `(right_x, baseline_y)`.
pub fn total_row_pair(width: u32, height: u32) -> (f32, f32) {
    (
        width as f32 / 2.0 + TOTAL_PAIR_RIGHT * height as f32,
        TOTAL_BASELINE * height as f32,
    )
}

/// The grid pane region the scroll tracker profiles: every column's width and the whole
/// pane's height, `(x, y, w, h)` in pixels.
///
/// The pane, not the window: the basket beside it never moves, and rows outside it (title
/// bar, navigation) carry no scroll information. Including them only dilutes the correlation.
/// The bottom is the pane's own clip edge, not the last calibrated row: at some scroll phases
/// the game renders a fourth label band below row 2, and a band the tracker cannot see is a
/// row of items that never gets read.
pub fn grid_strip(width: u32, height: u32) -> (u32, u32, u32, u32) {
    let left = col_left(width, height, 0) - fx(6.0) * height as f32;
    let right = col_left(width, height, GRID_COLS - 1) + (TILE_W + fx(6.0)) * height as f32;
    let top = ROW_TOPS[0] * height as f32 - fx(6.0) * height as f32;
    // The pane's own clip edge, measured on a live 1080p frame (card content stops between
    // y=982 and y=985). The strip has to reach it: at some scroll phases the pane renders a
    // fourth label band as low as y=937, and the locator can only find bands it can see.
    let bottom = PANE_BOTTOM * height as f32;
    (
        left.round() as u32,
        top.round() as u32,
        (right - left).round() as u32,
        (bottom - top).round() as u32,
    )
}

/// Where the scroll locator's label bands fall inside a row profile of `rows` samples, in that
/// profile's own pixels.
///
/// The locator reads a profile of the [`grid_strip`] pane, and every profile it can be handed
/// (a 1440p capture's strip, a downscaled one, the 1080p calibration itself) is the same pane
/// at a different length. That ratio is the only thing separating the calibrated anchors from
/// the ones the locator needs, so it is applied here rather than at each call site: two callers
/// re-deriving `profile.len() / strip_h` had already drifted apart on which of the three numbers
/// they remembered to scale.
pub fn label_anchors(rows: usize) -> LabelAnchors {
    let (_x, cal_top, _w, cal_h) = grid_strip(CAL_W, CAL as u32);
    let scale = rows as f32 / cal_h as f32;
    let at = |px: i32| (px as f32 * scale).round() as i32;
    LabelAnchors {
        strip_top: at(cal_top as i32),
        first_top: at(LABEL_BAND_TOP_1080),
        pitch: at(ROW_PITCH_1080),
        band: at(LABEL_BAND_H_1080),
    }
}

/// The label geometry [`label_anchors`] measures, in one profile's pixel space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LabelAnchors {
    /// The profile's first row, as a screen offset: the locator folds absolute rows.
    pub strip_top: i32,
    /// The topmost calibrated label band.
    pub first_top: i32,
    /// One row's vertical period.
    pub pitch: i32,
    /// A label band's height.
    pub band: i32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 1920x1080 calibration must reproduce the fixture's measured pixels exactly; these
    /// numbers were measured off the capture structurally: count-badge centres at
    /// (tile_left+20, tile_top+17.5) on a 207.5x222 grid, card borders at
    /// x=265.95+207.55k, ducat digit baselines. Any drift here is a drifted overlay.
    #[test]
    fn chip_masks_cover_where_the_overlay_draws() {
        // The grid chip's top-right corner is the tile anchor; the mask spans left of it.
        let (x, y, w, h) =
            grid_chip_mask(1920, 1080, 0, 0, 0, PRICE_CHIP_W_1080).expect("a visible tile masks");
        let (ax, ay) = tile_anchor(1920, 1080, 0, 0).unwrap();
        let right = (ax.round() as u32 + 3).min(1920);
        assert_eq!(
            x + w,
            right,
            "mask ends at the chip's right edge plus bleed"
        );
        assert!(
            (ay as u32) < y + 6 && (ay as u32) > y,
            "mask starts at the chip top"
        );
        assert!(
            w >= 90 && h >= 25,
            "mask covers icon + four digits: {x},{y},{w},{h}"
        );
        // A scrolled chip rides its row: the dy offset moves the mask with the publish.
        let scrolled =
            grid_chip_mask(1920, 1080, 0, 0, 40, PRICE_CHIP_W_1080).expect("scrolled chip masks");
        assert_eq!(scrolled.1, y + 40);
        // Chips scrolled past the pane's own clip edge are not drawn and not masked.
        assert_eq!(
            grid_chip_mask(1920, 1080, 0, 0, 900, PRICE_CHIP_W_1080),
            None
        );
        // Absent tiles do not mask either.
        assert_eq!(grid_chip_mask(1920, 1080, 6, 0, 0, PRICE_CHIP_W_1080), None);

        // Basket chips right-align on the row's pair edge, above its baseline.
        let (bx, by, bw, bh) = basket_chip_mask(1920, 1080, 1).expect("row 1 masks");
        let (right, baseline) = basket_row_pair(1920, 1080, 1).unwrap();
        assert_eq!(bx + bw, (right.round() as u32 + 3).min(1920));
        assert!(
            (by + bh) as f32 > baseline - 5.0 && (by + bh) as f32 <= baseline + 8.0,
            "basket mask sits on the chip band just above the baseline: {by}+{bh} vs {baseline}"
        );
        // Rows past BASKET_ROWS do not exist to mask.
        assert_eq!(basket_chip_mask(1920, 1080, BASKET_ROWS), None);
        // A scrolled publish shifts masks by design pixels: at 1440p capture scale the
        // +30-design-pixel anchor is +40 capture pixels.
        let base = grid_chip_mask(2560, 1440, 0, 0, 0, PRICE_CHIP_W_1080).unwrap();
        let shifted = grid_chip_mask(2560, 1440, 0, 0, 30, PRICE_CHIP_W_1080).unwrap();
        assert_eq!(shifted.1 - base.1, 40, "design dy scales to capture px");
        assert_eq!(shifted.0, base.0, "horizontal anchor is scroll-immune");
        // A strip mask shares the price chip's right edge and spans the strip width.
        let strip = grid_chip_mask(2560, 1440, 0, 0, 0, strip_mask_width_1080(2)).unwrap();
        assert_eq!(strip.0 + strip.2, base.0 + base.2);
        assert_eq!(strip.2, (fx(139.0) * 1440.0).round() as u32);

        // The TOTAL row masks outright.
        let total = total_chip_mask(1920, 1080);
        let (_tr, _tb) = total_row_pair(1920, 1080);
        assert!(total.2 >= 120, "the total chip is the widest: {total:?}");
    }

    /// The strip is the tile minus the game's owned badge: 51 design px clear with a
    /// one-digit count, 63 with two. Both share the tile's right edge with the price chip.
    #[test]
    fn a_strip_mask_starts_past_the_owned_badge_and_ends_at_the_tile_edge() {
        let (w, h) = (1920, 1080);
        let price = grid_chip_mask(w, h, 3, 1, 0, PRICE_CHIP_W_1080).unwrap();
        let strip = grid_chip_mask(w, h, 3, 1, 0, strip_mask_width_1080(2)).unwrap();
        let wide = grid_chip_mask(w, h, 3, 1, 0, strip_mask_width_1080(12)).unwrap();
        // All three right-align at the same tile edge.
        assert_eq!(price.0 + price.2, strip.0 + strip.2);
        assert_eq!(strip.2, 139);
        assert_eq!(
            wide.2, 127,
            "two digits on the badge narrow the strip by 12"
        );
    }

    #[test]
    fn the_1920x1080_calibration_reproduces_the_fixture() {
        assert_eq!(
            grid_label_rect(1920, 1080, 0, 0, 0),
            Some((76, 321, 190, 68)),
            "row 0 col 0 label crop"
        );
        assert_eq!(
            grid_label_rect(1920, 1080, 5, 2, 0),
            Some((1114, 765, 190, 68)),
            "last column, last row"
        );
        // A scrolled grid sits off its calibration rows by the tracked drift: the reads shift
        // with it (2026-08-23's dead session stopped at dy=-142).
        assert_eq!(
            grid_label_rect(1920, 1080, 0, 0, -142),
            Some((76, 179, 190, 68)),
            "label crop follows the scroll"
        );
        assert_eq!(
            grid_label_rect(1920, 1080, 0, 3, -142),
            Some((76, 845, 190, 68)),
            "the fourth band entering from below is readable"
        );
        assert_eq!(
            grid_label_rect(1920, 1080, 0, 3, 0),
            None,
            "the fourth band is clipped at rest"
        );
        assert_eq!(
            grid_label_rect(1920, 1080, 0, 2, 500),
            None,
            "a band pushed past the frame has nothing to read"
        );
        let (x, y) = tile_anchor(1920, 1080, 0, 0).unwrap();
        assert_eq!((x.round(), y.round()), (264.0, 199.0));
        // Chip anchors track the measured card corners, not the assumed ones.
        let (x5, y5) = tile_anchor(1920, 1080, 5, 2).unwrap();
        assert_eq!((x5.round(), y5.round()), (1302.0, 643.0));
        let (left, baseline) = basket_row_pair(1920, 1080, 0).unwrap();
        assert_eq!((left.round(), baseline.round()), (1750.0, 243.0));
        // Pitch 38 1/3: the seventh row's digits sit at 473 on the fixture.
        let (_, sixth) = basket_row_pair(1920, 1080, 6).unwrap();
        assert_eq!(sixth.round(), 473.0);
        let (right, base) = total_row_pair(1920, 1080);
        assert_eq!((right.round(), base.round()), (1700.0, 875.0));
        // Row 15 is the last that fits: its digits sit at 818, its band ends at 823,
        // and the TOTAL band starts at ~859.
        let (_, fifteenth) = basket_row_pair(1920, 1080, 15).unwrap();
        assert_eq!(fifteenth.round(), 818.0);
        assert_eq!(basket_row_pair(1920, 1080, BASKET_ROWS), None);
    }

    /// Height fractions keep every pitch proportional on a smaller 16:9 window.
    #[test]
    fn a_smaller_16x9_window_scales_by_height_and_stays_centred() {
        let (x, _, w, _) = grid_label_rect(1280, 720, 1, 1, 0).unwrap();
        // left = 640 + (76 - 960 + 207.5) * 720/1080
        assert_eq!(
            x,
            (640.0f32 + (76.0 - 960.0 + 207.5) * 720.0 / 1080.0).round() as u32,
            "centre-relative offset scales with height"
        );
        assert_eq!(w, (190.0f32 * 720.0 / 1080.0).round() as u32);
    }

    /// Slots outside the four positions that can intersect the pane have no rectangle to read.
    #[test]
    fn out_of_range_slots_are_none() {
        assert_eq!(grid_label_rect(1920, 1080, 6, 0, 0), None);
        assert_eq!(grid_label_rect(1920, 1080, 0, 4, 0), None);
        assert_eq!(tile_anchor(1920, 1080, 6, 0), None);
        assert_eq!(tile_anchor(1920, 1080, 0, 4), None);
        assert_eq!(basket_row_pair(1920, 1080, BASKET_ROWS), None);
    }

    /// A three-line label (*Styanax Prime Neuroptics Blueprint*, measured live on the
    /// unscrolled fixture at rows +124..+182 relative to its card top) stacks UPWARD out of
    /// the two-line box, so the crop grows upward too. Its bottom edge stays on the two-line
    /// geometry every other measurement was calibrated against. Above the label is the tile's
    /// dead space: nothing bright renders between the badge zone and row +123.
    #[test]
    fn a_three_line_label_fits_inside_the_crop() {
        let (_, y, _, h) = grid_label_rect(1920, 1080, 0, 0, 0).expect("row 0 crop");
        assert!(
            y <= 199 + 124,
            "crop top {y} clips the third line's cap height"
        );
        assert!(
            y + h >= 199 + 183,
            "crop bottom {} clips the first line's descenders",
            y + h
        );
    }

    /// The locator's anchors are the unscrolled label band's top and height, and they stay put
    /// when the OCR crop grows to catch three-line labels: the crop is what tesseract reads,
    /// the band is what the profile peaks on.
    #[test]
    fn the_label_band_anchors_are_independent_of_the_ocr_crop() {
        let (_x, y, _w, h) = grid_label_rect(1920, 1080, 0, 0, 0).expect("row 0 crop");
        assert!(
            y <= LABEL_BAND_TOP_1080 as u32,
            "the crop starts at or above the band it must contain: crop {y}, band {LABEL_BAND_TOP_1080}"
        );
        assert!(
            y + h >= (LABEL_BAND_TOP_1080 + LABEL_BAND_H_1080) as u32,
            "and ends at or below its bottom"
        );
    }

    /// At the calibration size the anchors are the 1080p constants themselves.
    #[test]
    fn label_anchors_are_the_calibration_constants_at_calibration_size() {
        let (_x, cal_top, _w, cal_h) = grid_strip(CAL_W, CAL as u32);
        let at = label_anchors(cal_h as usize);
        assert_eq!(at.strip_top, cal_top as i32);
        assert_eq!(at.first_top, LABEL_BAND_TOP_1080);
        assert_eq!(at.pitch, ROW_PITCH_1080);
        assert_eq!(at.band, LABEL_BAND_H_1080);
    }

    /// A profile twice as long describes a frame twice as tall, so every anchor doubles. The
    /// callers used to do this arithmetic themselves, each from a different measure of "tall".
    #[test]
    fn label_anchors_scale_with_the_profile_length() {
        let (_x, _y, _w, cal_h) = grid_strip(CAL_W, CAL as u32);
        let at = label_anchors(cal_h as usize * 2);
        assert_eq!(at.first_top, LABEL_BAND_TOP_1080 * 2);
        assert_eq!(at.pitch, ROW_PITCH_1080 * 2);
        assert_eq!(at.band, LABEL_BAND_H_1080 * 2);
    }
}

/// The name-OCR crop over one basket row's item name.
///
/// The basket begins at x=1256. Starting farther left crosses the sixth grid card's label band:
/// an empty basket slot can then become a valid closed-set match for an adjacent grid item.
pub fn basket_label_rect(width: u32, height: u32, row: usize) -> Option<(u32, u32, u32, u32)> {
    basket_row_rect(width, height, row, BASKET_NAME_LEFT_1080P)
}

/// The quantity-OCR crop over one basket row's optional stack count and item name.
///
/// Stack prefixes are right-aligned with their item names and can extend left of the basket's
/// name boundary. Quantity OCR uses a strict digit/separator whitelist, so this wider crop may
/// include the adjacent grid while still stopping before the basket's ducat digits.
pub fn basket_quantity_rect(width: u32, height: u32, row: usize) -> Option<(u32, u32, u32, u32)> {
    basket_row_rect(width, height, row, 1080.0)
}

fn basket_row_rect(
    width: u32,
    height: u32,
    row: usize,
    left_1080: f32,
) -> Option<(u32, u32, u32, u32)> {
    if row >= BASKET_ROWS {
        return None;
    }
    let (_, baseline) = basket_row_pair(width, height, row)?;
    let x = (width as f32 / 2.0 + fcx(left_1080) * height as f32).round() as u32;
    let y = (baseline - fx(21.0) * height as f32).round() as u32;
    Some((
        x,
        y,
        (fx(1748.0 - left_1080) * height as f32).round() as u32,
        (fx(26.0) * height as f32).round() as u32,
    ))
}

#[cfg(test)]
mod basket_label_tests {
    use super::*;

    #[test]
    fn name_crop_stays_inside_basket_while_quantity_crop_includes_stack_prefix() {
        assert_eq!(basket_label_rect(1920, 1080, 0), Some((1256, 222, 492, 26)));
        assert_eq!(
            basket_quantity_rect(1920, 1080, 0),
            Some((1080, 222, 668, 26))
        );
        assert_eq!(basket_label_rect(1920, 1080, BASKET_ROWS), None);
        assert_eq!(basket_quantity_rect(1920, 1080, BASKET_ROWS), None);
    }

    #[test]
    fn the_scroll_strip_spans_the_pane() {
        // Six columns on a 207.5 pitch from x=76, tile width 190: right edge 1303.5, inset 6
        // each side; vertically from above row 0's cards (199) down to the pane's clip edge.
        assert_eq!(grid_strip(1920, 1080), (70, 193, 1240, 790));
    }

    /// The pane the tracker profiles is the pane the game clips to, measured on a live 1080p
    /// frame: card content ends between y=982 and y=985, and a label band at 1009 is never
    /// rendered. Stopping the strip at the third row's label (841) hid the fourth band the
    /// pane does render at some scroll positions, and a band the locator cannot see is a row
    /// of items that never gets a chip.
    #[test]
    fn the_strip_reaches_the_panes_clip_edge() {
        let (_x, y, _w, h) = grid_strip(1920, 1080);
        assert_eq!(
            y + h,
            (PANE_BOTTOM * 1080.0).round() as u32,
            "the strip stops where the game stops drawing"
        );
    }
}
