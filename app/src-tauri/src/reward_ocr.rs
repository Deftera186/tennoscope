//! Read the four relic reward cards off the screen.
//!
//! This exists because memory cannot answer the question when the local player is a client. The
//! rewards are resident, but nothing observed links them to a player or a screen slot: there is no
//! per-player response record (the host keeps those, a client does not), pointers to the four
//! reward strings never cluster, and the display names sit far apart with no ordered buffer.
//!
//! Reading the screen sidesteps attribution entirely, because the cards are already in screen
//! order. It is not general OCR either: EE.log names the squad's relics before the screen renders,
//! so each card only has to be matched to the nearest of roughly two dozen known rewards. A
//! garbled read still lands on the right item, which is what makes this trustworthy enough to be
//! the fallback rather than a guess.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use warframe_acquisition::RewardCatalogEntry;

use crate::reward_source::VisualRewardSource;

/// Card geometry as fractions of the window, calibrated from a labelled 1920x1080 reward screen:
/// four cards on a 242px pitch from x=478, titles at y=430. Warframe scales its UI with the
/// window, so fractions carry across resolutions where a pixel table would not.
const CARD_LEFT: f32 = 478.0 / 1920.0;
const CARD_PITCH: f32 = 242.0 / 1920.0;
const CARD_WIDTH: f32 = 240.0 / 1920.0;
const TITLE_TOP: f32 = 418.0 / 1080.0;
/// A long reward name wraps to two lines, and the block is anchored so its first line sits above
/// where a single-line title would. The box has to cover both. A taller crop also reads cleaner:
/// the divider noise a tighter box picked up stops dominating the block.
const TITLE_HEIGHT: f32 = 76.0 / 1080.0;

/// Below this, the read is treated as a failure rather than published as a guess.
const MATCH_FLOOR: f32 = 0.6;

pub struct ScreenRewardSource {
    capture: PathBuf,
}

impl Default for ScreenRewardSource {
    fn default() -> Self {
        Self::new()
    }
}

impl ScreenRewardSource {
    pub fn new() -> Self {
        Self {
            // PPM, not PNG: the capture is thrown away after four crops, and PNG-encoding a
            // 1920x1080 frame costs 1.9s against 0.04s for raw pixels. That is the difference
            // between the overlay landing inside the first second of the screen and not.
            capture: std::env::temp_dir().join("tennoscope-reward-screen.ppm"),
        }
    }
}

impl VisualRewardSource for ScreenRewardSource {
    fn choices(&mut self, candidates: &[RewardCatalogEntry]) -> Result<Vec<String>, &'static str> {
        let window = warframe_window()?;
        capture_window(&window, &self.capture)?;
        let names = read_cards(&self.capture, candidates);
        let _ = std::fs::remove_file(&self.capture);
        names
    }
}

/// Read the four card titles out of a reward-screen image and match each to the relic pool.
///
/// Split out from the capture so it can be exercised against a real labelled screen instead of
/// only against a live game.
pub fn read_cards(
    image: &Path,
    candidates: &[RewardCatalogEntry],
) -> Result<Vec<String>, &'static str> {
    if candidates.is_empty() {
        return Err("no reward candidates");
    }
    let (width, height) = image_size(image)?;
    let mut names = Vec::with_capacity(4);
    for slot in 0..4 {
        let text = read_region(
            image,
            ((CARD_LEFT + CARD_PITCH * slot as f32) * width as f32) as u32,
            (TITLE_TOP * height as f32) as u32,
            (CARD_WIDTH * width as f32) as u32,
            (TITLE_HEIGHT * height as f32) as u32,
        )?;
        let matched = best_match(&text, candidates);
        // Without the raw text a failed read is unattributable: reading the wrong place, reading a
        // screen that is not the reward screen, and reading a card whose name is not in the pool
        // all surface as the same error.
        #[cfg(debug_assertions)]
        warframe_acquisition::append_debug_line(&format!(
            "[DEBUG-card] slot={slot} raw={text:?} match={matched:?}"
        ));
        let (name, score) = matched.ok_or("a reward card read as blank")?;
        if score < MATCH_FLOOR {
            return Err("reward card text did not match the relic pool");
        }
        names.push(name);
    }
    Ok(names)
}

