//! Read the four relic reward cards off the screen.
//!
//! This is the only source for the four cards. Memory cannot attribute them: the rewards are
//! resident, but nothing observed links them to a player or a screen slot, and that held whether
//! the local player was host or client. There is no per-player response record for anyone but the
//! local player, pointers to the four reward strings never cluster, and the display names sit far
//! apart with no ordered buffer.
//!
//! Reading the screen sidesteps attribution entirely, because the cards are already in screen
//! order. It is not general OCR either: EE.log names the squad's relics before the screen renders,
//! so each card only has to be matched to the nearest of roughly two dozen known rewards. A
//! garbled read still lands on the right item, which is what makes this trustworthy enough to be
//! published rather than guessed at.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use image::{DynamicImage, GenericImageView, GrayImage};

use warframe_acquisition::RewardCatalogEntry;

use crate::{overlay_window::WindowRect, reward_source::VisualRewardSource};

static LATEST_MATCHED_RECT: std::sync::Mutex<Option<WindowRect>> = std::sync::Mutex::new(None);

pub(crate) fn latest_matched_rect() -> Option<WindowRect> {
    *LATEST_MATCHED_RECT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn set_matched_rect(snapshot: &std::sync::Mutex<Option<WindowRect>>, rect: WindowRect) {
    *snapshot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(rect);
}

pub(crate) fn publish_latest_matched_rect(rect: WindowRect) {
    set_matched_rect(&LATEST_MATCHED_RECT, rect);
}

fn clear_matched_rect(snapshot: &std::sync::Mutex<Option<WindowRect>>) {
    *snapshot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
}

pub(crate) fn clear_latest_matched_rect() {
    clear_matched_rect(&LATEST_MATCHED_RECT);
}

/// Card geometry, calibrated from a labelled 1920x1080 reward screen: four cards on a 242px pitch
/// from x=478, i.e. a block centred on x=960.
///
/// These are fractions of window *height*, offset from the horizontal centre -- not fractions of
/// width. Warframe scales its HUD with height and centres it horizontally, so a card's distance
/// from the centre is a fixed multiple of the window height at every aspect ratio. Fractions of
/// width only look right because they agree with these at 16:9, and disagree everywhere else.
const CARD_PITCH: f32 = 242.0 / 1080.0;
const CARD_WIDTH: f32 = 240.0 / 1080.0;
/// Centre of the card block, as a signed fraction of height from the window's horizontal centre.
/// The 1920x1080 block spans x=478 to x=1444, whose centre is x=961 -- one pixel right of the
/// screen centre, which is measurement noise, not an offset. Keeping the measured value rather
/// than rounding it to zero is what makes the 1920x1080 calibration reproduce exactly.
const BLOCK_CENTRE: f32 = (478.0 + (242.0 * 3.0 + 240.0) / 2.0 - 960.0) / 1080.0;

/// The four-card block, for anything that needs to sit against the cards rather than read them.
///
/// The overlay used to invent its own rectangle -- 75% of the screen wide, 56% of the way down --
/// while this module had the cards measured to the pixel. Two independent guesses at one rectangle
/// is why the overlay was half again as wide as the cards and about 75px below them. There is one
/// definition now, and it is this one, because this is the one that is calibrated.
///
/// `BOTTOM` is the underside of the player-name row, measured at y=525 on the 2026-07-27 host
/// screen, with a few pixels of clearance. Vertical fractions were always fractions of height, so
/// this one needed no correction.
pub const CARD_BLOCK_BOTTOM: f32 = 530.0 / 1080.0;

/// A full squad, and the layout the fractions above are calibrated against.
pub const MAX_CARDS: usize = 4;

/// Left edge of the card block for a squad of `cards`, in pixels from the window's left edge.
///
/// Warframe centres the block on however many cards it has, so dropping a card pulls both edges in
/// by half a pitch. That is not a detail: on a three-card screen every card sits 121px right of
/// where a four-card reader looks, which is enough for slot 0's crop to straddle the gutter and cut
/// the first title in half. Centring the block is the same statement, and it is the one that keeps
/// holding when the aspect ratio changes.
pub fn card_block_left(cards: usize, width: u32, height: u32) -> f32 {
    width as f32 / 2.0 + BLOCK_CENTRE * height as f32 - card_block_width(cards, height) / 2.0
}

/// Width of the card block for a squad of `cards`, in pixels.
pub fn card_block_width(cards: usize, height: u32) -> f32 {
    (CARD_PITCH * cards.saturating_sub(1) as f32 + CARD_WIDTH) * height as f32
}

/// The title band, measured against three captured reward screens on 2026-07-27.
///
/// This box was y=418 high 76, which was wrong at both edges. The top clipped the ascenders off
/// the first line of a two-line title, and clipped glyphs do not read as noise -- they read as
/// confident wrong letters, so `Caliban Prime Chassis` came back as `Caliban Flime Gnassis`
/// (`C`->`G`, `h`->`n`) and the closed-set match had to absorb damage that was never in the pixels
/// on screen. The bottom reached past the title into the divider ornament below each card, which
/// tesseract read as a trailing `4` or `ty` on every single card, costing every read an edit.
///
/// y=408 clears the tallest ascender and height=58 stops above the divider. Thresholding cannot
/// substitute for this: clipped pixels are not on the screen to be recovered.
const TITLE_TOP: f32 = 408.0 / 1080.0;
const TITLE_HEIGHT: f32 = 58.0 / 1080.0;

/// Below this, the read is treated as a failure rather than published as a guess.
const MATCH_FLOOR: f32 = 0.6;
/// Below this a read is still published, but its crop is kept for diagnosis. Every labelled card
/// reads exactly once the title is separated from the card art, so anything under 0.85 is an
/// anomaly worth having the pixels for.
#[cfg(debug_assertions)]
const CROP_KEEP_BELOW: f32 = 0.85;

/// Distinguishes the crop files of concurrent readers.
///
/// Two readers are live at once whenever the log-triggered retry overlaps the poller, which is
/// exactly during the reward screen. Sharing one crop path means each deletes the other's file
/// mid-read, so the reads fail precisely when they are needed. Only the crop still touches the
/// disk -- it is what tesseract is handed -- because the capture itself now stays in memory.
static SCRATCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn scratch_file(kind: &str, extension: &str) -> PathBuf {
    let ticket = SCRATCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "tennoscope-{kind}-{}-{ticket}.{extension}",
        std::process::id()
    ))
}

