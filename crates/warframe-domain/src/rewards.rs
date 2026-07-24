use std::cmp::Reverse;

use serde::{Deserialize, Serialize};

use crate::DomainError;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RewardCandidate {
    pub name: String,
    pub platinum: u32,
    pub ducats: u32,
    pub owned: u32,
    pub mastery_relevant: bool,
    pub confidence: f32,
}

impl RewardCandidate {
    pub fn new(
        name: impl Into<String>,
        platinum: u32,
        ducats: u32,
        owned: u32,
        mastery_relevant: bool,
        confidence: f32,
    ) -> Result<Self, DomainError> {
        let name = name.into();
        if name.trim().is_empty() {
            return Err(DomainError::InvalidName);
        }
        if !confidence.is_finite() || !(0.0..=1.0).contains(&confidence) {
            return Err(DomainError::InvalidConfidence);
        }
        Ok(Self {
            name,
            platinum,
            ducats,
            owned,
            mastery_relevant,
            confidence,
        })
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub struct RewardView {
    pub cards: Vec<RewardCandidate>,
    best_value_index: Option<usize>,
}

impl RewardView {
    pub fn best_value_index(&self) -> Option<usize> {
        self.best_value_index
    }

    pub fn best_value_name(&self) -> Option<&str> {
        self.best_value_index
            .and_then(|index| self.cards.get(index))
            .map(|candidate| candidate.name.as_str())
    }
}

pub struct RewardAdvisor;

impl RewardAdvisor {
    pub fn advise(cards: Vec<RewardCandidate>) -> RewardView {
        let best_value_index = cards
            .iter()
            .enumerate()
            .filter(|(_, candidate)| candidate.confidence >= 0.80)
            .max_by_key(|(index, candidate)| {
                (candidate.platinum, candidate.ducats, Reverse(*index))
            })
            .map(|(index, _)| index);
        RewardView {
            cards,
            best_value_index,
        }
    }
}
