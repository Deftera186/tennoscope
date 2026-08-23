//! Kiosk screen geometry, calibrated against a labelled 1920x1080 capture
//! (`tests/fixtures/kiosk/kiosk-open.png`).
//!
//! Same convention as `reward_ocr.rs`: Warframe scales its HUD with window *height* and centres
//! it horizontally, so every constant below is a fraction of height -- horizontal positions as a
//! signed offset of that fraction from the window's horizontal centre. Fractions of width would
//! agree at 16:9 and silently drift everywhere else.
//!
//! Measured on the fixture (2026-08-23): six tile columns on a 206px pitch starting at x=68,
//! three thumbnail rows (tops y=197/423/640), the item label sitting below each thumbnail, a
//! sell-basket list whose ducat numbers right-align at x=1792, and the TOTAL row's digits
//! (16px tall, baseline y=877) with its gold ducat glyph at x=1729..1749.
//!
//! Consumed by recognition and rendering tasks; dead until then.
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

const COL_PITCH: f32 = fx(206.0);
const TILE_W: f32 = fx(198.0);
const GRID_LEFT: f32 = fcx(68.0);
const ROW_TOPS: [f32; 3] = [fx(197.0), fx(423.0), fx(640.0)];
/// Label band top relative to its row's thumbnail top.
const LABEL_DY: f32 = fx(142.0);
/// Label band height: two ~17px lines plus breathing room for the OCR crop.
const LABEL_H: f32 = fx(46.0);
/// Chip inset from the tile's right edge, and rise above the thumbnail top.
const CHIP_INSET: f32 = fx(6.0);
const CHIP_RISE: f32 = fx(2.0);

const BASKET_Y0: f32 = fx(224.0);
const BASKET_PITCH: f32 = fx(38.7);
/// Baseline offset within a basket row.
const BASKET_BASELINE_DY: f32 = fx(19.0);
/// Left edge where our `[icon][digits]` pair starts in a basket row.
const ROW_PAIR_LEFT: f32 = fcx(1814.0);

/// Right edge of our `[icon][digits]` pair in the TOTAL row (12px left of the gold glyph).
const TOTAL_PAIR_RIGHT: f32 = fcx(1717.0);
const TOTAL_BASELINE: f32 = fx(877.0);

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

/// A basket row's pair origin and digit baseline, in pixels: `(left_x, baseline_y)`.
pub fn basket_row_pair(width: u32, height: u32, row: usize) -> Option<(f32, f32)> {
    if row >= BASKET_ROWS {
        return None;
    }
    let left = width as f32 / 2.0 + ROW_PAIR_LEFT * height as f32;
    let baseline = (BASKET_Y0 + BASKET_PITCH * row as f32 + BASKET_BASELINE_DY) * height as f32;
    Some((left, baseline))
}

/// The TOTAL row pair's right edge and digit baseline, in pixels: `(right_x, baseline_y)`.
pub fn total_row_pair(width: u32, height: u32) -> (f32, f32) {
    (
        width as f32 / 2.0 + TOTAL_PAIR_RIGHT * height as f32,
        TOTAL_BASELINE * height as f32,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 1920x1080 calibration must reproduce the fixture's measured pixels exactly; these
    /// numbers were read off the labelled capture, and any drift here is a drifted overlay.
    #[test]
    fn the_1920x1080_calibration_reproduces_the_fixture() {
        assert_eq!(
            grid_label_rect(1920, 1080, 0, 0),
            Some((68, 339, 198, 46)),
            "row 0 col 0 label band"
        );
        assert_eq!(
            grid_label_rect(1920, 1080, 5, 2),
            Some((68 + 206 * 5, 782, 198, 46)),
            "last column, last row"
        );
        let (x, y) = tile_anchor(1920, 1080, 0, 0).unwrap();
        assert_eq!((x.round(), y.round()), (260.0, 195.0));
        let (left, baseline) = basket_row_pair(1920, 1080, 0).unwrap();
        assert_eq!((left.round(), baseline.round()), (1814.0, 243.0));
        let (right, base) = total_row_pair(1920, 1080);
        assert_eq!((right.round(), base.round()), (1717.0, 877.0));
    }

    /// Height fractions keep every pitch proportional on a smaller 16:9 window.
    #[test]
    fn a_smaller_16x9_window_scales_by_height_and_stays_centred() {
        let (x, _, w, _) = grid_label_rect(1280, 720, 1, 1).unwrap();
        // left = 640 + (-892 + 206) * 720/1080 = 640 - 457.33
        assert_eq!(
            x,
            (640.0f32 + (-892.0 + 206.0) * 720.0 / 1080.0).round() as u32,
            "centre-relative offset scales with height"
        );
        assert_eq!(w, (198.0f32 * 720.0 / 1080.0).round() as u32);
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
