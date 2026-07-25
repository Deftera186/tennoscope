use std::collections::BTreeSet;

const PROJECTION_PREFIX: &str = "/Lotus/Types/Game/Projections/";
const OPEN_REWARD_SCREEN: &str = "OpenVoidProjectionRewardScreenRMI";
const CLIENT_REWARD: &str = "Client got reward info from ";
const ALL_REWARDS: &str = "Client has reward info for all players now";
const GOT_REWARDS: &str = "ProjectionRewardChoice.lua: Got rewards";
const RENDERED_REWARD: &str = "ProjectionRewardChoice.lua: Missing icon data!";
const REWARD_TIMER: &str = "ProjectionsCountdown.lua: Initialize timer";
const CLOSED: &str = "ProjectionRewardChoice.lua: Relic reward screen shut down";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RewardLogEvent {
    BaselineRequested { relic_paths: Vec<String> },
    ChoicesReady { expected_choices: usize },
    Closed,
}

#[derive(Default)]
pub struct RewardLogMachine {
    loaded_relics: Vec<String>,
    responders: BTreeSet<String>,
    reward_window_open: bool,
    choices_emitted: bool,
    rewards_received: bool,
    rendered_cards: usize,
    carry: Vec<u8>,
}

impl RewardLogMachine {
    pub fn observe_bytes(&mut self, bytes: &[u8]) -> Vec<RewardLogEvent> {
        self.carry.extend_from_slice(bytes);
        let complete = self
            .carry
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        if complete == 0 {
            if self.carry.len() > 64 * 1024 {
                self.carry.clear();
            }
            return Vec::new();
        }
        let lines = self.carry.drain(..complete).collect::<Vec<_>>();
        String::from_utf8_lossy(&lines)
            .lines()
            .flat_map(|line| self.observe_line(line))
            .collect()
    }

    pub fn observe_line(&mut self, line: &str) -> Vec<RewardLogEvent> {
        if let Some(path) = projection_path(line) {
            if !self.loaded_relics.iter().any(|loaded| loaded == path) {
                self.loaded_relics.push(path.to_owned());
            }
        }
        if line.contains(OPEN_REWARD_SCREEN) && !self.reward_window_open {
            self.reward_window_open = true;
            self.responders.clear();
            self.choices_emitted = false;
            self.rewards_received = false;
            self.rendered_cards = 0;
            return vec![RewardLogEvent::BaselineRequested {
                relic_paths: self.loaded_relics.clone(),
            }];
        }
        if self.reward_window_open {
            if let Some(identity) = line
                .split_once(CLIENT_REWARD)
                .map(|(_, value)| value.trim())
            {
                if !identity.is_empty() {
                    self.responders.insert(identity.to_owned());
                }
            }
            if line.contains(ALL_REWARDS) || line.contains(GOT_REWARDS) {
                self.rewards_received = true;
            }
            if line.contains(RENDERED_REWARD) {
                self.rendered_cards = self.rendered_cards.saturating_add(1);
            }
            if line.contains(REWARD_TIMER) && self.rewards_received && !self.choices_emitted {
                let expected_choices = if self.rendered_cards > 0 {
                    self.rendered_cards
                } else {
                    self.responders.len()
                };
                if expected_choices <= 1 {
                    return Vec::new();
                }
                self.choices_emitted = true;
                return vec![RewardLogEvent::ChoicesReady { expected_choices }];
            }
        }
        if line.contains(CLOSED) && self.reward_window_open {
            self.reward_window_open = false;
            self.choices_emitted = false;
            self.responders.clear();
            self.rewards_received = false;
            self.rendered_cards = 0;
            self.loaded_relics.clear();
            return vec![RewardLogEvent::Closed];
        }
        Vec::new()
    }
}

fn projection_path(line: &str) -> Option<&str> {
    let start = line.find(PROJECTION_PREFIX)?;
    let remainder = &line[start..];
    let end = remainder
        .find(|character: char| character == ')' || character.is_ascii_whitespace())
        .unwrap_or(remainder.len());
    Some(&remainder[..end])
}
