//! Ask two cheap questions of the kiosk grid without reading it: is it moving, and where are
//! its label rows right now?
//!
//! A full recognition pass costs one tesseract crop per visible slot -- fine once a second,
//! impossible thirty. But a capture through grim costs ~25ms, so each tick profiles the strip
//! and answers both questions from one vector.
//!
//! The profile is the count of near-white pixels per row. Label glyphs are the whitest thing
//! the grid pane draws (measured 2026-08-23: label rows score 200-330, count badges 8-54,
//! gold thumbnails rarely cross 100), so text rows stand out of the profile sharply enough to
//! locate directly -- which is what makes the settled read self-locating: the topmost label
//! band names the grid's offset at ANY scroll position, list ends and clipped rows included,
//! with no anchor and no accumulated drift to lose.
//!
//! Motion is measured frame to frame, never against an older anchor. A static screen at any
//! offset is static; only the pixels that actually moved between two looks count as motion.
//! That distinction is load-bearing: an anchor-relative "the grid is 32px from where it was"
//! verdict on a still screen once read as motion forever, blocked the close streak, and left
//! a session that neither published nor died (2026-08-23, the undead minute).
//!
//! The frame-to-frame comparison is normalized cross-correlation (a Pearson r per candidate
//! shift over the rows both frames share), so the peak's value is itself the confidence. The
//! search is deliberately short -- two 60ms-apart looks rarely differ by more than a couple
//! hundred rows -- because short shifts are unambiguous: the grid's row pitch is 222, and a
//! search that reaches past half that can mistake one row for the next. Where an older design
//! searched half the strip and once crowned a false peak 80px away from the truth, this one
//! refuses to answer rather than guess far.

use image::DynamicImage;

/// How far the grid may move between two looks and still be found. A 60ms tick at a hard
/// flick crosses ~120px; 180 leaves headroom without reaching the 222 row pitch where rows
/// start impersonating each other.
pub const FRAME_DELTA_MAX: i32 = 180;

/// Minimum normalized correlation peak accepted as a measurement. A clean shift peaks above
/// 0.9; two unrelated frames peak around 0.3; the floor sits between them, nearer the noise.
pub const MIN_PEAK_RATIO: f32 = 0.5;

/// How much the winning shift must beat the best competing shift by. Structure that truly
/// moved has one sharp peak; periodic or noisy profiles produce rivals, and a tie is exactly
/// the confident-wrong-answer this module must never emit.
pub const MIN_PEAK_MARGIN: f32 = 0.08;

/// Minimum ratio between the best band window and the median one for the profile to hold a
/// grid at all. Live captures measured 11.8, 24.8 and 29.5; a pane with no label rows has no
/// peak to speak of, so the floor sits far below the signal and far above nothing.
const MIN_LABEL_CONTRAST: f32 = 3.0;
/// How bright the best band window must be in absolute profile counts. The contrast gate
/// alone is not enough: a pane of bare count badges (measured 8-54 per row against a floor
/// of ~2) concentrates into a window that towers over the median without containing a single
/// line of text. Real label windows average well over 100 (22 text rows at 250 inside 46).
const MIN_BAND_MEAN: f32 = 40.0;
/// How bright a band must be, against the best band in the same pane, to count as rendered.
/// Measured present bands scored 87 to 108 in the same frame; a band outside the pane's clip
/// scores near the floor.
const BAND_PRESENT_RATIO: f32 = 0.35;

/// How bright a folded row must be, against the loudest row of the fold, to count as glyph
/// structure when locating the grid. Glyph cores measured 130-330 white pixels per column
/// band on live captures, anti-aliased fringes under 85, floor under 5; the fraction splits
/// core from fringe. Relative, not absolute, because row counts scale with capture width.
const LOUD_ROW_FRACTION: f32 = 0.4;