/// The screen reader holds one capture object across its reads. The object itself is cheap; keeping
/// it here preserves per-source geometry-change suppression while each read still locates the
/// Warframe window and its current monitor again.
pub struct ScreenRewardSource {
    capture: crate::reward_capture::GameCapture,
}

impl Default for ScreenRewardSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenRewardSource {
    pub fn new() -> Self {
        Self {
            capture: crate::reward_capture::GameCapture::new(),
        }
    }
}

impl VisualRewardSource for ScreenRewardSource {
    fn choices(&mut self, candidates: &[RewardCatalogEntry]) -> Result<Vec<String>, &'static str> {
        let frames = self.capture.capture_candidates()?;
        read_capture_candidates(&frames, &LATEST_MATCHED_RECT, |frame| {
            read_cards_in(frame, candidates)
                .map(|cards| cards.into_iter().map(|(name, _)| name).collect())
        })
    }
}

fn read_capture_candidates<T>(
    candidates: &[crate::reward_capture::CapturedFrame],
    matched_rect: &std::sync::Mutex<Option<WindowRect>>,
    mut read: impl FnMut(&DynamicImage) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    let mut last_reason = "no Warframe window found";
    for candidate in candidates {
        match read(&candidate.image) {
            Ok(value) => {
                set_matched_rect(matched_rect, candidate.rect);
                return Ok(value);
            }
            Err(reason) => last_reason = reason,
        }
    }
    Err(last_reason)
}

/// Read the card titles out of a reward-screen image and match each to the relic pool.
///
/// Returns each card with the score it matched at. Callers only need the names, but the score is
/// what makes the crop geometry testable: a box that clips the title still lands on the right
/// reward through the closed-set match, so a name-only assertion passes against a misaligned crop
/// and proves nothing.
///
/// How many cards there are is not knowable ahead of time -- it is the squad size, and EE.log only
/// says so after the screen has already come and gone -- so the layouts are simply tried. Each
/// wrong one costs a single crop, because the read stops at the first card that will not match.
///
/// Widest first, and that ordering is load-bearing: a two-card block sits exactly over a four-card
/// block's middle two cards, so a four-card screen reads perfectly clean as "two cards" and would
/// quietly lose half the rewards if two were tried first. A solo run is not tried at all: one card
/// sits where a three-card screen's middle card sits, and a single reward is not a choice worth
/// advising on anyway.
///
/// Split out from the capture so it can be exercised against a real labelled screen instead of
/// only against a live game.
pub fn read_cards(
    image: &Path,
    candidates: &[RewardCatalogEntry],
) -> Result<Vec<(String, f32)>, &'static str> {
    let frame = image::open(image).map_err(|_| "capture could not be decoded")?;
    read_cards_in(&frame, candidates)
}

/// The same read against an already-decoded frame, which is what the live path has -- screen
/// capture stays in memory.
pub fn read_cards_in(
    image: &DynamicImage,
    candidates: &[RewardCatalogEntry],
) -> Result<Vec<(String, f32)>, &'static str> {
    if candidates.is_empty() {
        return Err("no reward candidates");
    }
    let (width, height) = image.dimensions();
    // Up to three layouts per poll rather than one, so a poll off the reward screen costs
    // three crops instead of one -- about 200ms every two seconds. Narrow it by asking the log for
    // the squad size if that ever shows up in a profile.
    let widest = read_cards_at(image, width, height, MAX_CARDS, candidates);
    if widest.is_ok() {
        return widest.map_err(|(_, reason)| reason);
    }
    // Ordering alone does not settle the four-against-two ambiguity, because it is the same pixels
    // either way: "two cards" and "four cards whose outer two would not match" are indistinguishable
    // at the two positions they share. The four-card slot 0 is the tiebreak. Blank means there is
    // genuinely nothing out there and the block really is narrower; any text at all means a wider
    // screen with a pool gap, and publishing its middle two as the whole screen would be a confident
    // half-answer. That case fails closed, exactly as it did before there was anything to guess.
    let outside_the_block_is_empty = matches!(&widest, Err((0, BLANK_CARD)));
    for cards in (2..MAX_CARDS).rev() {
        // Dropping one card shifts the block half a pitch, so a layout two cards narrower is shifted
        // a whole pitch and its slots land exactly on the widest layout's. Those are the ambiguous
        // ones, and only those need the tiebreak.
        let shares_slots_with_the_widest = (MAX_CARDS - cards) % 2 == 0;
        if shares_slots_with_the_widest && !outside_the_block_is_empty {
            break;
        }
        if let Ok(read) = read_cards_at(image, width, height, cards, candidates) {
            return Ok(read);
        }
    }
    widest.map_err(|(_, reason)| reason)
}

const BLANK_CARD: &str = "a reward card read as blank";

/// Reads the `cards` title slots of one layout, stopping at the first that will not match. The
/// failing slot comes back with the reason because `read_cards` needs to know whether the *first*
/// slot was the one that failed -- that is what tells a misplaced block apart from a pool gap.
fn read_cards_at(
    image: &DynamicImage,
    width: u32,
    height: u32,
    cards: usize,
    candidates: &[RewardCatalogEntry],
) -> Result<Vec<(String, f32)>, (usize, &'static str)> {
    let left = card_block_left(cards, width, height);
    let mut read = Vec::with_capacity(cards);
    for slot in 0..cards {
        let (text, crop) = read_region(
            image,
            (left + CARD_PITCH * slot as f32 * height as f32) as u32,
            (TITLE_TOP * height as f32) as u32,
            (CARD_WIDTH * height as f32) as u32,
            (TITLE_HEIGHT * height as f32) as u32,
        )
        .map_err(|reason| (slot, reason))?;
        let matched = best_match(&text, candidates);
        // Without the raw text a failed read is unattributable: reading the wrong place, reading a
        // screen that is not the reward screen, and reading a card whose name is not in the pool
        // all surface as the same error. The text alone is not enough either -- a misplaced crop
        // yields clean-looking wrong words rather than obvious garbage, which is how the title box
        // stayed 10px too low for five live runs. Keep the pixels behind a poor read.
        #[cfg(debug_assertions)]
        let keep_crop = matched
            .as_ref()
            .is_none_or(|(_, score)| *score < CROP_KEEP_BELOW);
        #[cfg(not(debug_assertions))]
        let keep_crop = false;
        log::debug!(
            "[DEBUG-card] cards={cards} slot={slot} raw={text:?} match={matched:?} crop={}",
            if keep_crop {
                crop.display().to_string()
            } else {
                "-".to_owned()
            }
        );
        if !keep_crop {
            let _ = std::fs::remove_file(&crop);
        }
        let (name, score) = matched.ok_or((slot, BLANK_CARD))?;
        if score < MATCH_FLOOR {
            return Err((slot, "reward card text did not match the relic pool"));
        }
        read.push((name, score));
    }
    Ok(read)
}

