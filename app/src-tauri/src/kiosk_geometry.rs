//! Kiosk screen geometry, calibrated against a labelled 1920x1080 capture
//! (`tests/fixtures/kiosk/kiosk-open.png`).
//!
//! Same convention as `reward_ocr.rs`: Warframe scales its HUD with window *height* and centres
//! it horizontally, so every constant below is a fraction of height -- horizontal positions as a
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
//! calibration of record -- fixture-exact, test-asserted -- and the TypeScript constants are
//! mirrors of these, not a second measurement.
#![allow(dead_code)]

/// Grid columns across the kiosk.
pub const GRID_COLS: usize = 6;
/// Maximum card bands visible in the clipped pane while scrolling. Three fit at rest; a fourth
/// enters through the bottom edge before the first leaves through the top.
pub const GRID_ROWS: usize = 4;
/// Basket rows the pane can show at once.
pub const BASKET_ROWS: usize = 8;

/// Left edge of the basket pane at 1920x1080. Name OCR must not cross into grid column six.
const BASKET_NAME_LEFT_1080P: f32 = 1256.0;

const CAL: f32 = 1080.0;

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
/// band that would land at y=1009 is never rendered -- the game clips before drawing it.
const PANE_BOTTOM: f32 = fx(983.0);
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
/// because three-line labels stack upward out of it -- *Styanax Prime Neuroptics Blueprint*
/// renders its first line at card top +124, 20px above the old crop, and went unpriced for
/// it (measured live, 2026-08-23). The bottom edge stays where it was.
const LABEL_DY: f32 = fx(122.0);
/// Label crop height: the two-line geometry's bottom edge (+190) plus the upward growth.
const LABEL_H: f32 = fx(68.0);
/// Each card border stroke's position in design pixels, measured off a live 1080p capture
/// (the lit columns were 264/265, 471/472, 679/680, 1094/1095, 1302/1303; col3 interpolated
/// on the 207.6px pitch the others confirm). A chip's right edge sits on these, flush with
/// its card's top-right corner -- see `tile_anchor`.
const COL_RIGHT_1080: [f32; 6] = [264.0, 471.5, 679.5, 887.0, 1094.5, 1302.5];

/// Digit baseline of the first basket row; rows follow on a 38 1/3px pitch.
const BASKET_FIRST_BASELINE: f32 = fx(243.0);
const BASKET_PITCH: f32 = fx(115.0 / 3.0);
/// Right edge where our `[icon][digits]` pair ends in a basket row: 9px left of the game's
/// earliest digit (the pane border sits at x~1814, so a left anchor there overflows).
const ROW_PAIR_RIGHT: f32 = fcx(1750.0);

/// Right edge of our `[icon][digits]` pair in the TOTAL row (13px left of the gold glyph,
/// whose body starts at x~1730).
const TOTAL_PAIR_RIGHT: f32 = fcx(1717.0);
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

/// Where a grid chip's top-right corner sits, in pixels.
///
/// The corner is the card border stroke's own position, measured per column off a live
/// capture (2026-08-24): the 207.5px column pitch rasterizes each border on a different
/// half-pixel, and a pitch formula rounded once per chip drifted up to 5px by the last
/// column -- the player asked for pixel-for-pixel and the formula could not deliver it.
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
/// bar, navigation) carry no scroll information -- including them only dilutes the correlation.
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The 1920x1080 calibration must reproduce the fixture's measured pixels exactly; these
    /// numbers were measured off the capture structurally -- count-badge centres at
    /// (tile_left+20, tile_top+17.5) on a 207.5x222 grid, card borders at
    /// x=265.95+207.55k, ducat digit baselines -- and any drift here is a drifted overlay.
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
        assert_eq!((right.round(), base.round()), (1717.0, 875.0));
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
    /// the two-line box, so the crop grows upward too -- its bottom edge stays on the two-line
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
