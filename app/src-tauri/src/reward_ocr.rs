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

/// Card geometry as fractions of the window, calibrated from a labelled 1920x1080 reward screen:
/// four cards on a 242px pitch from x=478. Warframe scales its UI with the window, so fractions
/// carry across resolutions where a pixel table would not.
const CARD_LEFT: f32 = 478.0 / 1920.0;
const CARD_PITCH: f32 = 242.0 / 1920.0;
const CARD_WIDTH: f32 = 240.0 / 1920.0;

/// The four-card block, for anything that needs to sit against the cards rather than read them.
///
/// The overlay used to invent its own rectangle -- 75% of the screen wide, 56% of the way down --
/// while this module had the cards measured to the pixel. Two independent guesses at one rectangle
/// is why the overlay was half again as wide as the cards and about 75px below them. There is one
/// definition now, and it is this one, because this is the one that is calibrated.
///
/// `BOTTOM` is the underside of the player-name row, measured at y=525 on the 2026-07-27 host
/// screen, with a few pixels of clearance.
///
/// Caveat carried by both users of these constants: they are fractions of window *width*, verified
/// only against 16:9 captures. If Warframe scales its HUD with height and centres it -- which is
/// the usual arrangement, and which these numbers cannot distinguish at 16:9 -- then both the crop
/// and the overlay drift on an ultrawide display. Fixing that means re-deriving from a non-16:9
/// capture, and it would be fixed here, once, for both.
pub const CARD_BLOCK_BOTTOM: f32 = 530.0 / 1080.0;

/// A full squad, and the layout the fractions above are calibrated against.
pub const MAX_CARDS: usize = 4;

/// Left edge of the card block for a squad of `cards`, as a fraction of window width.
///
/// Warframe centres the block on however many cards it has, so dropping a card pulls both edges in
/// by half a pitch. That is not a detail: on a three-card screen every card sits 121px right of
/// where a four-card reader looks, which is enough for slot 0's crop to straddle the gutter and cut
/// the first title in half.
pub fn card_block_left(cards: usize) -> f32 {
    CARD_LEFT + MAX_CARDS.saturating_sub(cards) as f32 * CARD_PITCH / 2.0
}