/// Locate the game window and capture it.
///
/// Linux window discovery uses xcap's X11 enumeration with an `xwininfo` fallback for nested Wine
/// virtual desktops. Linux monitor pixels are read directly from the X root so a Wayland desktop
/// cannot make xcap open its screenshot portal. Windows uses xcap's Windows Graphics Capture path,
/// which can read the game's D3D swapchain where GDI `BitBlt` returns a black frame.
///
/// The monitor is captured and cropped rather than the window captured directly: xcap's
/// `Window::capture_image` returns a stale frame for game windows on Windows (xcap#131), and a
/// reward screen read from a stale frame is a reward screen read from whatever was on screen a
/// moment ago.
///
/// The whole monitor is captured and cropped *here* rather than through `capture_region`, because
/// `capture_region` ignored which monitor it was asked. Measured on a two-output XWayland desktop
/// with xcap 0.9.8: `HDMI-A-1` at origin (0,0) and `HDMI-A-2` at (1920,0), and
/// `capture_region(0, 0, 1920, 1080)` returned byte-identical frames for both -- both of them
/// `HDMI-A-1`'s pixels. So a game on any monitor but the first read the first monitor's pixels,
/// every card missed the relic pool, and the overlay never appeared while the log showed a poller
/// running normally -- precisely the 2026-08-20 report, where the four cards were sitting on the
/// second monitor and read as `WOH DIGeil` and similar.
///
/// `GameCapture` supplies the X11/XWayland window and its containing monitor on every read. Keeping
/// the crop here makes the multi-monitor coordinate transform independent of window discovery.
///
/// Wrappers rather than a move because the crop maths is what the multi-monitor tests below pin
/// down, and those tests are the measured record of the 2026-08-20 wrong-monitor bug. Re-exporting
/// keeps them where they are, against the definitions they were written for.
pub(crate) fn visible_region_for(
    rect: WindowRect,
    origin_x: i32,
    origin_y: i32,
    monitor_width: u32,
    monitor_height: u32,
) -> Option<VisibleRegion> {
    visible_region(rect, origin_x, origin_y, monitor_width, monitor_height)
}

pub(crate) fn window_frame_from_monitor_for(
    whole: &image::RgbaImage,
    monitor_width: u32,
    monitor_height: u32,
    rect: WindowRect,
    visible: VisibleRegion,
) -> image::DynamicImage {
    window_frame_from_monitor(whole, monitor_width, monitor_height, rect, visible)
}

/// Cut the game window out of a whole-monitor capture and lay it into a window-sized frame.
///
/// Split from live capture because everything above it talks to the compositor and everything here
/// is arithmetic on pixels. The monitor-mixup bug lived in this half but could
/// only be reached through the other, so nothing could test it; this is the seam that makes the
/// multi-monitor case assertable without a second physical screen.
fn window_frame_from_monitor(
    whole: &image::RgbaImage,
    monitor_width: u32,
    monitor_height: u32,
    rect: WindowRect,
    visible: VisibleRegion,
) -> image::DynamicImage {
    // A scaled display hands back the framebuffer's pixels, not the logical ones asked for: X and
    // the window manager speak in logical units, the compositor captures physical ones. The crop
    // below is in logical units, so an unscaled frame would have it read the wrong place entirely
    // -- and the paste after it clips rather than scales, so an oversized piece becomes a magnified
    // corner and every card reads blank, which is exactly how a working reader looks from outside.
    // Resampling the whole monitor to its logical size first is what makes the fractions mean the
    // same thing on a scaled desktop as on an unscaled one.
    let resampled;
    let whole = if whole.dimensions() == (monitor_width, monitor_height) {
        whole
    } else {
        log::debug!(
            "[DEBUG-capture] scaled capture {:?} for monitor {monitor_width}x{monitor_height}",
            whole.dimensions(),
        );
        resampled = image::imageops::resize(
            whole,
            monitor_width,
            monitor_height,
            image::imageops::FilterType::Lanczos3,
        );
        &resampled
    };
    // Crop here rather than asking `capture_region` for the piece, because it hands back the first
    // monitor's pixels whatever monitor it belongs to -- see the live-capture note above.
    let captured =
        image::imageops::crop_imm(whole, visible.x, visible.y, visible.width, visible.height)
            .to_image();
    // Paste back at the window's own origin: every crop downstream is a fraction of the *window*,
    // so the frame handed on has to be window sized even when part of it was off screen.
    let mut frame = image::RgbaImage::new(rect.width, rect.height);
    image::imageops::replace(
        &mut frame,
        &captured,
        i64::from(visible.paste_x),
        i64::from(visible.paste_y),
    );
    image::DynamicImage::ImageRgba8(frame)
}

/// The part of the game window that is actually on this monitor.
///
/// `capture_region` rejects a region that reaches past the monitor rather than clipping it, so a
/// game in windowed mode sitting even one pixel off the edge -- which is ordinary, the title bar
/// gets dragged -- makes every capture fail with "could not capture the game window". Clamping the
/// offset alone is worse than failing: the region would then be captured from the wrong place and
/// the reward crops would silently read the pixels next to the cards.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VisibleRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    /// Where the captured piece belongs within the window-sized frame.
    paste_x: u32,
    paste_y: u32,
}

fn visible_region(
    rect: WindowRect,
    origin_x: i32,
    origin_y: i32,
    monitor_width: u32,
    monitor_height: u32,
) -> Option<VisibleRegion> {
    let (x, width, paste_x) = visible_axis(rect.x, origin_x, rect.width, monitor_width)?;
    let (y, height, paste_y) = visible_axis(rect.y, origin_y, rect.height, monitor_height)?;
    (width > 0 && height > 0).then_some(VisibleRegion {
        x,
        y,
        width,
        height,
        paste_x,
        paste_y,
    })
}

