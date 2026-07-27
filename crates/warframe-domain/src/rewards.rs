use std::cmp::Reverse;

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::DomainError;

#[derive(Clone, Debug, PartialEq, Serialize)]
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

#[derive(Deserialize)]
struct RewardCandidateDto {
    name: String,
    platinum: u32,
    ducats: u32,
    owned: u32,
    mastery_relevant: bool,
    confidence: f32,
}

impl<'de> Deserialize<'de> for RewardCandidate {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let dto = RewardCandidateDto::deserialize(deserializer)?;
        Self::new(
            dto.name,
            dto.platinum,
            dto.ducats,
            dto.owned,
            dto.mastery_relevant,
            dto.confidence,
        )
        .map_err(D::Error::custom)
    }
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct RewardView {
    cards: Vec<RewardCandidate>,
    best_value_index: Option<usize>,
    best_ducat_index: Option<usize>,
}

impl RewardView {
    pub fn cards(&self) -> &[RewardCandidate] {
        &self.cards
    }

    pub fn best_value_index(&self) -> Option<usize> {
        self.best_value_index
    }

    pub fn best_value_name(&self) -> Option<&str> {
        self.best_value_index
            .and_then(|index| self.cards.get(index))
            .map(|candidate| candidate.name.as_str())
    }

    pub fn best_ducat_index(&self) -> Option<usize> {
        self.best_ducat_index
    }
}

pub struct RewardAdvisor;

impl RewardAdvisor {
    /// Rank the cards twice, because platinum is not the only reason to pick one.
    ///
    /// A relic reward can be worth almost nothing on the market and still be the right take for a
    /// player saving for Baro, and most commons are worth the same 15 ducats so the two orders
    /// disagree often. Ranking once by platinum with ducats as a tiebreak buried that: the ducat
    /// answer was only ever visible when the platinum values happened to tie. Both orders are
    /// published and the choice is left to the player.
    ///
    /// The ducat winner is `None` when nothing on offer is worth any ducats -- four Forma would
    /// otherwise crown one of them for a currency none of them carry.
    pub fn advise(cards: Vec<RewardCandidate>) -> RewardView {
        let certain = |(_, candidate): &(usize, &RewardCandidate)| candidate.confidence >= 0.80;
        let best_value_index = cards
            .iter()
            .enumerate()
            .filter(certain)
            .max_by_key(|(index, candidate)| {
                (candidate.platinum, candidate.ducats, Reverse(*index))
            })
            .map(|(index, _)| index);
        let best_ducat_index = cards
            .iter()
            .enumerate()
            .filter(certain)
            .filter(|(_, candidate)| candidate.ducats > 0)
            .max_by_key(|(index, candidate)| {
                (candidate.ducats, candidate.platinum, Reverse(*index))
            })
            .map(|(index, _)| index);
        RewardView {
            cards,
            best_value_index,
            best_ducat_index,
        }
    }
}
