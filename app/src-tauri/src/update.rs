//! Self-update backend: install-kind classification, feed-checked updates.
//!
//! Policy (see temp spec `local://updater-design.md`): only portable installs
//! self-update (a writable AppImage, or per-user Windows NSIS). Everything else
//! gets nudge-only UI, and this module refuses check/download/install for those
//! kinds so a misclassified or hostile frontend cannot trigger an
//! AppImage-over-deb replacement.

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// Channel baked at build time. Release workflow sets
/// `TENNOSCOPE_CHANNEL=beta` for `-rc` tags; dev/local builds read stable.
pub const CHANNEL: &str = match option_env!("TENNOSCOPE_CHANNEL") {
    Some(c) => c,
    None => "stable",
};

const STABLE_FEED: &str =
    "https://github.com/Deftera186/tennoscope/releases/latest/download/latest.json";
const BETA_FEED: &str =
    "https://github.com/Deftera186/tennoscope/releases/latest/download/latest-beta.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallKind {
    Appimage,
    SystemLinux,
    PortableWin,
    SystemWin,
    Unknown,
}

#[derive(Debug, Clone, Serialize)]
pub struct VersionInfo {
    pub version: String,
    pub channel: &'static str,
    pub kind: InstallKind,
    /// False for an AppImage living somewhere read-only (e.g. /opt): the
    /// updater could download but never replace the file.
    pub writable: bool,
    pub updatable: bool,
    /// Present only when the manager and its exact command are known. Anything
    /// else gets the release-page fallback — never an invented command.
    pub manager: Option<String>,
    pub manager_command: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct UpdateSummary {
    pub version: String,
    pub current_version: String,
    pub notes: Option<String>,
    /// RFC 3339, when the feed carries one.
    pub date: Option<String>,
    /// Validated feed name this offer came from. Downloads reuse it, so a
    /// channel toggle after the offer cannot install a different version.
    pub feed: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CheckResult {
    pub kind: InstallKind,
    pub updatable: bool,
    pub update: Option<UpdateSummary>,
}

#[derive(Debug, Clone, Serialize)]
struct ProgressPayload {
    downloaded: u64,
    total: Option<u64>,
}

/// Pure classification core: `exe` is the running binary path, `appimage` is
/// `$APPIMAGE` when present, `writable` says the AppImage file itself can be
/// replaced, `os` is `std::env::consts::OS`, and `system_prefixes` holds the
/// lowercased machine-wide install roots (with trailing separator) on Windows.
/// Kept pure so the matrix is unit tested; the thin wrapper below reads the
/// real environment.
pub fn classify(
    exe: &std::path::Path,
    appimage: Option<&std::path::Path>,
    writable: bool,
    os: &str,
    system_prefixes: &[String],
) -> (InstallKind, bool) {
    if os == "linux" {
        if appimage.is_some() {
            // current_exe under an AppImage lives inside its mount
            // (/tmp/.mount_*; $TMPDIR custom roots fall back to Unknown, which
            // is the safe direction). Without that corroboration a stray
            // $APPIMAGE on a system/dev binary must not promote it to Appimage.
            let under_mount = exe.starts_with("/tmp/")
                && exe.components().any(|c| {
                    c.as_os_str()
                        .to_str()
                        .is_some_and(|s| s.starts_with(".mount_"))
                });
            if under_mount {
                return (InstallKind::Appimage, writable);
            }
        }
        if exe.starts_with("/usr") || exe.starts_with("/opt") {
            return (InstallKind::SystemLinux, false);
        }
        return (InstallKind::Unknown, false);
    }
    if os == "windows" {
        // Per-user NSIS lands under %LOCALAPPDATA%. Machine-wide installs live
        // under the real Program Files dirs (prefix-compared, case-insensitive)
        // or 8.3 short names; anything else user-writable counts as portable.
        // Substring matches require a trailing separator so a `program files
        // backup` folder stays portable.
        let lowered = exe.to_string_lossy().to_lowercase().replace('/', "\\");
        if system_prefixes.iter().any(|p| lowered.starts_with(p))
            || lowered.contains("program files\\")
            || lowered.contains("progra~1\\")
            || lowered.contains("progra~2\\")
        {
            return (InstallKind::SystemWin, false);
        }
        return (InstallKind::PortableWin, true);
    }
    (InstallKind::Unknown, false)
}

/// Portable kinds that may self-update once the file itself is replaceable.
pub fn updatable(kind: InstallKind, writable: bool) -> bool {
    matches!(kind, InstallKind::Appimage | InstallKind::PortableWin) && writable
}
/// Opening for write proves the file can be replaced. Mode bits lie for
/// root-owned files (0644 carries write bits the process cannot use).
pub fn file_writable(path: &std::path::Path) -> bool {
    std::fs::OpenOptions::new().write(true).open(path).is_ok()
}

fn current_classification() -> (InstallKind, bool) {
    let exe = std::env::current_exe().unwrap_or_default();
    let appimage = std::env::var_os("APPIMAGE").map(std::path::PathBuf::from);
    let writable = appimage.as_ref().is_some_and(|p| file_writable(p));
    let prefixes: Vec<String> = if std::env::consts::OS == "windows" {
        ["ProgramFiles", "ProgramFiles(x86)"]
            .iter()
            .filter_map(std::env::var_os)
            .map(|p| {
                let mut s = p.to_string_lossy().to_lowercase().replace('/', "\\");
                if !s.ends_with('\\') {
                    s.push('\\');
                }
                s
            })
            .collect()
    } else {
        Vec::new()
    };
    classify(
        &exe,
        appimage.as_deref(),
        writable,
        std::env::consts::OS,
        &prefixes,
    )
}

/// Beta-channel offer rule: default greater-than plus the equal-version case,
/// so an RC install (which reports plain `X.Y.Z`) is offered the real stable.
/// Frontend dedupes by feed `pub_date` so an installed RC is not re-offered.
pub fn beta_offers(current: &semver::Version, update: &semver::Version) -> bool {
    update >= current
}

fn feed_urls(feed: &str) -> Result<Vec<url::Url>, String> {
    // Scratch-tag verification (`vX.Y.Z-updater.N` prereleases) points a test
    // build at the scratch feed without touching the baked endpoints. Only the
    // local user can set process environment; the frontend cannot reach this.
    if let Some(url) = std::env::var("TENNOSCOPE_FEED_OVERRIDE_URL")
        .ok()
        .filter(|s| !s.is_empty())
    {
        return url
            .parse()
            .map(|u| vec![u])
            .map_err(|e| format!("invalid feed override url: {e}"));
    }
    let raw = match feed {
        "stable" => STABLE_FEED,
        "beta" => BETA_FEED,
        _ => return Err(format!("unknown update feed: {feed}")),
    };
    raw.parse()
        .map(|u| vec![u])
        .map_err(|e| format!("invalid feed url: {e}"))
}

/// `/etc/os-release` ID when it names a manager we ship exact commands for.
/// Today that is only Gentoo (the deftera overlay); every other ID maps to
/// nothing and the UI falls back to the release page.
pub fn manager_for_os_release(os_release: &str) -> Option<(&'static str, &'static str)> {
    let id = os_release
        .lines()
        .filter_map(|line| line.strip_prefix("ID="))
        .next()?
        .trim()
        .trim_matches('"');
    match id {
        "gentoo" => Some((
            "Gentoo (deftera overlay)",
            "sudo emaint sync --repo deftera && sudo emerge --ask --update games-util/tennoscope-bin",
        )),
        _ => None,
    }
}

fn manager_hint(kind: InstallKind) -> (Option<String>, Option<String>) {
    if !matches!(kind, InstallKind::SystemLinux) {
        return (None, None);
    }
    let release = std::fs::read_to_string("/etc/os-release").unwrap_or_default();
    manager_for_os_release(&release)
        .map(|(m, c)| (Some(m.to_owned()), Some(c.to_owned())))
        .unwrap_or((None, None))
}

#[tauri::command]
pub fn get_version_info() -> VersionInfo {
    let (kind, writable) = current_classification();
    let updatable = updatable(kind, writable);
    let (manager, manager_command) = manager_hint(kind);
    VersionInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        channel: CHANNEL,
        kind,
        writable,
        updatable,
        manager,
        manager_command,
    }
}
#[tauri::command]
pub async fn update_check(app: AppHandle, feed: String) -> Result<CheckResult, String> {
    // Read-only for every kind: the nudge card needs version knowledge, and a
    // check installs nothing. The write path (`update_download_and_install`)
    // is where non-portable kinds are refused.
    let (kind, writable) = current_classification();
    let updatable = updatable(kind, writable);
    let urls = feed_urls(&feed)?;
    let mut builder = app.updater_builder();
    builder = builder
        .endpoints(urls)
        .map_err(|e| format!("bad update endpoints: {e}"))?;
    if CHANNEL == "beta" {
        builder =
            builder.version_comparator(|current, update| beta_offers(&current, &update.version));
    }
    let updater = builder.build().map_err(|e| format!("updater setup: {e}"))?;
    let found = updater
        .check()
        .await
        .map_err(|e| format!("update check failed: {e}"))?;
    Ok(CheckResult {
        kind,
        updatable,
        update: found.map(|u| UpdateSummary {
            version: u.version,
            current_version: u.current_version,
            notes: u.body,
            date: u.date.map(|d| {
                d.format(&time::format_description::well_known::Rfc3339)
                    .unwrap_or_default()
            }),
            // `date` renders empty when the feed omits it; callers treat "" as absent.
            feed: feed.clone(),
        }),
    })
}

#[tauri::command]
pub async fn update_download_and_install(
    app: AppHandle,
    feed: String,
) -> Result<UpdateSummary, String> {
    let (kind, writable) = current_classification();
    if !updatable(kind, writable) {
        return Err("this install does not self-update; use your package manager".to_owned());
    }
    let urls = feed_urls(&feed)?;
    let mut builder = app.updater_builder();
    builder = builder
        .endpoints(urls)
        .map_err(|e| format!("bad update endpoints: {e}"))?;
    if CHANNEL == "beta" {
        builder =
            builder.version_comparator(|current, update| beta_offers(&current, &update.version));
    }
    let updater = builder.build().map_err(|e| format!("updater setup: {e}"))?;
    let Some(found) = updater
        .check()
        .await
        .map_err(|e| format!("update check failed: {e}"))?
    else {
        return Err("no update available".to_owned());
    };
    let summary = UpdateSummary {
        version: found.version.clone(),
        current_version: found.current_version.clone(),
        notes: found.body.clone(),
        date: found.date.map(|d| {
            d.format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_default()
        }),
        feed: feed.clone(),
    };
    let handle = app.clone();
    // The chunk callback reports each chunk's length, not a running total.
    let mut downloaded_total: u64 = 0;
    found
        .download_and_install(
            move |chunk, total| {
                downloaded_total += chunk as u64;
                let _ = handle.emit(
                    "update-progress",
                    ProgressPayload {
                        downloaded: downloaded_total,
                        total,
                    },
                );
            },
            || {},
        )
        .await
        .map_err(|e| format!("update install failed: {e}"))?;
    Ok(summary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn v(s: &str) -> semver::Version {
        s.parse().unwrap()
    }

    #[test]
    fn appimage_needs_mount_corroboration() {
        // Under an AppImage current_exe resolves into /tmp/.mount_*, never the
        // .AppImage file: $APPIMAGE alone must not promote a binary.
        let (kind, up) = classify(
            Path::new("/tmp/.mount_A1b2C3/usr/bin/tennoscope"),
            Some(Path::new("/home/u/TennoScope.AppImage")),
            true,
            "linux",
            &[],
        );
        assert_eq!(kind, InstallKind::Appimage);
        assert!(up);
    }

    #[test]
    fn stray_appimage_env_falls_through_to_exe_rules() {
        // A deb binary with $APPIMAGE leaked into its environment stays a
        // system install: the exe is nowhere near a mount namespace.
        let (kind, up) = classify(
            Path::new("/usr/bin/tennoscope"),
            Some(Path::new("/home/u/TennoScope.AppImage")),
            true,
            "linux",
            &[],
        );
        assert_eq!(kind, InstallKind::SystemLinux);
        assert!(!up);
    }

    #[test]
    fn unwritable_appimage_blocks_self_update() {
        let (kind, up) = classify(
            Path::new("/tmp/.mount_A1b2C3/usr/bin/tennoscope"),
            Some(Path::new("/opt/TennoScope.AppImage")),
            false,
            "linux",
            &[],
        );
        assert_eq!(kind, InstallKind::Appimage);
        assert!(!up);
    }

    #[test]
    fn system_linux_prefixes() {
        for exe in ["/usr/bin/tennoscope", "/opt/tennoscope/tennoscope"] {
            let (kind, up) = classify(Path::new(exe), None, false, "linux", &[]);
            assert_eq!(kind, InstallKind::SystemLinux, "{exe}");
            assert!(!up);
        }
    }

    #[test]
    fn dev_and_unknown_builds_never_self_update() {
        let (kind, up) = classify(
            Path::new("/home/u/warframe-helper/target/debug/tennoscope"),
            None,
            false,
            "linux",
            &[],
        );
        assert_eq!(kind, InstallKind::Unknown);
        assert!(!up);
    }

    #[test]
    fn windows_current_user_is_portable() {
        let (kind, up) = classify(
            Path::new(r"C:\Users\u\AppData\Local\TennoScope\tennoscope.exe"),
            None,
            false,
            "windows",
            &[],
        );
        assert_eq!(kind, InstallKind::PortableWin);
        assert!(up);
    }

    #[test]
    fn windows_program_files_variants_are_system() {
        let prefixes = [
            "c:\\program files\\".to_owned(),
            "c:\\program files (x86)\\".to_owned(),
        ];
        for exe in [
            r"C:\Program Files\TennoScope\tennoscope.exe",
            r"C:\Program Files (x86)\TennoScope\tennoscope.exe",
            r"c:\program files\tennoscope\tennoscope.exe",
            // 8.3 short names never contain the long-form substring.
            r"C:\PROGRA~1\TennoScope\tennoscope.exe",
            r"C:\PROGRA~2\TennoScope\tennoscope.exe",
        ] {
            let (kind, up) = classify(Path::new(exe), None, false, "windows", &prefixes);
            assert_eq!(kind, InstallKind::SystemWin, "{exe}");
            assert!(!up);
        }
    }

    #[test]
    fn program_files_backup_folder_stays_portable() {
        let (kind, up) = classify(
            Path::new(r"C:\Users\u\program files backup\tennoscope.exe"),
            None,
            false,
            "windows",
            &[],
        );
        assert_eq!(kind, InstallKind::PortableWin);
        assert!(up);
    }

    #[test]
    fn mount_backup_dir_cannot_corroborate() {
        let (kind, _) = classify(
            Path::new("/home/u/.mount_backup/tennoscope"),
            Some(Path::new("/home/u/TennoScope.AppImage")),
            true,
            "linux",
            &[],
        );
        assert_eq!(kind, InstallKind::Unknown);
    }

    #[test]
    fn open_probe_decides_appimage_replaceability() {
        let dir = tempfile::tempdir().expect("tempdir");
        let writable = dir.path().join("writable.AppImage");
        let locked = dir.path().join("locked.AppImage");
        std::fs::write(&writable, b"x").expect("write");
        std::fs::write(&locked, b"x").expect("write");
        let mut permissions = std::fs::metadata(&locked).expect("meta").permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&locked, permissions).expect("chmod");
        let exe = Path::new("/tmp/.mount_X9y8Z7w/usr/bin/tennoscope");
        let (kind, up) = classify(
            exe,
            Some(&writable),
            super::file_writable(&writable),
            "linux",
            &[],
        );
        assert_eq!(kind, InstallKind::Appimage);
        assert!(up);
        let (kind, up) = classify(
            exe,
            Some(&locked),
            super::file_writable(&locked),
            "linux",
            &[],
        );
        assert_eq!(kind, InstallKind::Appimage);
        assert!(!up);
    }
    #[test]
    fn windows_custom_paths_stay_portable() {
        // A machine-wide install at a custom path outside Program Files is
        // indistinguishable from per-user at classify time, and both are
        // user-writable: pinned as portable so the behavior is explicit.
        let (kind, up) = classify(
            Path::new(r"D:\Apps\TennoScope\tennoscope.exe"),
            None,
            false,
            "windows",
            &[],
        );
        assert_eq!(kind, InstallKind::PortableWin);
        assert!(up);
    }

    #[test]
    fn updatable_tracks_kind_and_writability() {
        assert!(updatable(InstallKind::Appimage, true));
        assert!(updatable(InstallKind::PortableWin, true));
        assert!(!updatable(InstallKind::Appimage, false));
        assert!(!updatable(InstallKind::SystemLinux, true));
        assert!(!updatable(InstallKind::SystemWin, true));
        assert!(!updatable(InstallKind::Unknown, true));
    }

    #[test]
    fn beta_offers_equal_versions_stable_offers_greater_only() {
        assert!(beta_offers(&v("0.12.0"), &v("0.12.0")));
        assert!(beta_offers(&v("0.12.0"), &v("0.12.1")));
        assert!(!beta_offers(&v("0.12.1"), &v("0.12.0")));
    }

    #[test]
    fn feed_whitelist_rejects_arbitrary_urls() {
        assert!(feed_urls("stable").is_ok());
        assert!(feed_urls("beta").is_ok());
        assert!(feed_urls("https://evil.example/feed.json").is_err());
    }

    #[test]
    fn only_gentoo_gets_a_manager_command() {
        let gentoo = "NAME=Gentoo\nID=gentoo\nPRETTY_NAME=\"Gentoo Linux\"\n";
        let (manager, command) = manager_for_os_release(gentoo).expect("gentoo maps");
        assert!(manager.contains("Gentoo"));
        assert!(command.contains("emerge"));
        assert!(manager_for_os_release("NAME=Ubuntu\nID=ubuntu\n").is_none());
        assert!(manager_for_os_release("NAME=Fedora\nID=\"fedora\"\n").is_none());
        assert!(manager_for_os_release("").is_none());
    }
}
