use serde::Serialize;

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
    pub publish: bool,
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
            publish: self.hits >= self.hits_required,
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
            publish: false,
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