/// One axis of the clip: where to capture from, how much of it is on the monitor, and how far into
/// the window-sized frame the piece belongs.
fn visible_axis(start: i32, origin: i32, span: u32, monitor: u32) -> Option<(u32, u32, u32)> {
    let offset = i64::from(start) - i64::from(origin);
    let clipped = offset.max(0);
    let visible = (offset + i64::from(span)).min(i64::from(monitor)) - clipped;
    Some((
        u32::try_from(clipped).ok()?,
        u32::try_from(visible.max(0)).ok()?,
        u32::try_from(clipped - offset).ok()?,
    ))
}

/// Crop one region and OCR it. Returns the text and the crop, which the caller deletes -- it is
/// kept only when the read was poor enough to be worth looking at.
///
/// The card title is near-white text laid over arbitrary card art, and handing tesseract that
/// greyscale crop directly makes it read the art: a dark helmet behind a word garbles it, and card
/// borders at the edge of the crop come back as leading `|`, `Fr` or `pA UY`. Isolating the text
/// from the art is what fixes both, and it is a two-step job. `-normalize` first, so the cutoff is
/// relative to the crop's own brightness rather than an absolute grey level -- that is what makes
/// one constant work across card art, and it should also absorb another machine's gamma. Then
/// `-threshold` to drop everything dimmer than the text, and `-negate` because tesseract is trained
/// on dark-on-light.
///
/// 74% is the middle of a plateau, not a tuned peak, and the plateau was re-swept against this Rust
/// pipeline rather than inherited from the ImageMagick one it replaced. Over the eleven labelled
/// fixture cards: 66% and below garbles a card, 70% and 74% read every card exactly, 78% drops
/// `Caliban Prime Chassis Blueprint` to 0.96 and 82% drops `Bronco Prime Receiver` to 0.89. So the
/// usable band is 70-78% and 74% sits in it with room on both sides.
fn read_region(
    image: &DynamicImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<(String, PathBuf), &'static str> {
    let crop = scratch_file("reward-crop", "png");
    prepare_crop(image, x, y, width, height)
        .save(&crop)
        .map_err(|_| "could not write the reward card crop")?;
    let text = ocr_crop(&crop)?;
    Ok((text, crop))
}

/// Crop, greyscale, normalize, threshold, invert and upscale one card title.
///
/// Split from the file handling so the pipeline can be driven from pixels in a test rather than
/// only through a temp file.
pub fn prepare_crop(source: &DynamicImage, x: u32, y: u32, width: u32, height: u32) -> GrayImage {
    let cropped = source.view(x, y, width, height).to_image();
    let grey = cropped
        .pixels()
        .map(|pixel| luma(pixel[0], pixel[1], pixel[2]))
        .collect::<Vec<_>>();
    let prepared = threshold_inverted(&normalize_contrast(&grey));
    let prepared = GrayImage::from_raw(width, height, prepared)
        .unwrap_or_else(|| GrayImage::new(width, height));
    // 300%, matching what `magick -resize` did: tesseract reads a 58px title band poorly and a
    // 174px one exactly. ImageMagick's default filter is Mitchell, which `image` does not offer, so
    // the replacement was swept over the eleven labelled fixture cards rather than guessed at:
    // Nearest reads `Burston Prime Stock` at 0.89 and Triangle at 0.94, while CatmullRom and
    // Lanczos3 both read every card exactly bar the wrapped screen's known speck at 0.954. Either
    // of the last two would do; CatmullRom is the cheaper kernel.
    image::imageops::resize(
        &prepared,
        width * 3,
        height * 3,
        image::imageops::FilterType::CatmullRom,
    )
}

/// ImageMagick 7's `-colorspace gray`: a Rec.709 weighted sum of the *gamma-encoded* bytes.
///
/// `image`'s own `to_luma8` is Rec.601 and weights red at 76 rather than 54. On near-white text
/// over dark card art that difference is large enough to move pixels across the threshold, so the
/// weighting is spelled out here rather than taken from the crate.
pub fn luma(red: u8, green: u8, blue: u8) -> u8 {
    let value = 0.212_656 * red as f32 + 0.715_158 * green as f32 + 0.072_186 * blue as f32;
    value.round().clamp(0.0, 255.0) as u8
}

/// ImageMagick's `-normalize`, which is `-contrast-stretch 2%x1%` rather than a plain min-max
/// stretch.
///
/// The clipping is what makes one threshold constant work across card art: it discards the darkest
/// 2% and brightest 1% of the histogram before stretching, so a stray specular highlight cannot pin
/// the top of the range and leave the actual text sitting well below the cutoff.
pub fn normalize_contrast(grey: &[u8]) -> Vec<u8> {
    if grey.is_empty() {
        return Vec::new();
    }
    let mut histogram = [0_u32; 256];
    for value in grey {
        histogram[*value as usize] += 1;
    }
    let total = grey.len() as f64;
    let black_clip = (total * 0.02) as u32;
    let white_clip = (total * 0.01) as u32;

    let mut seen = 0;
    let low = histogram
        .iter()
        .position(|count| {
            seen += count;
            seen > black_clip
        })
        .unwrap_or(0) as u8;
    let mut seen = 0;
    let high = histogram
        .iter()
        .rposition(|count| {
            seen += count;
            seen > white_clip
        })
        .unwrap_or(255) as u8;

    // A flat crop -- a capture taken a moment too early is entirely black -- has no range to
    // stretch, and dividing by it would panic on exactly that frame.
    if high <= low {
        return grey.to_vec();
    }
    let span = (high - low) as f32;
    grey.iter()
        .map(|value| (((*value).clamp(low, high) - low) as f32 * 255.0 / span).round() as u8)
        .collect()
}

/// `-threshold 74% -negate` in one pass: keep what is brighter than the cutoff, then invert,
/// because tesseract is trained on dark-on-light.
pub fn threshold_inverted(grey: &[u8]) -> Vec<u8> {
    // ImageMagick compares strictly against the scaled cutoff: at 74% of 255 that is 188.7, so 188
    // is background and 189 is text.
    let cutoff = 0.74 * 255.0;
    grey.iter()
        .map(|value| if (*value as f32) > cutoff { 0 } else { 255 })
        .collect()
}

/// The bundled Tesseract's file name, which is the whole of the platform difference.
pub const TESSERACT_EXECUTABLE: &str = if cfg!(windows) {
    "tesseract.exe"
} else {
    "tesseract"
};

