fn main() {
    // The console window is a property of the *build profile*, not of debug assertions. A
    // pre-release build leaves assertions on so the `[DEBUG-…]` tracing survives, and gating the
    // Windows subsystem on `debug_assertions` handed every tester an extra black cmd window.
    println!("cargo::rustc-check-cfg=cfg(release_profile)");
    if std::env::var("PROFILE").as_deref() == Ok("release") {
        println!("cargo::rustc-cfg=release_profile");
    }
    tauri_build::build()
}