/// Width of the card block for a squad of `cards`, as a fraction of window width.
pub fn card_block_width(cards: usize) -> f32 {
    CARD_PITCH * cards.saturating_sub(1) as f32 + CARD_WIDTH
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

pub struct ScreenRewardSource;

impl Default for ScreenRewardSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenRewardSource {
    pub fn new() -> Self {
        Self
    }
}

impl VisualRewardSource for ScreenRewardSource {
    fn choices(&mut self, candidates: &[RewardCatalogEntry]) -> Result<Vec<String>, &'static str> {
        let (_, frame) = capture_game_window()?;
        read_cards_in(&frame, candidates)
            .map(|cards| cards.into_iter().map(|(name, _)| name).collect())
    }
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

/// The same read against an already-decoded frame, which is what the live path has -- the capture
/// never touches the disk now that xcap hands back pixels.
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
    let left = card_block_left(cards);
    let mut read = Vec::with_capacity(cards);
    for slot in 0..cards {
        let (text, crop) = read_region(
            image,
            ((left + CARD_PITCH * slot as f32) * width as f32) as u32,
            (TITLE_TOP * height as f32) as u32,
            (CARD_WIDTH * width as f32) as u32,
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
        #[cfg(debug_assertions)]
        warframe_acquisition::append_debug_line(&format!(
            "[DEBUG-card] cards={cards} slot={slot} raw={text:?} match={matched:?} crop={}",
            if keep_crop {
                crop.display().to_string()
            } else {
                "-".to_owned()
            }
        ));
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

/// The game's window title. Warframe titles its window the same on every platform and under every
/// launcher, which its window *class* does not do -- that is `steam_app_230410` under Steam and
/// `warframe.x64.exe` under bare Wine.
const WINDOW_TITLE: &str = "Warframe";

/// Locate the game window and capture it.
///
/// This used to shell out to `xwininfo -root -tree` and then `import`, which meant a Linux install
/// needed x11-utils and ImageMagick and a Windows one could not work at all. xcap does both, in
/// process, on both platforms -- on Windows through Windows Graphics Capture, which is the only
/// path that can read a D3D swapchain at all; GDI's `BitBlt` returns a black frame.
///
/// The monitor is captured and cropped rather than the window captured directly: xcap's
/// `Window::capture_image` returns a stale frame for game windows on Windows (xcap#131), and a
/// reward screen read from a stale frame is a reward screen read from whatever was on screen a
/// moment ago.
pub(crate) fn capture_game_window() -> Result<(WindowRect, image::DynamicImage), &'static str> {
    let rect = warframe_window_rect()?;
    let monitor = xcap::Monitor::from_point(rect.x, rect.y)
        .map_err(|_| "the game window is not on any monitor")?;
    let (origin_x, origin_y) = (
        monitor.x().map_err(|_| "could not read the monitor")?,
        monitor.y().map_err(|_| "could not read the monitor")?,
    );
    let frame = monitor
        .capture_region(
            rect.x.saturating_sub(origin_x).max(0) as u32,
            rect.y.saturating_sub(origin_y).max(0) as u32,
            rect.width,
            rect.height,
        )
        .map_err(|_| "could not capture the game window")?;
    Ok((rect, image::DynamicImage::ImageRgba8(frame)))
}

/// Where the game's window is, in the desktop's own coordinates.
pub(crate) fn warframe_window_rect() -> Result<WindowRect, &'static str> {
    let windows = xcap::Window::all().map_err(|_| "could not enumerate windows")?;
    let found = largest_warframe_window(windows.iter().filter_map(|window| {
        Some((
            window.title().ok()?,
            WindowRect {
                x: window.x().ok()?,
                y: window.y().ok()?,
                width: window.width().ok()?,
                height: window.height().ok()?,
            },
        ))
    }));
    if let Some(rect) = found {
        return Ok(rect);
    }
    // In Wine's virtual-desktop mode the game window is nested inside the desktop window rather
    // than being a top-level client, and xcap enumerates via `_NET_CLIENT_LIST_STACKING`, which
    // lists only top-level managed clients. Walking the whole root tree is what finds it, and that
    // is what this fallback is for -- it is the configuration the tree walk was written for.
    #[cfg(target_os = "linux")]
    if let Some((_, rect)) = warframe_window_from_xwininfo_tree(&xwininfo_tree()) {
        return Ok(rect);
    }
    Err("no Warframe window found")
}

/// Pick the game's window out of a list of candidates.
///
/// Wine spawns several 1x1 helper windows that share the game's title, and on Windows the launcher
/// briefly holds a window of its own, so the first match is routinely not the game. The largest
/// exact-title match is. The 100px floor drops the helpers before size even matters.
pub fn largest_warframe_window(
    candidates: impl Iterator<Item = (String, WindowRect)>,
) -> Option<WindowRect> {
    candidates
        .filter(|(title, _)| title == WINDOW_TITLE)
        .map(|(_, rect)| rect)
        .filter(|rect| rect.width >= 100 && rect.height >= 100)
        .max_by_key(|rect| u64::from(rect.width) * u64::from(rect.height))
}

#[cfg(target_os = "linux")]
fn xwininfo_tree() -> String {
    Command::new("xwininfo")
        .args(["-root", "-tree"])
        .output()
        .map(|tree| String::from_utf8_lossy(&tree.stdout).into_owned())
        .unwrap_or_default()
}

/// Pick the game's window out of `xwininfo -root -tree` output.
///
/// Each line ends with the window's size-and-offset and then its absolute position:
/// `0x1400003 "Warframe": ("Warframe" "steam_app_230410")  1920x1080+1920+0  +1920+0`
///
/// The absolute position is in X root coordinates, which for an XWayland client is the
/// compositor's own output layout -- a window on a second monitor reports that monitor's offset --
/// so the rectangle can be handed straight to the overlay.
///
/// Wine spawns several 1x1 helper windows that share the game's title, and in virtual-desktop mode
/// the real window is nested rather than top-level, so the largest match wins rather than the
/// first one seen.
pub fn warframe_window_from_xwininfo_tree(tree: &str) -> Option<(String, WindowRect)> {
    tree.lines()
        .filter(|line| line.contains("\"Warframe\":"))
        .filter_map(parse_window_line)
        .filter(|(_, rect)| rect.width >= 100 && rect.height >= 100)
        .max_by_key(|(_, rect)| u64::from(rect.width) * u64::from(rect.height))
}

fn parse_window_line(line: &str) -> Option<(String, WindowRect)> {
    let id = line.split_whitespace().next()?;
    let mut tail = line.split_whitespace().rev();
    let absolute = tail.next()?;
    let size = tail.next()?;
    let (width, rest) = size.split_once('x')?;
    let height: String = rest.chars().take_while(char::is_ascii_digit).collect();
    // A negative offset prints as `+-100`, so the leading `+` is a separator and not a sign.
    let (x, y) = absolute.strip_prefix('+')?.split_once('+')?;
    Some((
        id.to_owned(),
        WindowRect {
            x: x.parse().ok()?,
            y: y.parse().ok()?,
            width: width.parse().ok()?,
            height: height.parse().ok()?,
        },
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
    let program = TESSERACT
        .get()
        .cloned()
        .unwrap_or_else(|| "tesseract".into());
    let mut command = Command::new(&program);
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
    let text = command
        .arg(image)
        .args(["-", "--psm", "11"])
        .output()
        .map_err(|_| "tesseract is not available")?;
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

pub fn best_match(text: &str, candidates: &[RewardCatalogEntry]) -> Option<(String, f32)> {
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
    use super::warframe_window_from_xwininfo_tree;

    /// Real `xwininfo -root -tree` lines. Warframe's IME helpers carry the same class name as the
    /// game window and one of them carries its title too, so picking the first match by name alone
    /// grabs a 1x1 window and captures nothing.
    #[test]
    fn only_the_real_game_window_is_picked_up() {
        let helpers = [
            r#"0x2a00002 "Warframe": ("steam_app_warframe" "steam_app_warframe")  1x1+0+0  +0+0"#,
            r#"0x1e00003 "Warframe": ("steam_app_warframe" "steam_app_warframe")  5x5+0+0  +0+0"#,
            r#"0x1600001 "Warframe": ("steam_app_warframe" "steam_app_warframe")  111x1+8+34  +8+34"#,
        ];
        for helper in helpers {
            assert!(
                warframe_window_from_xwininfo_tree(helper).is_none(),
                "accepted {helper}"
            );
        }

        let game = r#"0x2a00001 "Warframe": ("steam_app_warframe" "steam_app_warframe")  1920x1080+1920+0  +1920+0"#;
        let tree = format!("{}\n{game}\n", helpers.join("\n"));
        let (id, rect) = warframe_window_from_xwininfo_tree(&tree).unwrap();
        assert_eq!(id, "0x2a00001");
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (1920, 0, 1920, 1080)
        );
    }
}
