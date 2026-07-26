//! Shared helpers for the app's integration tests.

/// Point instrumented code at a per-test log instead of the one the live app appends to.
///
/// `read_cards`, the visual retry loop and the poller all write to a shared debug log that is the
/// only evidence channel for live reward runs. Tests exercising them append fixture output to that
/// same file, where it is indistinguishable from a real fissure -- which already caused one
/// misreading of a live run's log.
///
/// `Once` because tests share a process and `set_var` must not race against itself.
pub fn isolate_debug_log() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| unsafe {
        std::env::set_var(
            "TENNOSCOPE_DEBUG_LOG",
            std::env::temp_dir().join("tennoscope-test.log"),
        );
    });
}
