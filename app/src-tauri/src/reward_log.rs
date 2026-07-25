const PROJECTION_PREFIX: &str = "/Lotus/Types/Game/Projections/";
const OPEN_REWARD_SCREEN: &str = "OpenVoidProjectionRewardScreen";
const CLIENT_REWARD: &str = "Client got reward info from ";
const HOST_REWARD: &str = "Host got reward info from ";
const WAITING_REWARD: &str = "Still waiting on response from ";
const CLIENT_ALL_REWARDS: &str = "Client has reward info for all players now";
const HOST_ALL_REWARDS: &str = "Host has reward info for all players now";
const GOT_REWARDS: &str = "ProjectionRewardChoice.lua: Got rewards";
const RENDERED_REWARD: &str = "ProjectionRewardChoice.lua: Missing icon data!";
const REWARD_TIMER: &str = "ProjectionsCountdown.lua: Initialize timer";
const CLOSED: &str = "ProjectionRewardChoice.lua: Relic reward screen shut down";
const GETS_REWARD: &str = " gets reward ";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RewardLogEvent {
    BaselineRequested {
        relic_paths: Vec<String>,
    },
    ChoicesReady {
        expected_choices: usize,
        local_reward_path: Option<String>,
    },
    ResponderReceived {
        identity: String,
        is_local: bool,
    },
    ResponsesComplete {
        responders: Vec<String>,
        screen_order: Vec<String>,
        local_reward_path: Option<String>,
        local_identity: Option<String>,
    },
    Closed,
}

#[derive(Default)]
pub struct RewardLogMachine {
    loaded_relics: Vec<String>,
    responders: Vec<String>,
    squad_ring: Vec<String>,
    reward_window_open: bool,
    choices_emitted: bool,
    rewards_received: bool,
    responses_complete_emitted: bool,
    rendered_cards: usize,
    local_reward_path: Option<String>,
    local_identity: Option<String>,
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
                if self.loaded_relics.len() >= 2 && !self.reward_window_open {
                    return vec![RewardLogEvent::BaselineRequested {
                        relic_paths: self.loaded_relics.clone(),
                    }];
                }
            }
        }
        if line.contains(OPEN_REWARD_SCREEN) && !self.reward_window_open {
            self.reward_window_open = true;
            self.responders.clear();
            self.squad_ring.clear();
            self.choices_emitted = false;
            self.rewards_received = false;
            self.responses_complete_emitted = false;
            self.rendered_cards = 0;
            self.local_reward_path = None;
            self.local_identity = None;
            return Vec::new();
        }
        if self.reward_window_open {
            if let Some((prefix, path)) = line.split_once(GETS_REWARD) {
                let path = path.trim();
                if path.starts_with("/Lotus/") {
                    self.local_reward_path = Some(path.to_owned());
                    self.local_identity = prefix
                        .split_whitespace()
                        .next_back()
                        .filter(|identity| {
                            identity.len() == 24
                                && identity.bytes().all(|byte| byte.is_ascii_hexdigit())
                        })
                        .map(str::to_owned);
                }
            }
            if let Some(identity) = [CLIENT_REWARD, HOST_REWARD]
                .into_iter()
                .find_map(|marker| line.split_once(marker).map(|(_, value)| value.trim()))
            {
                if !identity.is_empty() {
                    if !self.responders.iter().any(|known| known == identity) {
                        self.responders.push(identity.to_owned());
                        if self.squad_ring.is_empty() {
                            self.squad_ring.push(identity.to_owned());
                        }
                        return vec![RewardLogEvent::ResponderReceived {
                            identity: identity.to_owned(),
                            is_local: self.local_identity.as_deref() == Some(identity),
                        }];
                    }
                }
            }
            if let Some((_, identity)) = line.split_once(WAITING_REWARD) {
                let identity = identity.trim();
                if !identity.is_empty() && !self.squad_ring.iter().any(|known| known == identity) {
                    self.squad_ring.push(identity.to_owned());
                }
            }
            if line.contains(CLIENT_ALL_REWARDS)
                || line.contains(HOST_ALL_REWARDS)
                || line.contains(GOT_REWARDS)
            {
                self.rewards_received = true;
                if !self.responses_complete_emitted {
                    self.responses_complete_emitted = true;
                    for identity in &self.responders {
                        if !self.squad_ring.contains(identity) {
                            self.squad_ring.push(identity.clone());
                        }
                    }
                    let mut screen_order = self.squad_ring.clone();
                    if let Some(local_identity) = self.local_identity.as_deref()
                        && let Some(local_index) = screen_order
                            .iter()
                            .position(|identity| identity == local_identity)
                    {
                        screen_order.rotate_left(local_index);
                    }
                    return vec![RewardLogEvent::ResponsesComplete {
                        responders: self.responders.clone(),
                        screen_order,
                        local_reward_path: self.local_reward_path.clone(),
                        local_identity: self.local_identity.clone(),
                    }];
                }
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
                return vec![RewardLogEvent::ChoicesReady {
                    expected_choices,
                    local_reward_path: self.local_reward_path.clone(),
                }];
            }
        }
        if line.contains(CLOSED) && self.reward_window_open {
            self.reward_window_open = false;
            self.choices_emitted = false;
            self.responders.clear();
            self.squad_ring.clear();
            self.rewards_received = false;
            self.responses_complete_emitted = false;
            self.rendered_cards = 0;
            self.local_reward_path = None;
            self.local_identity = None;
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