/// Normalized cross-correlation of two row profiles: the shift (in rows) that best explains
/// `next` as `prev` moved vertically, if that shift is confident enough to name.
///
/// Positive means content moved down the screen: `next[i]` is `prev[i - dy]`. Each shift is
/// scored only over the rows both frames share, so edge effects cannot dilute the true peak,
/// and the winner must clear both the absolute floor and a margin over the runner-up.
pub fn estimate_dy(prev: &[f32], next: &[f32], max_shift: i32, min_peak: f32) -> Option<i32> {
    let len = prev.len().min(next.len());
    // The search never reaches past half the strip: every candidate shift needs rows to spare
    // on both sides, and a strip that cannot afford that has no business guessing.
    let range = max_shift.min(len as i32 / 2 - 4);
    if range < 4 {
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

    // Best shift and the best shift that is a different peak (>= 8 rows away), so the margin
    // compares peaks, not a winner against its own shoulder.
    let (mut best_shift, mut best_score, mut rival_score) =
        (0_i32, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for shift in -range..=range {
        // next[i] against prev[i - shift], over the rows both cover.
        let lo = shift.max(0) as usize;
        let hi = (len as i32 + shift).min(len as i32) as usize;
        if hi <= lo {
            continue;
        }
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
        let score = dot / (left * right).sqrt();
        if score > best_score {
            if (shift - best_shift).abs() >= 8 {
                rival_score = best_score;
            }
            best_score = score;
            best_shift = shift;
        } else if score > rival_score && (shift - best_shift).abs() >= 8 {
            rival_score = score;
        }
    }
    (best_score >= min_peak && best_score - rival_score >= MIN_PEAK_MARGIN).then_some(best_shift)
}

/// The grid's offset, named by its own label rows.
///
/// The grid repeats on `pitch`, so the profile is folded over it: every row votes in exactly
/// one phase bucket, and the `band`-wide window with the brightest mean names where the label
/// rows sit. Folding is what makes this work at any scroll position -- three or four bands all
/// vote for the same phase, so the answer gets stronger the further the player scrolls, where
/// run-length band detection got weaker. (Its predecessor demanded a 14-row contiguous run
/// above a brightness floor; real label lines run 11, so it returned `None` on every live
/// capture and the overlay never appeared.)
///
/// The phase alone is not the answer: it names where bands *would* be, and the pane clips, so
/// the offset returned is the topmost band the pane actually renders. That puts read row 0 on
/// a real label instead of past the pane's edge.
///
/// Precision: a 46-row band around ~32 rows of text can slide ~14 rows and still contain
/// every glyph row, and within that containment slack the mean window score cannot name a
/// winner -- on synthetic panes it ties exactly (and answered whichever edge the scan order
/// favored), while on real captures texture tilts it into a shallow monotone slope whose
/// argmax sat at the slack's low edge, reporting an unscrolled grid as 8 rows scrolled
/// (2026-08-23). So the phase is named by counting rows that are DECISIVELY bright
/// (`LOUD_ROW_FRACTION`): every window fully containing the glyph cores holds exactly the
/// same count -- an integer tie -- and every window that clips one does not. The midpoint of
/// that tied run, walked circularly because the run straddles the fold boundary more often
/// than not, is the least-wrong point in a range the data cannot narrow.
///
/// `y0` is the absolute row the profile starts at, `first_label_top` the absolute row of the
/// unscrolled grid's first label band. `None` means no grid in this profile -- an animation
/// frame or an emptied filter, not a closed kiosk.
pub fn label_offset(
    profile: &[f32],
    y0: i32,
    first_label_top: i32,
    pitch: i32,
    band: i32,
) -> Option<i32> {
    if profile.is_empty() || pitch <= 0 || band <= 0 || band >= pitch {
        return None;
    }
    let (pitch_n, band_n) = (pitch as usize, band as usize);
    // Fold: every row votes in one bucket. Means, not sums -- the buckets hold unequal row
    // counts when the strip is not a whole number of pitches.
    let mut sums = vec![0.0_f32; pitch_n];
    let mut counts = vec![0.0_f32; pitch_n];
    for (row, value) in profile.iter().enumerate() {
        let bucket = (row as i32 + y0 - first_label_top).rem_euclid(pitch) as usize;
        sums[bucket] += value;
        counts[bucket] += 1.0;
    }
    let folded: Vec<f32> = sums
        .iter()
        .zip(&counts)
        .map(|(sum, count)| if *count > 0.0 { sum / count } else { 0.0 })
        .collect();
    // The brightest band-wide window, by mean -- this gates presence but localizes nothing
    // (see the precision note for why).
    let mean_at = |start: usize| -> f32 {
        (0..band_n)
            .map(|offset| folded[(start + offset) % pitch_n])
            .sum::<f32>()
            / band_n as f32
    };
    let means: Vec<f32> = (0..pitch_n).map(mean_at).collect();
    let mut sorted = means.clone();
    sorted.sort_by(f32::total_cmp);
    let median = sorted[sorted.len() / 2];
    let peak = means.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    if peak < median.max(f32::EPSILON) * MIN_LABEL_CONTRAST || peak < MIN_BAND_MEAN {
        return None;
    }
    // The phase, by count of decisively-bright rows: integer ties across the containment
    // slack, midpoint of the tied run as the answer.
    let loudest = folded.iter().copied().fold(0.0_f32, f32::max);
    let loud_at = |start: usize| -> i32 {
        (0..band_n)
            .filter(|offset| folded[(start + offset) % pitch_n] >= loudest * LOUD_ROW_FRACTION)
            .count() as i32
    };
    let counts: Vec<i32> = (0..pitch_n).map(loud_at).collect();
    let tallest = counts.iter().copied().max().unwrap_or(0);
    let (mut run_start, mut walked) = (
        counts
            .iter()
            .position(|&count| count == tallest)
            .unwrap_or(0),
        0_usize,
    );
    while walked < pitch_n && counts[(run_start + pitch_n - 1) % pitch_n] == tallest {
        run_start = (run_start + pitch_n - 1) % pitch_n;
        walked += 1;
    }
    let mut run_len = 1_usize;
    while run_len < pitch_n && counts[(run_start + run_len) % pitch_n] == tallest {
        run_len += 1;
    }
    let phase = (run_start + (run_len - 1) / 2) % pitch_n;
    // Which bands at that phase does the pane actually render? Score each against the
    // unfolded profile; the pane's clip edge shows up as a band that scores near nothing.
    let band_top = |k: i32| first_label_top + phase as i32 + pitch * k;
    let first_k = (y0 - band_top(0)).div_euclid(pitch);
    let mut bands = Vec::new();
    for k in first_k..first_k + profile.len() as i32 / pitch + 2 {
        let top = band_top(k);
        let start = top - y0;
        if start < 0 || start + band > profile.len() as i32 {
            continue;
        }
        let score = profile[start as usize..(start + band) as usize]
            .iter()
            .sum::<f32>()
            / band as f32;
        bands.push((top, score));
    }
    let best = bands.iter().map(|(_, score)| *score).fold(0.0, f32::max);
    bands
        .into_iter()
        .find(|(_, score)| *score >= best * BAND_PRESENT_RATIO)
        .map(|(top, _)| top - first_label_top)
}
/// Near-white pixel count per row over columns `[x, x + w)` -- the strip the tracker looks
/// at. Rows outside the requested band are not the tracker's business; the caller crops the
/// geometry. Label glyphs are the whitest structure in the pane, so text rows spike in this
/// profile while thumbnails and backgrounds stay near the floor.
pub fn row_profiles(image: &DynamicImage, x: u32, y: u32, w: u32, h: u32) -> Vec<f32> {
    let luma = image.to_luma8();
    let (width, height) = luma.dimensions();
    let w = w.min(width.saturating_sub(x));
    let h = h.min(height.saturating_sub(y));
    let mut rows = vec![0.0_f32; h as usize];
    for row in 0..h {
        let mut sum = 0_u32;
        for column in 0..w {
            if luma.get_pixel(x + column, y + row)[0] > 225 {
                sum += 1;
            }
        }
        rows[row as usize] = sum as f32;
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A strip with label-like structure at 1080p scale: bands every 222 rows over a textured
    /// floor, like the real pane -- each band's two text lines carry their own noise-like
    /// pattern and tone, because real rows show different items. The texture is load-bearing:
    /// flat floors and ramp patterns correlate positively with each other at every shift
    /// (Pearson only sees covariance), which manufactures alias peaks no implementation could
    /// refuse. A uniform floor would make wrong shifts score ~0.95 against a true 1.0.
    fn label_strip(len: usize, first_label: usize) -> Vec<f32> {
        // Per-band glyph tones, the way some labels catch more light than others.
        const TONES: [f32; 4] = [252.0, 188.0, 270.0, 158.0];
        // One round of xorshift-multiply hashing; enough noise to stand in for glyphs.
        let pattern = |j: usize, band: usize, line: u64| -> f32 {
            let mut x = (j as u64).wrapping_mul(37_476_139_368)
                ^ (band as u64).wrapping_mul(668_265_263)
                ^ line.wrapping_mul(2_246_822_519);
            x ^= x >> 13;
            x = x.wrapping_mul(1_274_126_177);
            ((x ^ (x >> 16)) % 97) as f32 / 97.0
        };
        let mut rows = vec![0.0_f32; len];
        for (i, row) in rows.iter_mut().enumerate() {
            // Thumbnails, borders, gradients: the pane's floor is textured, not flat.
            *row = 4.0 + ((i * 7919) % 23) as f32 * 1.6;
        }
        let mut label = first_label;
        let mut band = 0_usize;
        while label + 39 <= len {
            let tone = TONES[band % TONES.len()];
            for (j, row) in (label..label + 17).enumerate() {
                rows[row] = tone * (0.30 + 0.70 * pattern(j, band, 1));
            }
            for (j, row) in (label + 22..label + 39).enumerate() {
                if row < len {
                    rows[row] = tone * (0.26 + 0.70 * pattern(j, band, 2));
                }
            }
            // a count badge above the thumbnail: bright but flat and weak
            if label >= 127 + 14 {
                for row in &mut rows[(label - 127)..(label - 113).min(len)] {
                    *row = 30.0;
                }
            }
            band += 1;
            label += 222;
        }
        rows
    }

    #[test]
    fn a_shifted_strip_is_recovered_exactly() {
        let base = label_strip(648, 150);
        for dy in [17_i32, -9, 0, 48, -48] {
            let next = shift(&base, dy);
            assert_eq!(
                estimate_dy(&base, &next, FRAME_DELTA_MAX, MIN_PEAK_RATIO),
                Some(dy),
                "dy={dy}"
            );
        }
    }

    /// Two looks 142 rows apart -- a fast flick between 60ms ticks -- still measure: the
    /// frame-to-frame range exists so real motion is never blindness.
    #[test]
    fn a_fast_flick_between_looks_is_measured() {
        let base = label_strip(648, 150);
        for dy in [120_i32, -142, 180, -64] {
            let next = shift(&base, dy);
            assert_eq!(
                estimate_dy(&base, &next, FRAME_DELTA_MAX, MIN_PEAK_RATIO),
                Some(dy),
                "dy={dy}"
            );
        }
    }

    /// `new[i] = old[i - dy]` where the source row exists, and fresh floor where it does not:
    /// real scrolling moves content in and out of the pane, it does not wrap around. (A
    /// wrap-around helper on a periodic strip manufactures alias peaks at dy±222 that no
    /// correlation could distinguish from the truth.)
    fn shift(rows: &[f32], dy: i32) -> Vec<f32> {
        let len = rows.len() as i64;
        (0..rows.len())
            .map(|i| {
                let source = i as i64 - dy as i64;
                if source < 0 || source >= len {
                    4.0 + ((i * 10_4729) % 23) as f32 * 1.6
                } else {
                    rows[source as usize]
                }
            })
            .collect()
    }

    /// A whole pitch jumped: rows impersonate each other past half a pitch, so this may
    /// honestly fail to see the jump -- but whatever it names must be small. A confident far
    /// shift here would fling the chips; a transient "still" only costs one look before the
    /// settled read re-anchors the view absolutely.
    #[test]
    fn a_whole_pitch_jump_never_names_a_far_shift() {
        let base = label_strip(648, 150);
        let next = shift(&base, 222);
        match estimate_dy(&base, &next, FRAME_DELTA_MAX, MIN_PEAK_RATIO) {
            None => {}
            Some(dy) => assert!(
                dy.abs() < 8,
                "a whole-row jump was named as dy={dy}; that is a guess, not a measurement"
            ),
        }
    }

    #[test]
    fn unrelated_strips_do_not_measure() {
        // Two independent noisy strips: every normalized correlation must land near zero, so
        // the best of ~97 shifts is noise -- below the floor, because a confident wrong dy is
        // the one error this module must never make.
        let left = noise(400, 1);
        let right = noise(400, 2);
        assert_eq!(
            estimate_dy(&left, &right, FRAME_DELTA_MAX, MIN_PEAK_RATIO),
            None
        );
    }

    /// A profile whose structure repeats on a period the search can reach: every copy of the
    /// period scores identically, so no shift carries a margin and the tracker refuses rather
    /// than pick one. (The copies must sit inside the search range for the tie to exist at
    /// all -- a half-length alias is structurally unreachable because the range never
    /// exceeds half the strip.)
    #[test]
    fn an_ambiguous_peak_is_refused_even_when_tall() {
        let mut cell = vec![2.0_f32; 100];
        for row in &mut cell[30..47] {
            *row = 250.0;
        }
        for row in &mut cell[52..68] {
            *row = 240.0;
        }
        let periodic: Vec<f32> = cell.iter().cycle().take(300).copied().collect();
        assert_eq!(
            estimate_dy(&periodic, &periodic, 110, MIN_PEAK_RATIO),
            None,
            "a tie between periods 0 and +/-100 is not a measurement"
        );
    }

    #[test]
    fn flat_strips_are_blind_not_still() {
        let flat = vec![128.0_f32; 300];
        assert_eq!(
            estimate_dy(&flat, &flat, FRAME_DELTA_MAX, MIN_PEAK_RATIO),
            None
        );
    }

    // --- the locator ---

    const PITCH: i32 = 222;
    const BAND: i32 = 46;
    const STRIP: usize = 790;

    /// A synthetic pane: label bands on the grid's period, each two bright text lines inside a
    /// 46-row band, with faint badge rows between them. `present` says which bands the pane
    /// renders -- the ones the player can actually see.
    fn pane(len: usize, phase: i32, present: &[bool]) -> Vec<f32> {
        let mut rows = vec![2.0_f32; len];
        for (k, rendered) in present.iter().enumerate() {
            if !rendered {
                continue;
            }
            // band top in strip coordinates: the calibration band (343) at `phase`, minus the
            // strip's own origin (193)
            let top = 343 + PITCH * k as i32 + phase - 193;
            for line in [6_i32, 27] {
                for at in top + line..top + line + 11 {
                    if at >= 0 && (at as usize) < len {
                        rows[at as usize] = 250.0;
                    }
                }
            }
            // a count badge above the thumbnail: bright, but nothing like text
            let badge = top - 120;
            for at in badge..badge + 14 {
                if at >= 0 && (at as usize) < len {
                    rows[at as usize] = 30.0;
                }
            }
        }
        rows
    }

    /// The phases here span where live sessions actually sat on 2026-08-23 (0, -142, -111)
    /// plus both ends of a pitch, and the anchor is the topmost band whose text lies inside
    /// the strip -- a band scrolled mostly past the pane's top edge cannot be read by anybody.
    ///
    /// The assertions are exact, not containment: the pane's folded profile is perfectly flat
    /// across the whole containment plateau, so every algorithm gets the same tie to break,
    /// and the tie-break is part of the contract. (Textured fixtures cannot pin this -- the
    /// fold averages their floor noise into near-ties whose winner is noise -- which is
    /// precisely how the real grid's unscrolled frame came to publish -8.)
    #[test]
    fn the_grid_offset_is_named_at_any_scroll_position() {
        for phase in [0_i32, -33, -111, -142, -200] {
            let strip = pane(STRIP, phase, &[true, true, true]);
            // The first band's text clears the strip top (y=193) only when its top (343 +
            // phase) sits at or above 187; otherwise the anchor is the next band down.
            let anchor = if phase >= -156 { phase } else { phase + PITCH };
            // Glyph rows span band rows 6..37, so windows starting anywhere in -8..=6
            // contain them all; fifteen tied starts, midpoint -8+7 = -1.
            assert_eq!(
                label_offset(&strip, 193, 343, PITCH, BAND),
                Some(anchor - 1)
            );
        }
    }

    /// Four bands render when the phase pushes one in from the bottom; row 0 must anchor on
    /// the topmost rendered band, so all read rows land on real labels instead of one landing
    /// past the end.
    #[test]
    fn the_topmost_rendered_band_anchors_the_read() {
        let strip = pane(STRIP, -149, &[true, true, true, true]);
        assert_eq!(label_offset(&strip, 193, 343, PITCH, BAND), Some(-150));
    }

    /// A phase whose first band is above the pane's clip edge: that band is not rendered, and
    /// anchoring row 0 on it would read the header. The offset names the first band that is.
    #[test]
    fn a_band_the_pane_does_not_render_is_not_the_anchor() {
        let strip = pane(STRIP, -20, &[false, true, true, true]);
        assert_eq!(
            label_offset(&strip, 193, 343, PITCH, BAND),
            Some(-20 + PITCH - 1)
        );
    }

    /// No labels anywhere -- an animation frame, or a grid filtered down to nothing. The
    /// locator says so instead of naming an offset, and the caller skips the look. (It does
    /// NOT mean the kiosk closed: EE.log decides that.)
    #[test]
    fn a_pane_without_labels_locates_nothing() {
        assert_eq!(label_offset(&vec![2.0; STRIP], 193, 343, PITCH, BAND), None);
        let mut badges = vec![2.0_f32; STRIP];
        for badge in &mut badges[120..140] {
            *badge = 30.0;
        }
        assert_eq!(label_offset(&badges, 193, 343, PITCH, BAND), None);
    }

    /// Structure with no periodicity at the grid's pitch is not a grid.
    #[test]
    fn noise_does_not_locate() {
        assert_eq!(label_offset(&noise(STRIP, 3), 193, 343, PITCH, BAND), None);
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
    fn row_profiles_count_near_white_pixels_per_row() {
        // 2x2 image: one all-white row, one mid-gray row.
        let mut image = image::GrayImage::new(2, 2);
        for x in 0..2 {
            image.put_pixel(x, 0, image::Luma([240]));
            image.put_pixel(x, 1, image::Luma([100]));
        }
        let dynamic = DynamicImage::ImageLuma8(image);
        assert_eq!(row_profiles(&dynamic, 0, 0, 2, 2), vec![2.0, 0.0]);
        // Out-of-band requests clip rather than panic.
        assert_eq!(row_profiles(&dynamic, 1, 1, 99, 99), vec![0.0]);
    }
}