/// Which Tesseract to run, given the app's resource directory.
///
/// Windows has no package manager to lean on and no player should have to install an OCR engine
/// before the overlay works, so the NSIS bundle ships one under `tesseract/` and this prefers it.
/// The fallback is not a nicety: a `cargo test` run has no resource directory at all, and every
/// Linux package still gets Tesseract from the distribution.
pub fn tesseract_program(resource_dir: &Path) -> PathBuf {
    let bundled = resource_dir.join("tesseract").join(TESSERACT_EXECUTABLE);
    if bundled.is_file() {
        return bundled;
    }
    PathBuf::from("tesseract")
}

/// Set once at startup from the resolved resource directory, because the OCR path is reached from
/// worker threads that have no `AppHandle` to ask.
static TESSERACT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Point the OCR path at the bundled engine. Called once from Tauri's `setup`; later calls lose,
/// which is what makes this safe to call from a test that only wants the default.
pub fn use_bundled_tesseract(resource_dir: &Path) {
    let _ = TESSERACT.set(tesseract_program(resource_dir));
}

/// OCR a crop that `read_region` has already isolated to text.
///
/// `--psm 11`, sparse text, rather than the obvious `--psm 6`, one uniform block. The title band
/// reserves room above the title for a second line, and on a one-line title that room is empty --
/// so anything the game draws up there arrives as a speck floating above the words. `psm 6` has to
/// call one of them "the block", and when it picks the speck it does not merely add noise, it
/// returns the speck *instead of the title*: a real 2026-07-28 crop reading `Dual Zoren Prime
/// Handle` came back as `"| @\nn |\n|"`. Every poll failed that way until the speck went, which cost
/// about nine seconds of a fifteen-second screen.
///
/// `psm 11` does not have to choose -- it reads every text region it finds. Swept over the twelve
/// labelled crops from four captured screens plus that live one, `psm 11` and `psm 12` read all
/// twelve; `psm 3`, `4` and `6` miss the speck case entirely, `psm 7` mangles wrapped titles, and
/// `psm 13` clips leading letters. `11` over `12` only because `12` adds orientation detection this
/// does not need. What it costs is a little leading punctuation, which `normalise` drops before the
/// match ever sees it.
pub fn ocr_crop(image: &Path) -> Result<String, &'static str> {
    run_tesseract(image, "11", None)
}

/// OCR one already-isolated text line, restricted to the supplied glyph set.
pub(crate) fn ocr_crop_line(image: &Path, whitelist: &str) -> Result<String, &'static str> {
    run_tesseract(image, "7", Some(whitelist))
}

fn run_tesseract(
    image: &Path,
    page_segmentation_mode: &str,
    whitelist: Option<&str>,
) -> Result<String, &'static str> {
    let program = TESSERACT
        .get()
        .cloned()
        .unwrap_or_else(|| "tesseract".into());
    let mut command = Command::new(&program);
    // One recognition thread per spawn: tesseract's own OpenMP pooling oversubscribes the
    // machine when several crops are read side by side, and the kiosk poller does exactly that.
    command.env("OMP_THREAD_LIMIT", "1");
    // The bundled engine's `eng.traineddata` sits beside it, not in the install prefix it was
    // compiled with, so it has to be told where to look. `--tessdata-dir` rather than the
    // `TESSDATA_PREFIX` environment variable because setting one of those is `unsafe` since the
    // 2024 edition, and this crate forbids that.
    if let Some(directory) = program.parent().filter(|path| !path.as_os_str().is_empty()) {
        command.args([
            std::ffi::OsStr::new("--tessdata-dir"),
            directory.as_os_str(),
        ]);
    }
    command
        .arg(image)
        .args(["-", "--psm", page_segmentation_mode]);
    if let Some(whitelist) = whitelist {
        command.args(["-c", &format!("tessedit_char_whitelist={whitelist}")]);
    }
    let text = command.output().map_err(|_| "tesseract is not available")?;
    Ok(String::from_utf8_lossy(&text.stdout).into_owned())
}

/// Compare on alphanumerics only. That is what lets a read of "2 X Forma Blueprint W\:" land on
/// "2X Forma Blueprint" instead of drifting to a different reward.
fn normalise(text: &str) -> String {
    text.chars()
        .filter(char::is_ascii_alphanumeric)
        .flat_map(char::to_lowercase)
        .collect()
}

/// How near-exact a fragment of a read must be before it may name the whole card.
///
/// Scoring fragments is what lets a card be found under the specks Tesseract returns at
/// `--psm 11`, but it cannot be done at `MATCH_FLOOR`: `blueprint` is a suffix of most rewards
/// and alone scores 0.64 against `Forma Blueprint`, so a mis-crop that recovers one generic word
/// would resolve to a confident wrong reward. Measured: `WOH DIGeil / Blueprint` -- this file's
/// own 2026-08-20 wrong-monitor read -- scored 0.56 and was rejected before fragments were
/// scored, and 0.64 and accepted after. A fragment has to be a near-exact match to speak.
const PARTIAL_MATCH_FLOOR: f32 = 0.85;

/// The card's text split on blank lines, each group's own lines rejoined with a space.
///
/// Blank means whitespace-only rather than exactly `"\n\n"`: tesseract's stdout is passed through
/// `String::from_utf8_lossy` with no newline normalisation, so on Windows the separator is
/// `"\r\n\r\n"`, and a blank line that carries a stray space is common on a dark card. Splitting
/// on the literal `"\n\n"` left both shapes as a single group, which made this whole mechanism
/// silently inert exactly where it was most needed.
fn text_groups(text: &str) -> Vec<String> {
    let mut groups = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            if !current.is_empty() {
                groups.push(current.join(" "));
                current.clear();
            }
        } else {
            current.push(line);
        }
    }
    if !current.is_empty() {
        groups.push(current.join(" "));
    }
    groups.retain(|group| !normalise(group).is_empty());
    groups
}

