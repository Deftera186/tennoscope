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
//! three card rows on an exact 222px pitch (tops y=199/421/643 -- confirmed by count-badge
//! centres landing at tile_left+20, tile_top+17.5 in all six columns and three rows), the item
//! label band below each thumbnail, a sell-basket list whose ducat digits sit on baselines
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
/// Grid rows visible without scrolling.
pub const GRID_ROWS: usize = 3;
/// Basket rows the pane can show at once.
pub const BASKET_ROWS: usize = 8;

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
const ROW_TOPS: [f32; 3] = [fx(199.0), fx(421.0), fx(643.0)];
/// Label band top relative to its row's card top.
const LABEL_DY: f32 = fx(144.0);
/// Label band height: two ~17px lines plus breathing room for the OCR crop.
const LABEL_H: f32 = fx(46.0);
/// Chip inset from the tile's right edge, and rise above the card top.
const CHIP_INSET: f32 = fx(6.0);
const CHIP_RISE: f32 = fx(2.0);

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

/// The OCR crop over one tile's label: `(x, y, w, h)` in pixels, or `None` past the grid.
pub fn grid_label_rect(
    width: u32,
    height: u32,
    col: usize,
    row: usize,
) -> Option<(u32, u32, u32, u32)> {
    if col >= GRID_COLS || row >= GRID_ROWS {
        return None;
    }
    let x = col_left(width, height, col).round() as u32;
    let y = (ROW_TOPS[row] * height as f32 + LABEL_DY * height as f32).round() as u32;
    Some((
        x,
        y,
        (TILE_W * height as f32).round() as u32,
        (LABEL_H * height as f32).round() as u32,
    ))
}

/// Where a grid chip's top-right corner sits, in pixels.
pub fn tile_anchor(width: u32, height: u32, col: usize, row: usize) -> Option<(f32, f32)> {
    if col >= GRID_COLS || row >= GRID_ROWS {
        return None;
    }
    let right = col_left(width, height, col) + TILE_W * height as f32;
    let top = ROW_TOPS[row] * height as f32 - CHIP_RISE * height as f32;
    Some((right - CHIP_INSET * height as f32, top))
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

/// The grid pane region the scroll tracker profiles: every column's width, from just above the
/// first thumbnail row to just below the last label band, `(x, y, w, h)` in pixels.
///
/// The pane, not the window: the basket beside it never moves, and rows outside it (title bar,
/// navigation) carry no scroll information -- including them only dilutes the correlation.
pub fn grid_strip(width: u32, height: u32) -> (u32, u32, u32, u32) {
    let left = col_left(width, height, 0) - fx(6.0) * height as f32;
    let right = col_left(width, height, GRID_COLS - 1) + (TILE_W + fx(6.0)) * height as f32;
    let top = ROW_TOPS[0] * height as f32 - fx(6.0) * height as f32;
    let bottom = (ROW_TOPS[GRID_ROWS - 1] + LABEL_DY + LABEL_H + fx(8.0)) * height as f32;
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
            grid_label_rect(1920, 1080, 0, 0),
            Some((76, 343, 190, 46)),
            "row 0 col 0 label band"
        );
        assert_eq!(
            grid_label_rect(1920, 1080, 5, 2),
            Some((1114, 787, 190, 46)),
            "last column, last row"
        );
        let (x, y) = tile_anchor(1920, 1080, 0, 0).unwrap();
        assert_eq!((x.round(), y.round()), (260.0, 197.0));
        // Chip anchors track the measured card corners, not the assumed ones.
        let (x5, y5) = tile_anchor(1920, 1080, 5, 2).unwrap();
        assert_eq!((x5.round(), y5.round()), (1298.0, 641.0));
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
        let (x, _, w, _) = grid_label_rect(1280, 720, 1, 1).unwrap();
        // left = 640 + (76 - 960 + 207.5) * 720/1080
        assert_eq!(
            x,
            (640.0f32 + (76.0 - 960.0 + 207.5) * 720.0 / 1080.0).round() as u32,
            "centre-relative offset scales with height"
        );
        assert_eq!(w, (190.0f32 * 720.0 / 1080.0).round() as u32);
    }

    /// Past-the-grid slots have no rectangle to read.
    #[test]
    fn out_of_range_slots_are_none() {
        assert_eq!(grid_label_rect(1920, 1080, 6, 0), None);
        assert_eq!(grid_label_rect(1920, 1080, 0, 3), None);
        assert_eq!(tile_anchor(1920, 1080, 6, 0), None);
        assert_eq!(basket_row_pair(1920, 1080, BASKET_ROWS), None);
    }
}

/// The OCR crop over one basket row's item name.
///
/// The pane starts near x=1256 and its item names end well before the ducat column: the game
/// draws the ducat value right-aligned at x<=1792 and the digits begin around x=1758, so the
/// crop stops at 1748 -- a trailing "100" read into the text costs every match an edit distance
/// it does not have to spend. Vertically the band hugs the measured digit baseline.
pub fn basket_label_rect(width: u32, height: u32, row: usize) -> Option<(u32, u32, u32, u32)> {
    if row >= BASKET_ROWS {
        return None;
    }
    let (_, baseline) = basket_row_pair(width, height, row)?;
    let x = (width as f32 / 2.0 + fcx(1256.0) * height as f32).round() as u32;
    let y = (baseline - fx(21.0) * height as f32).round() as u32;
    Some((
        x,
        y,
        (fx(1748.0 - 1256.0) * height as f32).round() as u32,
        (fx(26.0) * height as f32).round() as u32,
    ))
}

#[cfg(test)]
mod basket_label_tests {
    use super::*;

    #[test]
    fn basket_label_band_hugs_the_baseline_and_stops_before_the_ducat_column() {
        assert_eq!(basket_label_rect(1920, 1080, 0), Some((1256, 222, 492, 26)));
        assert_eq!(basket_label_rect(1920, 1080, BASKET_ROWS), None);
    }

    #[test]
    fn the_scroll_strip_spans_the_pane() {
        // Six columns on a 207.5 pitch from x=76, tile width 190: right edge 1303.5, inset 6
        // each side; vertically from above row 0's cards (199) to below row 2's label band
        // (643 + 144 + 46 + 8).
        assert_eq!(grid_strip(1920, 1080), (70, 193, 1240, 648));
    }
}
