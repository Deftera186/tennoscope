//! Ask one cheap question of the kiosk grid without reading it: where is it relative to the
//! anchor?
//!
//! A full recognition pass costs one tesseract crop per visible slot -- fine once a second,
//! impossible thirty. But a capture through grim costs ~25ms, so each tick compares the strip's
//! row luma profile against the anchor frame's and streams the answer: a confident shift rides
//! to the frontend as an offset (the chips follow the scroll in real time), a confident zero
//! means the anchor still holds, and no confident answer at all fades until the next settled
//! read re-anchors.
//!
//! The comparison is normalized cross-correlation (a Pearson r per candidate shift), so the
//! peak's value is itself the confidence: structure that moved together peaks near 1.0, two
//! unrelated frames peak near 0.3, and a flat strip has nothing to say at all.

use image::DynamicImage;

/// How far the grid can move between two looks and still be found: a 33ms tick at a hard flick
/// of the wheel moves the list a few dozen rows; anything further is not scrolling.
pub const MAX_SHIFT: i32 = 48;

/// Minimum normalized correlation peak accepted as a measurement. A clean shift peaks above
/// 0.9; two unrelated frames of the same length peak around 0.3 (the max of ~97 noise samples);
/// the floor sits between them, nearer the noise.
pub const MIN_PEAK_RATIO: f32 = 0.5;

/// Normalized cross-correlation of two row profiles: the shift (in rows) that best explains
/// `next` as `prev` moved vertically, if that shift is confident enough to name.
///
/// Positive means content moved down the screen: `next[i]` is `prev[i - dy]`. Sub-pixel shifts
/// are not attempted -- the caller only needs to know whether the anchor still holds, and the
/// next settled read is what re-aligns everything.
pub fn estimate_dy(prev: &[f32], next: &[f32], max_shift: i32, min_peak: f32) -> Option<i32> {
    let len = prev.len().min(next.len());
    // Correlation needs rows to spare on both sides of every candidate shift, and a strip that
    // short has no business guessing anyway.
    if (len as i32) < 2 * max_shift + 8 {
        return None;
    }
    let (prev, next) = (&prev[..len], &next[..len]);
    let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
    let (pm, nm) = (mean(prev), mean(next));
    // Zero-mean first: a uniformly bright strip correlates with a uniformly bright strip at
    // every shift, and the strip's own brightness says nothing about where the rows are.
    let centered = |v: &[f32], m: f32| v.iter().map(|x| x - m).collect::<Vec<f32>>();
    let (p, n) = (centered(prev, pm), centered(next, nm));
    let variance = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>();
    // A flat profile (no variance) is the "kiosk closed" reading -- blindness, not zero motion.
    if variance(&p) <= f32::EPSILON * len as f32 || variance(&n) <= f32::EPSILON * len as f32 {
        return None;
    }

    let mut best_shift = 0_i32;
    let mut best_score = f32::NEG_INFINITY;
    for shift in -max_shift..=max_shift {
        // next[i] against prev[i - shift], over the rows both cover.
        let lo = shift.max(0) as usize;
        let hi = (len as i32 + shift).min(len as i32) as usize;
        let mut dot = 0.0_f32;
        let mut left = 0.0_f32;
        let mut right = 0.0_f32;
        for i in lo..hi {
            // In-range by construction (lo/hi were clamped for this shift); signed arithmetic
            // because a negative shift makes `i - shift` exceed usize's comfort.
            let p_at = p[(i as i64 - shift as i64) as usize];
            dot += n[i] * p_at;
            left += p_at * p_at;
            right += n[i] * n[i];
        }
        if hi <= lo {
            continue;
        }
        let score = dot / (left * right).sqrt();
        if score > best_score {
            best_score = score;
            best_shift = shift;
        }
    }
    (best_score >= min_peak).then_some(best_shift)
}

/// Mean luma per row over columns `[x, x + w)` -- the strip the tracker looks at. Rows outside
/// the requested band are not the tracker's business; the caller crops the geometry.
pub fn row_profiles(image: &DynamicImage, x: u32, y: u32, w: u32, h: u32) -> Vec<f32> {
    let luma = image.to_luma8();
    let (width, height) = luma.dimensions();
    let w = w.min(width.saturating_sub(x));
    let h = h.min(height.saturating_sub(y));
    let mut rows = vec![0.0_f32; h as usize];
    for row in 0..h {
        let mut sum = 0_u32;
        for column in 0..w {
            sum += u32::from(luma.get_pixel(x + column, y + row)[0]);
        }
        rows[row as usize] = if w > 0 { sum as f32 / w as f32 } else { 0.0 };
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A strip with structure: bands of varying brightness, like rows of thumbnails and labels.
    fn strip(len: usize) -> Vec<f32> {
        (0..len)
            .map(|row| {
                let band = (row / 13) % 5;
                let base = match band {
                    0 => 20.0,
                    1 => 180.0,
                    2 => 60.0,
                    3 => 200.0,
                    _ => 100.0,
                };
                base + (row % 7) as f32
            })
            .collect()
    }

    #[test]
    fn a_shifted_strip_is_recovered_exactly() {
        let base = strip(400);
        for dy in [17_i32, -9, 0, 48, -48] {
            let next = shift(&base, dy);
            assert_eq!(
                estimate_dy(&base, &next, MAX_SHIFT, MIN_PEAK_RATIO),
                Some(dy),
                "dy={dy}"
            );
        }
    }

    #[test]
    fn flat_strips_are_blind_not_still() {
        let flat = vec![128.0_f32; 300];
        assert_eq!(estimate_dy(&flat, &flat, MAX_SHIFT, MIN_PEAK_RATIO), None);
    }

    #[test]
    fn unrelated_strips_do_not_measure() {
        // Two independent noisy strips: every normalized correlation must land near zero, so
        // the best of ~97 shifts is noise -- below the floor, because a confident wrong dy is
        // the one error this module must never make.
        let left = noise(400, 1);
        let right = noise(400, 2);
        assert_eq!(estimate_dy(&left, &right, MAX_SHIFT, MIN_PEAK_RATIO), None);
    }

    /// Deterministic pseudo-noise (`x = (x * 7919 + 13) % 1000`), so the test is reproducible.
    fn noise(len: usize, seed: u64) -> Vec<f32> {
        let mut state = seed * 2_654_435_761;
        (0..len)
            .map(|_| {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1);
                (state % 1000) as f32
            })
            .collect()
    }

    #[test]
    fn row_profiles_average_luma_per_row() {
        // 2x2 image: one bright row, one dark row.
        let mut image = image::GrayImage::new(2, 2);
        for x in 0..2 {
            image.put_pixel(x, 0, image::Luma([200]));
            image.put_pixel(x, 1, image::Luma([100]));
        }
        let dynamic = DynamicImage::ImageLuma8(image);
        assert_eq!(row_profiles(&dynamic, 0, 0, 2, 2), vec![200.0, 100.0]);
        // Out-of-band requests clip rather than panic.
        assert_eq!(row_profiles(&dynamic, 1, 1, 99, 99), vec![100.0]);
    }

    /// `new[i] = old[i - dy]` with wrap-around at the edges, so lengths always match.
    fn shift(rows: &[f32], dy: i32) -> Vec<f32> {
        (0..rows.len())
            .map(|i| {
                let source = (i as i64 - dy as i64).rem_euclid(rows.len() as i64) as usize;
                rows[source]
            })
            .collect()
    }
}