/// The best pool match for a card.
///
/// Two thresholds, because the two kinds of read carry different weight. The whole text is what
/// the card actually said, so it is scored without a floor here; `read_cards_at` applies
/// `MATCH_FLOOR` to the returned score. The whole text resolves a wrapped name, where
/// `Dual Zoren Prime` and `Blueprint` each match nothing alone. A fragment is a guess about which
/// part of the read is the name, so it only counts here at `PARTIAL_MATCH_FLOOR`: that is what
/// recovers `Forma Blueprint` from under three specks of noise (0.64 -> 1.0) without letting a
/// lone `Blueprint` name a card it cannot identify.
///
/// Still a closed-set match -- it returns the nearest pool name, never "not in the pool" -- so
/// the floors are the only guard against a confident wrong answer.
pub fn best_match(text: &str, candidates: &[RewardCatalogEntry]) -> Option<(String, f32)> {
    let whole = best_match_of(text, candidates);
    let groups = text_groups(text);
    let mut fragments = groups.clone();
    for start in 0..groups.len() {
        fragments.push(groups[start..].join(" "));
    }
    // Deduped on the normalised form, because `normalise` strips the separators that are the only
    // difference between a wrapped name and its rejoined groups -- on raw text every such pair
    // scores twice. Seeding with the whole text drops the `start == 0` run, which normalises to
    // exactly it. That is to avoid a redundant pass over the pool, not to protect a floor: the
    // whole text is scored above without a local floor either way, and a duplicate of it can only
    // tie that score, never change the result whose score the caller checks against `MATCH_FLOOR`.
    let mut seen = vec![normalise(text)];
    let mut distinct = Vec::with_capacity(fragments.len());
    for fragment in fragments {
        let normalised = normalise(&fragment);
        if normalised.is_empty() || seen.contains(&normalised) {
            continue;
        }
        seen.push(normalised);
        distinct.push(fragment);
    }
    let best_fragment = distinct
        .iter()
        .filter_map(|fragment| best_match_of(fragment, candidates))
        .filter(|(_, score)| *score >= PARTIAL_MATCH_FLOOR)
        .max_by(|(_, left), (_, right)| left.total_cmp(right));
    match (whole, best_fragment) {
        (Some(whole), Some(fragment)) => Some(if fragment.1 > whole.1 {
            fragment
        } else {
            whole
        }),
        (whole, fragment) => whole.or(fragment),
    }
}

/// One read against the pool. This is the whole of what `best_match` used to be.
fn best_match_of(text: &str, candidates: &[RewardCatalogEntry]) -> Option<(String, f32)> {
    let read = normalise(text);
    if read.is_empty() {
        return None;
    }
    candidates
        .iter()
        .map(|candidate| {
            let known = normalise(&candidate.name);
            let distance = edit_distance(&read, &known);
            let longest = read.chars().count().max(known.chars().count()).max(1);
            let score = 1.0 - distance as f32 / longest as f32;
            (candidate.name.clone(), score)
        })
        .max_by(|(_, left), (_, right)| left.total_cmp(right))
}

fn edit_distance(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut previous = (0..=right.len()).collect::<Vec<_>>();
    let mut current = vec![0; right.len() + 1];
    for (row, left_char) in left.chars().enumerate() {
        current[0] = row + 1;
        for (column, right_char) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left_char != *right_char);
            current[column + 1] = substitution
                .min(previous[column + 1] + 1)
                .min(current[column] + 1);
        }
        std::mem::swap(&mut previous, &mut current);
    }
    previous[right.len()]
}

#[cfg(test)]
mod tests {
    use image::GenericImageView;

    use std::sync::Mutex;

    use crate::reward_capture::{CapturedFrame, FrameBackend, RectOrigin};

    use super::{
        VisibleRegion, WindowRect, clear_matched_rect, read_capture_candidates, visible_region,
        window_frame_from_monitor,
    };

    /// A flat monitor capture, tagged so a frame can be traced back to the screen it came from.
    fn monitor(width: u32, height: u32, tag: u8) -> image::RgbaImage {
        image::RgbaImage::from_pixel(width, height, image::Rgba([tag, tag, tag, 255]))
    }

    fn candidate(x: i32, tag: u8) -> CapturedFrame {
        CapturedFrame {
            rect: WindowRect {
                x,
                y: 0,
                width: 1920,
                height: 1080,
            },
            image: image::DynamicImage::ImageRgba8(monitor(1, 1, tag)),
            rect_origin: RectOrigin::X11,
            frame_backend: FrameBackend::X11,
        }
    }

    /// Mutation caught: publishing before OCR or returning the first failure would leave the
    /// overlay on monitor A even though only monitor B contained the reward screen.
    #[test]
    fn failed_candidate_a_then_successful_b_publishes_b() {
        let matched = Mutex::new(None);
        let frames = [candidate(0, 10), candidate(1920, 20)];

        let result = read_capture_candidates(&frames, &matched, |image| {
            if image.to_rgba8().get_pixel(0, 0)[0] == 20 {
                Ok(vec!["Forma Blueprint".to_owned()])
            } else {
                Err("reward card text did not match the relic pool")
            }
        });

        assert_eq!(result, Ok(vec!["Forma Blueprint".to_owned()]));
        assert_eq!(*matched.lock().unwrap(), Some(frames[1].rect));
    }

    /// Mutation caught: writing each attempted rect would publish the final failed monitor even
    /// though no candidate had proven it contained Warframe's reward screen.
    #[test]
    fn all_failed_candidates_publish_nothing_and_keep_the_last_reason() {
        let matched = Mutex::new(None);
        let frames = [candidate(0, 10), candidate(1920, 20)];

        let result: Result<(), &'static str> =
            read_capture_candidates(&frames, &matched, |image| {
                if image.to_rgba8().get_pixel(0, 0)[0] == 10 {
                    Err("a reward card read as blank")
                } else {
                    Err("reward card text did not match the relic pool")
                }
            });

        assert_eq!(result, Err("reward card text did not match the relic pool"));
        assert_eq!(*matched.lock().unwrap(), None);
    }

    /// Mutation caught: `lock().ok()` would silently stop publishing forever after one panic
    /// poisoned the process-wide matched-rect snapshot.
    #[test]
    fn matched_rect_publication_recovers_a_poisoned_snapshot() {
        let matched = Mutex::new(None);
        let _ = std::panic::catch_unwind(|| {
            let _guard = matched.lock().unwrap();
            panic!("poison the local matched-rect snapshot");
        });
        let frame = candidate(1920, 20);

        let result = read_capture_candidates(std::slice::from_ref(&frame), &matched, |_| Ok(()));

        assert_eq!(result, Ok(()));
        assert_eq!(
            *matched
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            Some(frame.rect)
        );
    }

    /// Mutation caught: game-exit teardown hid the overlay but retained the previous session's
    /// monitor rectangle, so previews could be positioned against stale geometry.
    #[test]
    fn clearing_matched_rect_removes_the_previous_session() {
        let matched = Mutex::new(Some(candidate(1920, 20).rect));
        clear_matched_rect(&matched);
        assert_eq!(*matched.lock().unwrap(), None);
    }

