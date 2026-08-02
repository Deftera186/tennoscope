// Prevents additional console window on Windows in release, DO NOT REMOVE!!
// Keyed on the build profile (see `build.rs`), not on `debug_assertions`: a pre-release build is
// `--release` with assertions left on for the tracing, and that must not reintroduce the console.
#![cfg_attr(release_profile, windows_subsystem = "windows")]
#![forbid(unsafe_code)]

fn main() {
    app_lib::run();
}
