//! Log-monitor health recordings log transitions, not steady state.
//!
//! The monitor thread re-records the log-monitor health every second. When the game is not
//! running the steady state is "degraded, EE.log not found" — warning about it on every tick
//! floods the console with lines that carry no new information.

use std::sync::{Mutex, Once};

use app_core::AppCore;

struct Capture;

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static INSTALL: Once = Once::new();

impl log::Log for Capture {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        LINES
            .lock()
            .expect("lines lock")
            .push(record.args().to_string());
    }
    fn flush(&self) {}
}

fn install_capture() {
    INSTALL.call_once(|| {
        log::set_boxed_logger(Box::new(Capture)).expect("logger installs once");
        log::set_max_level(log::LevelFilter::Debug);
    });
    LINES.lock().expect("lines lock").clear();
}

fn lines() -> Vec<String> {
    LINES.lock().expect("lines lock").clone()
}

fn count(haystack: &[String], needle: &str) -> usize {
    haystack.iter().filter(|line| line.contains(needle)).count()
}

#[test]
fn log_monitor_health_logs_only_on_state_transitions() {
    install_capture();
    let mut core = AppCore::in_memory().unwrap();

    // From startup the log monitor is already degraded ("Waiting for Warframe EE.log"),
    // so re-recording the same degraded state each tick must stay silent.
    for _ in 0..3 {
        core.record_log_monitor_degraded("EE.log not found; retrying")
            .unwrap();
    }
    // The game running hbm: first ready is a transition, the second is a re-record.
    for _ in 0..2 {
        core.record_log_monitor_ready().unwrap();
    }
    // A read failure is a new state; repeating it is not.
    for _ in 0..2 {
        core.record_log_monitor_failure("EE.log could not be read")
            .unwrap();
    }
    // Back to degraded: one transition, one line.
    for _ in 0..2 {
        core.record_log_monitor_degraded("EE.log not found; retrying")
            .unwrap();
    }

    let lines = lines();
    assert_eq!(
        count(&lines, "log monitor degraded"),
        1,
        "repeated degraded recordings must not repeat the warning: {lines:?}"
    );
    assert_eq!(
        count(&lines, "log monitor ready"),
        1,
        "repeated ready recordings must not repeat the note: {lines:?}"
    );
    assert_eq!(
        count(&lines, "log monitor failed"),
        1,
        "repeated failure recordings must not repeat the warning: {lines:?}"
    );
}
