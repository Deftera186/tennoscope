//! Replay a real relic run's EE.log and assert the poller gets armed in time.
//!
//! Four live runs in a row produced no overlay, and every diagnosis was made by reading code and
//! waiting for the user to play again -- a loop measured in hours that answered one question per
//! iteration. This is the same question asked in milliseconds.
//!
//! The fixture is an unedited slice of EE.log covering one fissure: the four relic loads, the
//! reward screen, and the shutdown. It is filtered to the lines the machine cares about, not
//! rewritten.
//!
//! What matters is the *gap*. The poller is only worth having if relic loading is announced far
//! enough ahead of the reward screen to survive the game's log flush delay, measured at ~7.5s. If
//! `BaselineRequested` does not land before `OpenVoidProjectionRewardScreenRMI`, the poller arms
//! too late and the design is wrong -- not the implementation.

use app_lib::{RewardLogEvent, RewardLogMachine};

const RUN: &str = include_str!("fixtures/relic-run-ee.log");

/// Seconds from the game's own clock at the start of a line, so the fixture stays the source of
/// truth for timing instead of a number copied into the test.
fn timestamp(line: &str) -> Option<f64> {
    line.split_whitespace().next()?.parse().ok()
}

#[test]
fn a_real_run_arms_the_poller_before_the_reward_screen_opens() {
    let mut machine = RewardLogMachine::default();
    let mut armed_at = None;
    let mut screen_at = None;

    for line in RUN.lines() {
        for event in machine.observe_line(line) {
            match event {
                RewardLogEvent::BaselineRequested { .. } if armed_at.is_none() => {
                    armed_at = timestamp(line);
                }
                RewardLogEvent::RewardWindowOpened if screen_at.is_none() => {
                    screen_at = timestamp(line);
                }
                _ => {}
            }
        }
    }

    let armed_at = armed_at.expect("no BaselineRequested: the poller never arms on a real run");
    let screen_at = screen_at.expect("fixture never opened the reward screen");
    let lead = screen_at - armed_at;
    assert!(
        lead > 30.0,
        "poller armed only {lead:.1}s before the screen; the log flush delay alone is ~7.5s"
    );
}

/// The squad's relic pool is what turns a garbled read into the right item, so an empty or
/// single-relic pool silently disables the closed-set match that the whole visual path depends on.
#[test]
fn a_real_run_names_the_squads_relics_before_the_screen() {
    let mut machine = RewardLogMachine::default();
    let mut relics = Vec::new();

    for line in RUN.lines() {
        for event in machine.observe_line(line) {
            match event {
                RewardLogEvent::BaselineRequested { relic_paths } => relics = relic_paths,
                RewardLogEvent::RewardWindowOpened => {
                    assert_eq!(
                        relics.len(),
                        4,
                        "expected four relics by the time the screen opened, got {relics:?}"
                    );
                    return;
                }
                _ => {}
            }
        }
    }
    panic!("fixture never opened the reward screen");
}

/// A second fissure in the same session has to arm again. `baseline_requested` and the relic list
/// are cleared on shutdown; if either leaked, run one would work and every run after it would not
/// -- which is indistinguishable from "the poller never works" unless the replay covers two runs.
#[test]
fn a_second_run_in_the_same_session_arms_again() {
    let mut machine = RewardLogMachine::default();
    let mut arms = 0;

    for _ in 0..2 {
        for line in RUN.lines() {
            for event in machine.observe_line(line) {
                if matches!(event, RewardLogEvent::BaselineRequested { .. }) {
                    arms += 1;
                }
            }
        }
    }

    assert!(
        arms >= 2,
        "only {arms} baseline request(s) across two runs; the second fissure never armed"
    );
}