/// The game runs under Proton as an XWayland client, so its window is reachable through plain X11
/// with no compositor portal. Several 1x1 IME helpers share its class name; only the real window
/// has a three-or-more digit geometry.
fn warframe_window() -> Result<String, &'static str> {
    let tree = Command::new("xwininfo")
        .args(["-root", "-tree"])
        .output()
        .map_err(|_| "xwininfo is not available")?;
    let tree = String::from_utf8_lossy(&tree.stdout);
    tree.lines()
        .find(|line| line.contains("\"Warframe\":") && has_window_geometry(line))
        .and_then(|line| line.split_whitespace().next())
        .map(str::to_owned)
        .ok_or("no Warframe window found")
}

fn has_window_geometry(line: &str) -> bool {
    line.split_whitespace().any(|field| {
        field.split_once('x').is_some_and(|(left, right)| {
            left.len() >= 3
                && left.bytes().all(|byte| byte.is_ascii_digit())
                && right
                    .split(['+', '-'])
                    .next()
                    .is_some_and(|value| value.len() >= 3)
        })
    })
}

fn capture_window(window: &str, target: &Path) -> Result<(), &'static str> {
    let status = Command::new("import")
        .args(["-window", window])
        .arg(target)
        .status()
        .map_err(|_| "import is not available")?;
    status
        .success()
        .then_some(())
        .ok_or("could not capture the game window")
}

/// Pixel dimensions of the capture. The live path writes PPM for speed; the fixtures the tests read
/// are PNG.
fn image_size(path: &Path) -> Result<(u32, u32), &'static str> {
    let bytes = std::fs::read(path).map_err(|_| "capture could not be read")?;
    if bytes.starts_with(b"\x89PNG") {
        let header = bytes.get(16..24).ok_or("capture was truncated")?;
        let read = |at: usize| {
            u32::from_be_bytes([header[at], header[at + 1], header[at + 2], header[at + 3]])
        };
        return Ok((read(0), read(4)));
    }
    if !bytes.starts_with(b"P6") {
        return Err("capture was not a PNG or PPM");
    }
    // "P6" then width, height, maxval as whitespace-separated tokens, with '#' comments allowed.
    let header = String::from_utf8_lossy(bytes.get(..128).unwrap_or(&bytes));
    let mut fields = header
        .lines()
        .skip(1)
        .filter(|line| !line.starts_with('#'))
        .flat_map(str::split_whitespace);
    let width = fields.next().and_then(|f| f.parse().ok());
    let height = fields.next().and_then(|f| f.parse().ok());
    width.zip(height).ok_or("capture header was unreadable")
}

fn read_region(
    image: &Path,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
) -> Result<String, &'static str> {
    let crop = std::env::temp_dir().join("tennoscope-reward-crop.png");
    let cropped = Command::new("magick")
        .arg(image)
        .args(["-crop", &format!("{width}x{height}+{x}+{y}"), "+repage"])
        .args(["-colorspace", "gray", "-resize", "300%"])
        .arg(&crop)
        .status()
        .map_err(|_| "magick is not available")?;
    if !cropped.success() {
        return Err("could not crop the reward card");
    }
    let text = Command::new("tesseract")
        .arg(&crop)
        .args(["-", "--psm", "6"])
        .output()
        .map_err(|_| "tesseract is not available")?;
    let _ = std::fs::remove_file(&crop);
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
    use super::has_window_geometry;

    /// Real `xwininfo -root -tree` lines. Warframe's IME helpers carry the same class name as the
    /// game window, so picking the first match by name alone grabs a 1x1 window and captures
    /// nothing.
    #[test]
    fn only_the_real_game_window_has_a_usable_geometry() {
        assert!(has_window_geometry(
            r#"0x2a00001 "Warframe": ("steam_app_warframe" "steam_app_warframe")  1920x1080+1920+0  +1920+0"#
        ));
        for helper in [
            r#"0x2a00002 "Default IME": ("steam_app_warframe" "steam_app_warframe")  1x1+0+0  +0+0"#,
            r#"0x1e00003 (has no name): ("steam_app_warframe" "steam_app_warframe")  5x5+0+0  +0+0"#,
            r#"0x1600001 "Input": ("steam_app_warframe" "steam_app_warframe")  111x1+8+34  +8+34"#,
        ] {
            assert!(!has_window_geometry(helper), "accepted {helper}");
        }
    }
}
