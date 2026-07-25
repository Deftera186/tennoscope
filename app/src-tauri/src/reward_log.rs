use std::collections::BTreeSet;

const PROJECTION_PREFIX: &str = "/Lotus/Types/Game/Projections/";
const OPEN_REWARD_SCREEN: &str = "OpenVoidProjectionRewardScreenRMI";
const CLIENT_REWARD: &str = "Client got reward info from ";
const ALL_REWARDS: &str = "Client has reward info for all players now";
const GOT_REWARDS: &str = "ProjectionRewardChoice.lua: Got rewards";
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
}

impl RewardLogMachine {
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
            if (line.contains(ALL_REWARDS) || line.contains(GOT_REWARDS))
                && !self.choices_emitted
                && self.responders.len() > 1
            {
                self.choices_emitted = true;
                return vec![RewardLogEvent::ChoicesReady {
                    expected_choices: self.responders.len(),
                }];
            }
        }
        if line.contains(CLOSED) && self.reward_window_open {
            self.reward_window_open = false;
            self.choices_emitted = false;
            self.responders.clear();
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
