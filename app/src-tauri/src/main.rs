// Prevents additional console window on Windows in release, DO NOT REMOVE!!
// Keyed on the build profile (see `build.rs`), not on `debug_assertions`: a pre-release build is
// `--release` with assertions left on for the tracing, and that must not reintroduce the console.
#![cfg_attr(release_profile, windows_subsystem = "windows")]
#![forbid(unsafe_code)]

fn main() {
    #[cfg(target_os = "linux")]
    if app_lib::linux_renderer::current_renderer_preflight_action()
        == app_lib::linux_renderer::RendererPreflightAction::RelaunchWithDmabufDisabled
    {
        use std::os::unix::process::CommandExt;

        let mut arguments = std::env::args_os();
        let Some(executable) = arguments.next() else {
            app_lib::run();
            return;
        };
        let error =
            app_lib::linux_renderer::renderer_relaunch_command(&executable, arguments).exec();
        eprintln!("TennoScope could not apply the NVIDIA WebKit workaround: {error}");
    }

    app_lib::run();
}
