//! Per-poll logging must describe transitions, not steady state.
//!
//! The monitor ticks every second; when the game is not running the steady state is "game
//! absent, EE.log unavailable" and the loop re-records it every tick. Logging each recording
//! floods the console (and the log file) with lines that carry no new information.

use std::sync::{Arc, Mutex, Once};

use app_lib::{LogObservation, MonitorInput, MonitorMachine};

struct Capture;

static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
static INSTALL: Once = Once::new();
static SERIAL: Mutex<()> = Mutex::new(());

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

fn install_capture() -> Arc<Mutex<Vec<String>>> {
    INSTALL.call_once(|| {
        log::set_boxed_logger(Box::new(Capture)).expect("logger installs once");
        log::set_max_level(log::LevelFilter::Debug);
    });
    Arc::new(Mutex::new(Vec::new()))
}

#[test]
fn absent_game_is_not_an_event_when_it_was_never_running() {
    let _serial = SERIAL.lock().expect("serial lock");
    install_capture();
    let mut monitor = MonitorMachine::new(0);
    for tick in 0..5 {
        monitor.tick(MonitorInput::absent(tick));
    }
    let repeated = LINES
        .lock()
        .expect("lines")
        .iter()
        .filter(|line| line.contains("game process gone"))
        .count();
    assert_eq!(
        repeated, 0,
        "a machine that never saw a process must not announce its absence every tick"
    );
}

#[test]
fn disappearance_logs_once_and_not_again_until_it_returns() {
    let _serial = SERIAL.lock().expect("serial lock");
    install_capture();
    let mut monitor = MonitorMachine::new(0);
    monitor.tick(MonitorInput::running(
        0,
        7,
        Some(LogObservation::new("a", 0, Vec::new())),
    ));
    monitor.tick(MonitorInput::absent(1));
    monitor.tick(MonitorInput::absent(2));
    monitor.tick(MonitorInput::absent(3));
    let gone = LINES
        .lock()
        .expect("lines")
        .iter()
        .filter(|line| line.contains("game process gone, resetting"))
        .count();
    assert_eq!(
        gone, 1,
        "the disappearance is one transition, not one line per tick"
    );
}

#[test]
fn no_tick_chatter_line_is_emitted_per_poll() {
    let _serial = SERIAL.lock().expect("serial lock");
    install_capture();
    let mut monitor = MonitorMachine::new(0);
    monitor.tick(MonitorInput::running(
        0,
        7,
        Some(LogObservation::new("a", 1, b"x\n".to_vec())),
    ));
    monitor.tick(MonitorInput::running(1, 7, None));
    let chatter = LINES
        .lock()
        .expect("lines")
        .iter()
        .filter(|line| line.starts_with("monitor: tick"))
        .count();
    assert_eq!(
        chatter, 0,
        "a per-poll tick line would flood the console on long sessions"
    );
}