    #[test]
    fn clearing_matched_rect_recovers_a_poisoned_snapshot() {
        let matched = Mutex::new(Some(candidate(1920, 20).rect));
        let _ = std::panic::catch_unwind(|| {
            let _guard = matched.lock().unwrap();
            panic!("poison the local matched-rect snapshot");
        });

        clear_matched_rect(&matched);

        assert_eq!(
            *matched
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
            None
        );
    }

    /// The reward screen must be cut from the capture of the monitor the game is actually on.
    ///
    /// This is the 2026-08-20 bug, and it was not a geometry mistake: `capture_region` hands back
    /// the *first* monitor's pixels whatever monitor it is asked, measured byte-identical on a
    /// two-output desktop with xcap 0.9.8. So a game on the second screen was read against the
    /// first screen's pixels, every card missed the relic pool, `poll failed: reward card text did
    /// not match the relic pool` repeated for the whole three minutes of the fissure, and no
    /// overlay ever appeared. Reading the report's own screenshot confirmed it: the left half read
    /// as `WOH DIGeil`, the right half -- where the game was -- read all four cards exactly.
    ///
    /// Asserting on the tag is what pins it. A frame built from the handed-in capture carries that
    /// capture's tag; one that quietly sampled another screen carries the other's.
    #[test]
    fn the_frame_is_cut_from_the_monitor_the_game_is_on() {
        // Warframe fullscreen on the second monitor of a 3840x1080 desktop, which is the reported
        // layout: `HDMI-A-1` at (0,0) and `HDMI-A-2` at (1920,0).
        let rect = WindowRect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let visible = visible_region(rect, 1920, 0, 1920, 1080).expect("the window is on screen");
        assert_eq!(
            (visible.x, visible.y, visible.paste_x, visible.paste_y),
            (0, 0, 0, 0),
            "a fullscreen window is captured monitor-relative, so the offsets are the origin"
        );

        let second = monitor(1920, 1080, 200);
        let frame = window_frame_from_monitor(&second, 1920, 1080, rect, visible);
        assert_eq!(frame.dimensions(), (1920, 1080));
        assert_eq!(
            frame.to_rgba8().get_pixel(960, 540)[0],
            200,
            "the frame must carry the pixels of the monitor it was handed, not another screen's"
        );

        // The other screen's capture is the same shape and the same call, and must never be what a
        // game on the second monitor reads -- that is the whole of the bug.
        let first = monitor(1920, 1080, 40);
        let wrong = window_frame_from_monitor(&first, 1920, 1080, rect, visible);
        assert_ne!(
            wrong.to_rgba8().get_pixel(960, 540)[0],
            frame.to_rgba8().get_pixel(960, 540)[0],
            "two monitors with different contents must not produce the same frame"
        );
    }

    /// A window hanging off the monitor keeps its place in the window-sized frame: the captured
    /// piece is pasted at the offset it belongs to, and the rest stays empty rather than sliding
    /// the cards over by the width of the missing strip.
    #[test]
    fn a_clipped_window_is_pasted_where_it_belongs() {
        let rect = WindowRect {
            x: -100,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let visible = visible_region(rect, 0, 0, 1920, 1080).expect("partly on screen");
        assert_eq!((visible.x, visible.width, visible.paste_x), (0, 1820, 100));

        let frame = window_frame_from_monitor(&monitor(1920, 1080, 200), 1920, 1080, rect, visible)
            .to_rgba8();
        assert_eq!(frame.dimensions(), (1920, 1080));
        assert_eq!(
            frame.get_pixel(0, 540),
            &image::Rgba([0, 0, 0, 0]),
            "the strip that was off screen stays empty"
        );
        assert_eq!(
            frame.get_pixel(1000, 540)[0],
            200,
            "the piece that was on screen lands at its own offset"
        );
    }

    /// A scaled desktop hands back framebuffer pixels, and the crop that follows is in logical
    /// units, so the whole monitor is resampled to its logical size before anything is cut out of
    /// it. Without that the crop reads the wrong place entirely.
    #[test]
    fn an_oversized_capture_is_resampled_to_the_logical_monitor_first() {
        let rect = WindowRect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let visible = visible_region(rect, 1920, 0, 1920, 1080).expect("the window is on screen");
        // 1.5x scaling: the compositor hands back 2880x1620 for a 1920x1080 logical screen.
        let frame = window_frame_from_monitor(&monitor(2880, 1620, 200), 1920, 1080, rect, visible);
        assert_eq!(
            frame.dimensions(),
            (1920, 1080),
            "the frame is window sized whatever the framebuffer's scale"
        );
        assert_eq!(frame.to_rgba8().get_pixel(1900, 1070)[0], 200);
    }

    /// `capture_region` rejects an out-of-bounds region rather than clipping it, so a game window
    /// hanging off the edge of the monitor -- ordinary in windowed mode, and the shape a second
    /// monitor produces at every capture -- failed the whole read. The clip is what keeps that a
    /// partial frame instead of no frame.
    #[test]
    fn a_window_hanging_off_the_monitor_is_clipped_rather_than_refused() {
        let full = WindowRect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            visible_region(full, 1920, 0, 1920, 1080),
            Some(VisibleRegion {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                paste_x: 0,
                paste_y: 0
            }),
            "a window filling its monitor must capture whole and unshifted"
        );

        // Dragged 100px past the right edge and 50 above the top: the capture shrinks, and the
        // piece that survives belongs 50px down in the window-sized frame, not at its origin.
        let overhanging = WindowRect {
            x: 1820,
            y: -50,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            visible_region(overhanging, 0, 0, 1920, 1080),
            Some(VisibleRegion {
                x: 1820,
                y: 0,
                width: 100,
                height: 1030,
                paste_x: 0,
                paste_y: 50
            })
        );

        // Entirely off the monitor is the one case with nothing to read; an empty region would be
        // rejected by `capture_region` anyway, so it has to be an absence here.
        let elsewhere = WindowRect {
            x: 4000,
            y: 0,
            width: 800,
            height: 600,
        };
        assert_eq!(visible_region(elsewhere, 0, 0, 1920, 1080), None);
    }

    fn pool_entry(name: &str) -> super::RewardCatalogEntry {
        super::RewardCatalogEntry {
            name: name.to_owned(),
            ducats: 0,
        }
    }

