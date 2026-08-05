//! Shared helpers for the app's integration tests.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// Point instrumented code at a per-test log instead of the one the live app appends to.
///
/// `read_cards`, the visual retry loop and the poller all emit through the `log` crate, which is
/// the only evidence channel for live reward runs. Tests exercising them append fixture output to
/// that same channel, where it is indistinguishable from a real fissure -- which already caused
/// one misreading of a live run's log. Install a logger that writes to a temp file, once per test
/// binary.
pub fn isolate_debug_log() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let path = std::env::var_os("TENNOSCOPE_DEBUG_LOG")
            .map(PathBuf::from)
            .unwrap_or_else(|| std::env::temp_dir().join("tennoscope-test.log"));
        let file = Mutex::new(
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .expect("test log opens"),
        );
        log::set_boxed_logger(Box::new(TestLogger(file))).expect("logger installs once");
        log::set_max_level(log::LevelFilter::Debug);
    });
}

struct TestLogger(Mutex<std::fs::File>);

impl log::Log for TestLogger {
    fn enabled(&self, _metadata: &log::Metadata<'_>) -> bool {
        true
    }
    fn log(&self, record: &log::Record<'_>) {
        if let Ok(mut file) = self.0.lock() {
            let _ = writeln!(file, "{} {}", record.level(), record.args());
        }
    }
    fn flush(&self) {}
}
