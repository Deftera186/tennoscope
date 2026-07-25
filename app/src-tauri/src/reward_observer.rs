use serde::Serialize;
use std::{
    io::Write,
    process::{Command, Stdio},
};
use warframe_acquisition::RewardCatalogEntry;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RewardObservation {
    pub name: String,
    pub confidence: f32,
}

impl RewardObservation {
    pub fn certain(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            confidence: 1.0,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ObserverTransition {
    pub show: bool,
    pub hide: bool,
    pub choices: Vec<RewardObservation>,
}

pub struct RewardObserverState {
    hits_required: u8,
    misses_required: u8,
    hits: u8,
    misses: u8,
    visible: bool,
    candidate: Vec<RewardObservation>,
}

impl RewardObserverState {
    pub fn new(hits_required: u8, misses_required: u8) -> Self {
        Self {
            hits_required: hits_required.max(1),
            misses_required: misses_required.max(1),
            hits: 0,
            misses: 0,
            visible: false,
            candidate: Vec::new(),
        }
    }

    pub fn observe(&mut self, choices: Vec<RewardObservation>) -> ObserverTransition {
        if !(2..=4).contains(&choices.len()) {
            return self.miss();
        }
        self.misses = 0;
        if same_choices(&self.candidate, &choices) {
            self.hits = self.hits.saturating_add(1);
        } else {
            self.candidate = choices.clone();
            self.hits = 1;
        }
        let show = !self.visible && self.hits >= self.hits_required;
        if show {
            self.visible = true;
        }
        ObserverTransition {
            show,
            hide: false,
            choices,
        }
    }

    pub fn miss(&mut self) -> ObserverTransition {
        self.hits = 0;
        self.candidate.clear();
        self.misses = self.misses.saturating_add(1);
        let hide = self.visible && self.misses >= self.misses_required;
        if hide {
            self.visible = false;
        }
        ObserverTransition {
            show: false,
            hide,
            choices: Vec::new(),
        }
    }
}

fn same_choices(left: &[RewardObservation], right: &[RewardObservation]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.name == right.name)
}

pub fn normalize_ocr(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn match_reward_text(text: &str, catalog: &[&str]) -> Vec<RewardObservation> {
    let lines = text
        .lines()
        .map(normalize_ocr)
        .filter(|line| line.len() >= 4)
        .collect::<Vec<_>>();
    let mut matches = Vec::<(usize, RewardObservation)>::new();
    for (line_index, line) in lines.iter().enumerate() {
        let best = catalog
            .iter()
            .filter_map(|name| {
                let normalized = normalize_ocr(name);
                let distance = levenshtein(line, &normalized);
                let length = line.chars().count().max(normalized.chars().count()).max(1);
                let confidence = 1.0 - distance as f32 / length as f32;
                (confidence >= 0.78).then_some((confidence, *name))
            })
            .max_by(|left, right| left.0.total_cmp(&right.0));
        if let Some((confidence, name)) = best
            && !matches.iter().any(|(_, reward)| reward.name == name)
        {
            matches.push((
                line_index,
                RewardObservation {
                    name: name.to_owned(),
                    confidence,
                },
            ));
        }
    }
    matches.sort_by_key(|(line_index, _)| *line_index);
    matches
        .into_iter()
        .map(|(_, reward)| reward)
        .take(4)
        .collect()
}

fn levenshtein(left: &str, right: &str) -> usize {
    let right = right.chars().collect::<Vec<_>>();
    let mut costs = (0..=right.len()).collect::<Vec<_>>();
    for (left_index, left_char) in left.chars().enumerate() {
        let mut previous = costs[0];
        costs[0] = left_index + 1;
        for (right_index, right_char) in right.iter().enumerate() {
            let old = costs[right_index + 1];
            costs[right_index + 1] = if left_char == *right_char {
                previous
            } else {
                1 + previous.min(costs[right_index]).min(old)
            };
            previous = old;
        }
    }
    costs[right.len()]
}

pub fn observe_live_rewards(
    catalog: &[RewardCatalogEntry],
) -> Result<Vec<RewardObservation>, &'static str> {
    let mut grim = Command::new("grim");
    if let Some(region) = focused_reward_region() {
        grim.args(["-g", &region]);
    }
    let screenshot = grim.arg("-").output().map_err(|_| "grim is unavailable")?;
    if !screenshot.status.success() || screenshot.stdout.is_empty() {
        return Err("screen capture failed");
    }
    let mut child = Command::new("tesseract")
        .args(["stdin", "stdout", "--psm", "11", "-l", "eng"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "Tesseract is unavailable")?;
    child
        .stdin
        .take()
        .ok_or("OCR input unavailable")?
        .write_all(&screenshot.stdout)
        .map_err(|_| "OCR input failed")?;
    let output = child.wait_with_output().map_err(|_| "OCR failed")?;
    if !output.status.success() {
        return Err("OCR failed");
    }
    let text = String::from_utf8_lossy(&output.stdout);
    let names = catalog
        .iter()
        .map(|entry| entry.name.as_str())
        .collect::<Vec<_>>();
    Ok(match_reward_text(&text, &names))
}

#[derive(serde::Deserialize)]
struct SwayOutput {
    active: bool,
    focused: bool,
    rect: SwayRect,
}

#[derive(serde::Deserialize)]
struct SwayRect {
    x: i32,
    y: i32,
    width: u32,
    height: u32,
}

fn focused_reward_region() -> Option<String> {
    if let Some(rect) = crate::overlay_window::warframe_window_rect() {
        let x = rect.x + i32::try_from(rect.width * 7 / 100).ok()?;
        let y = rect.y + i32::try_from(rect.height * 8 / 100).ok()?;
        let width = rect.width * 86 / 100;
        let height = rect.height * 43 / 100;
        return Some(format!("{x},{y} {width}x{height}"));
    }
    let output = Command::new("swaymsg")
        .args(["-t", "get_outputs", "-r"])
        .output()
        .ok()?;
    let outputs: Vec<SwayOutput> = serde_json::from_slice(&output.stdout).ok()?;
    let output = outputs
        .iter()
        .find(|output| output.active && output.focused)
        .or_else(|| outputs.iter().find(|output| output.active))?;
    let x = output.rect.x + i32::try_from(output.rect.width * 7 / 100).ok()?;
    let y = output.rect.y + i32::try_from(output.rect.height * 8 / 100).ok()?;
    let width = output.rect.width * 86 / 100;
    let height = output.rect.height * 43 / 100;
    Some(format!("{x},{y} {width}x{height}"))
}