    /// The 2026-08-22 report's own slot 1: Tesseract returns three noise fragments above the
    /// real name. Scoring the whole blob puts this at 0.636 against a 0.6 floor, so one more
    /// speck of noise would have discarded a correct read.
    #[test]
    fn a_card_read_with_noise_above_it_scores_on_its_own_line() {
        let pool = [
            pool_entry("Forma Blueprint"),
            pool_entry("Wisp Prime Neuroptics Blueprint"),
            pool_entry("Dual Zoren Prime Blueprint"),
        ];
        let (name, score) = super::best_match("&\n\nvr\n\ni STrTl\n\nForma Blueprint", &pool)
            .expect("a noisy read still resolves");
        assert_eq!(name, "Forma Blueprint");
        assert!(
            score > 0.9,
            "scored {score}: the noise is still being scored against the pool"
        );
    }

    /// The reason the whole-text candidate has to stay: the game wraps a long reward name onto
    /// two lines, and each line alone matches nothing.
    #[test]
    fn a_wrapped_reward_name_still_matches_across_its_lines() {
        let pool = [
            pool_entry("Dual Zoren Prime Blueprint"),
            pool_entry("Forma Blueprint"),
        ];
        let (name, score) = super::best_match("Dual Zoren Prime\n\nBlueprint", &pool)
            .expect("a wrapped name resolves");
        assert_eq!(name, "Dual Zoren Prime Blueprint");
        assert!(score > 0.99, "scored {score} on an exact wrapped read");
    }

    /// Scoring more candidates must not invent a match out of pure noise: the floor is the only
    /// thing standing between a garbled read and a confident wrong answer.
    #[test]
    fn pure_noise_still_scores_below_the_match_floor() {
        let pool = [
            pool_entry("Forma Blueprint"),
            pool_entry("Wisp Prime Neuroptics Blueprint"),
        ];
        for noise in ["&\n\nvr\n\ni STrTl", "xx\n\nzzz qq"] {
            let score = super::best_match(noise, &pool)
                .map(|(_, score)| score)
                .unwrap_or_default();
            assert!(
                score < super::MATCH_FLOOR,
                "{noise:?} scored {score}, at or above the {} floor",
                super::MATCH_FLOOR
            );
        }

        // Asserted as an absence rather than a low score: a score of 0.0 from `unwrap_or_default`
        // is below the floor whatever the implementation does, so it would pass vacuously.
        assert!(
            super::best_match("\n\n\n", &pool).is_none(),
            "a read with no alphanumerics has nothing to match and must be an absence"
        );
    }

    /// A mis-crop that recovers one generic word must not name a card. `Blueprint` is a suffix on
    /// most Warframe rewards, so alone it identifies nothing -- yet it scores 0.64 against
    /// `Forma Blueprint`, over the 0.6 floor.
    ///
    /// `WOH DIGeil` is this file's own recorded read of the 2026-08-20 wrong-monitor capture. That
    /// bug surfaced as `reward card text did not match the relic pool`; scoring fragments at
    /// `MATCH_FLOOR` would have turned it into a confident wrong reward instead.
    #[test]
    fn a_mis_cropped_read_that_recovers_one_generic_word_is_rejected() {
        let pool = [
            pool_entry("Forma Blueprint"),
            pool_entry("Wisp Prime Neuroptics Blueprint"),
            pool_entry("Dual Zoren Prime Blueprint"),
        ];
        for mis_crop in ["WOH DIGeil\n\nBlueprint", "aaaa bbbb\n\ncccc\n\nBlueprint"] {
            let score = super::best_match(mis_crop, &pool)
                .map(|(_, score)| score)
                .unwrap_or_default();
            assert!(
                score < super::MATCH_FLOOR,
                "{mis_crop:?} scored {score}, at or above the {} floor: one generic word is \
                 naming a card it cannot identify",
                super::MATCH_FLOOR
            );
        }
    }

    /// `ocr_crop` hands back tesseract's stdout through `String::from_utf8_lossy` with no newline
    /// normalisation, and Windows is a supported capture path, so grouping that keys on the
    /// literal `"\n\n"` is inert on exactly the platform half of the users are on.
    #[test]
    fn noise_above_a_name_is_read_through_a_windows_line_ending() {
        let pool = [
            pool_entry("Forma Blueprint"),
            pool_entry("Wisp Prime Neuroptics Blueprint"),
        ];
        let (name, score) =
            super::best_match("&\r\n\r\nvr\r\n\r\ni STrTl\r\n\r\nForma Blueprint", &pool)
                .expect("a CRLF read still resolves");
        assert_eq!(name, "Forma Blueprint");
        assert!(score > 0.9, "scored {score} on a CRLF read");
    }

    /// A "blank" line off a dark card routinely carries a stray space, which is not an empty
    /// string. Treating blank as whitespace-only is what keeps the groups separated.
    #[test]
    fn a_blank_line_carrying_a_space_still_separates_groups() {
        let pool = [
            pool_entry("Forma Blueprint"),
            pool_entry("Wisp Prime Neuroptics Blueprint"),
        ];
        let (name, score) = super::best_match("&\n \nvr\n \ni STrTl\n \nForma Blueprint", &pool)
            .expect("a read whose blank lines carry spaces still resolves");
        assert_eq!(name, "Forma Blueprint");
        assert!(score > 0.9, "scored {score} on a space-separated read");
    }

    /// The case between the other two: noise above a name that is itself wrapped. Neither the
    /// whole text nor any single group matches here -- only a trailing run of groups does, which
    /// is what makes that loop load-bearing rather than decoration.
    #[test]
    fn a_trailing_run_of_groups_recovers_a_wrapped_name_under_noise() {
        let pool = [
            pool_entry("Dual Zoren Prime Blueprint"),
            pool_entry("Forma Blueprint"),
            pool_entry("Wisp Prime Neuroptics Blueprint"),
        ];
        let (name, score) = super::best_match("vr\n\nDual Zoren Prime\n\nBlueprint", &pool)
            .expect("a wrapped name under noise resolves");
        assert_eq!(name, "Dual Zoren Prime Blueprint");
        // Asserted near-exact, not merely over the floor: the whole-text read already carries this
        // input to 0.92, because two chars of noise against a 25-char read is a small penalty. So
        // a `> 0.9` assertion passes with the trailing-run loop deleted and pins nothing. Only the
        // rejoined run `Dual Zoren Prime Blueprint` reaches 1.0.
        assert!(score > 0.99, "scored {score} on a wrapped name under noise");
    }
}
