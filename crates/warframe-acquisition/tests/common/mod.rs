//! Shared test helpers for the acquisition integration tests.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

/// A log-crate logger that appends formatted lines to a file, so instrumented
/// scans keep an evidence trail that assertions can read back. One file per
/// test binary; installed at most once per process.
pub fn install_test_logger() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let path = std::env::var_os("TENNOSCOPE_TEST_LOG")
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
