#![cfg(target_os = "linux")]

use std::{
    ffi::{OsStr, OsString},
    fs,
    os::unix::fs::symlink,
    path::Path,
};

use app_lib::linux_renderer::{
    RendererPreflightAction, renderer_preflight_action, renderer_relaunch_command,
};
use tempfile::tempdir;

fn add_drm_card(root: &Path, card: &str, driver: &str, boot_vga: bool) {
    let device = root.join(card).join("device");
    fs::create_dir_all(&device).unwrap();
    fs::write(
        device.join("boot_vga"),
        if boot_vga { "1\n" } else { "0\n" },
    )
    .unwrap();
    symlink(
        Path::new("/sys/bus/pci/drivers").join(driver),
        device.join("driver"),
    )
    .unwrap();
}

#[test]
fn proprietary_nvidia_primary_gpu_relaunches_with_dmabuf_disabled() {
    let directory = tempdir().unwrap();
    add_drm_card(directory.path(), "card0", "nvidia", true);

    assert_eq!(
        renderer_preflight_action(directory.path(), false, false),
        RendererPreflightAction::RelaunchWithDmabufDisabled
    );
}

#[test]
fn non_nvidia_and_secondary_nvidia_gpus_keep_dmabuf_enabled() {
    let directory = tempdir().unwrap();
    add_drm_card(directory.path(), "card0", "amdgpu", true);
    add_drm_card(directory.path(), "card1", "nvidia", false);

    assert_eq!(
        renderer_preflight_action(directory.path(), false, false),
        RendererPreflightAction::Continue
    );
}

#[test]
fn nouveau_primary_gpu_keeps_dmabuf_enabled() {
    let directory = tempdir().unwrap();
    add_drm_card(directory.path(), "card0", "nouveau", true);

    assert_eq!(
        renderer_preflight_action(directory.path(), false, false),
        RendererPreflightAction::Continue
    );
}

#[test]
fn explicit_renderer_setting_and_relaunch_marker_prevent_relaunch() {
    let directory = tempdir().unwrap();
    add_drm_card(directory.path(), "card0", "nvidia", true);

    assert_eq!(
        renderer_preflight_action(directory.path(), true, false),
        RendererPreflightAction::Continue
    );
    assert_eq!(
        renderer_preflight_action(directory.path(), false, true),
        RendererPreflightAction::Continue
    );
}

#[test]
fn relaunch_preserves_arguments_and_sets_both_environment_flags() {
    let command = renderer_relaunch_command(
        OsStr::new("/opt/tennoscope"),
        [OsString::from("--one"), OsString::from("two words")],
    );

    assert_eq!(command.get_program(), OsStr::new("/opt/tennoscope"));
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        [OsStr::new("--one"), OsStr::new("two words")]
    );
    let environment = command.get_envs().collect::<Vec<_>>();
    assert!(environment.contains(&(
        OsStr::new("WEBKIT_DISABLE_DMABUF_RENDERER"),
        Some(OsStr::new("1"))
    )));
    assert!(environment.contains(&(
        OsStr::new("TENNOSCOPE_DMABUF_RELAUNCHED"),
        Some(OsStr::new("1"))
    )));
}
