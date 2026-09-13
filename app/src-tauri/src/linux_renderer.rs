use std::{
    ffi::{OsStr, OsString},
    fs,
    path::Path,
    process::Command,
};

pub const RELAUNCH_MARKER: &str = "TENNOSCOPE_DMABUF_RELAUNCHED";
const DMABUF_RENDERER: &str = "WEBKIT_DISABLE_DMABUF_RENDERER";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RendererPreflightAction {
    Continue,
    RelaunchWithDmabufDisabled,
}

pub fn renderer_preflight_action(
    drm_root: &Path,
    renderer_setting_present: bool,
    relaunch_marker_present: bool,
) -> RendererPreflightAction {
    if renderer_setting_present || relaunch_marker_present {
        return RendererPreflightAction::Continue;
    }

    let proprietary_nvidia_is_primary = fs::read_dir(drm_root)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(Result::ok)
        .any(|entry| {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                return false;
            };
            if !name
                .strip_prefix("card")
                .is_some_and(|suffix| suffix.chars().all(|character| character.is_ascii_digit()))
            {
                return false;
            }

            let device = entry.path().join("device");
            let boot_vga =
                fs::read_to_string(device.join("boot_vga")).is_ok_and(|value| value.trim() == "1");
            let proprietary_nvidia = fs::read_link(device.join("driver"))
                .ok()
                .and_then(|driver| driver.file_name().map(OsStr::to_owned))
                .is_some_and(|driver| driver == "nvidia");
            boot_vga && proprietary_nvidia
        });

    if proprietary_nvidia_is_primary {
        RendererPreflightAction::RelaunchWithDmabufDisabled
    } else {
        RendererPreflightAction::Continue
    }
}

pub fn renderer_relaunch_command(
    executable: &OsStr,
    arguments: impl IntoIterator<Item = OsString>,
) -> Command {
    let mut command = Command::new(executable);
    command.args(arguments);
    command.env(DMABUF_RENDERER, "1");
    command.env(RELAUNCH_MARKER, "1");
    command
}

pub fn current_renderer_preflight_action() -> RendererPreflightAction {
    renderer_preflight_action(
        Path::new("/sys/class/drm"),
        std::env::var_os(DMABUF_RENDERER).is_some(),
        std::env::var_os(RELAUNCH_MARKER).is_some(),
    )
}
