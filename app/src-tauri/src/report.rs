//! Report assembly for the Diagnostics report block.
//!
//! Two deliverables, with a hard privacy wall between them:
//! - `report_text` (clipboard / report.txt) is sanitized and never contains
//!   EE.log content.
//! - The report folder additionally carries the raw app log and, only for
//!   acquisition failures, a copy of Warframe's EE.log under a name that
//!   flags it as sensitive. EE.log holds IPs, email addresses and account
//!   handles and must never be attached to a public issue.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

pub const LOG_EXCERPT_BYTES: usize = 256 * 1024;

#[derive(Clone)]
pub struct ReportMeta {
    pub version: String,
    pub profile: String,
    pub os_arch: String,
    pub timestamp: String,
    pub log_dir: PathBuf,
    pub app_data: PathBuf,
}

pub struct ReportRequest {
    pub meta: ReportMeta,
    pub health_json: String,
    pub ee_log_wanted: bool,
    pub ee_log_path: Option<PathBuf>,
}

#[derive(Serialize)]
pub struct CollectedReport {
    pub report_text: String,
    pub folder_path: PathBuf,
    pub ee_log_included: bool,
}

/// What happened to EE.log on the way into a report.
///
/// EE.log is best effort by design: it is sensitive, hard to copy while the
/// game is running, and only wanted when acquisition failed. A failure to copy
/// it must never take the rest of the report down with it.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EeLogState {
    NotRequested,
    Included,
    CopyFailed,
}

impl EeLogState {
    pub fn included(self) -> bool {
        self == Self::Included
    }
}

pub fn collect_report(request: &ReportRequest) -> Result<CollectedReport, String> {
    let folder = request.meta.app_data.join("reports").join(utc_stamp());
    fs::create_dir_all(&folder)
        .map_err(|error| format!("could not create the report folder: {error}"))?;
    let ee_log_wanted = request.ee_log_wanted
        && request
            .ee_log_path
            .as_ref()
            .is_some_and(|path| path.is_file());
    let ee_log_state = if ee_log_wanted {
        let source = request.ee_log_path.as_ref().expect("checked above");
        match copy_ee_log(source, &folder) {
            Ok(()) => EeLogState::Included,
            Err(error) => {
                log::warn!("could not copy EE.log for the report: {error}");
                EeLogState::CopyFailed
            }
        }
    } else {
        EeLogState::NotRequested
    };
    let report_text = assemble_report_text(&request.meta, &request.health_json, ee_log_state)?;
    fs::write(folder.join("report.txt"), &report_text)
        .map_err(|error| format!("could not write report.txt: {error}"))?;
    for file in log_files(&request.meta.log_dir) {
        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| "log file has no name".to_owned())?;
        fs::copy(&file, folder.join(name))
            .map_err(|error| format!("could not copy {name}: {error}"))?;
    }
    Ok(CollectedReport {
        report_text,
        folder_path: folder,
        ee_log_included: ee_log_state.included(),
    })
}

/// The one thing that can go wrong between "EE.log exists" and "EE.log is in
/// the folder": the copy itself, which the running game can block.
fn copy_ee_log(source: &Path, folder: &Path) -> std::io::Result<()> {
    fs::copy(source, folder.join("EE.log (sensitive)"))?;
    Ok(())
}

pub fn assemble_report_text(
    meta: &ReportMeta,
    health_json: &str,
    ee_log_state: EeLogState,
) -> Result<String, String> {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let username = std::env::var_os("USER").and_then(|user| user.into_string().ok());
    let excerpt = log_excerpt(&meta.log_dir);
    let mut text = format!(
        "TennoScope report — {} ({}) — {} — {}\n\n",
        meta.version, meta.profile, meta.os_arch, meta.timestamp
    );
    let log_path = meta.log_dir.join("tennoscope.log").display().to_string();
    text.push_str(&format!(
        "Log file: {}\n\n",
        sanitize(
            &log_path,
            home.as_deref().unwrap_or(Path::new("")),
            username.as_deref()
        )
    ));
    text.push_str("--- Diagnostics ---\n");
    text.push_str(health_json);
    text.push('\n');
    text.push_str(&format!(
        "\n--- Log excerpt (last {} KiB) ---\n",
        LOG_EXCERPT_BYTES / 1024
    ));
    text.push_str(&sanitize(
        &excerpt,
        home.as_deref().unwrap_or(Path::new("")),
        username.as_deref(),
    ));
    text.push('\n');
    text.push_str("\n--- Notes ---\n");
    text.push_str("Attach the saved report folder if you used Save logs. Nothing is sent anywhere — this text only leaves the machine by your own paste or attach.\n");
    match ee_log_state {
        EeLogState::NotRequested => {}
        EeLogState::Included => {
            text.push_str("EE.log is included in the report folder. It contains IPs, email addresses and account handles. Do not attach it to a public issue — send it to the maintainer on Discord (@deftera).\n");
        }
        EeLogState::CopyFailed => {
            text.push_str("EE.log was requested but could not be copied (the game usually keeps it locked) — it is not in this report. If the acquisition issue is urgent, send the report folder to the maintainer on Discord (@deftera) and mention the missing EE.log.\n");
        }
    }
    Ok(text)
}

fn log_excerpt(log_dir: &Path) -> String {
    let path = log_dir.join("tennoscope.log");
    let Ok(bytes) = fs::read(&path) else {
        return "(no log file yet — nothing has been logged this session)".to_owned();
    };
    let start = bytes.len().saturating_sub(LOG_EXCERPT_BYTES);
    String::from_utf8_lossy(&bytes[start..]).into_owned()
}

pub fn log_files(log_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(log_dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with("tennoscope.log"))
        })
        .collect();
    files.sort();
    files
}

pub fn sanitize(text: &str, home: &Path, username: Option<&str>) -> String {
    let mut out = text.to_owned();
    if let Some(home) = home.to_str().filter(|home| !home.is_empty()) {
        out = replace_bounded(&out, home, "~");
    }
    if let Some(username) = username.filter(|username| !username.is_empty()) {
        out = replace_bounded(&out, username, "<user>");
    }
    out
}

/// Replace every occurrence of `needle` that is not glued to a word character.
///
/// A username like "bob" or a home like "/home/bob" also occurs inside other
/// words ("builder", "/home/bob-archive"). Replacing those fragments destroys
/// the report's text, so only free-standing occurrences are scrubbed. A home
/// path still needs its next character to be a path separator (or the end) —
/// scrubbing "/home/bob" out of "/home/bobx" would leave "~x", a path that no
/// longer exists, and a username leaves its own trailing word fragment behind.
fn replace_bounded(text: &str, needle: &str, replacement: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(position) = rest.find(needle) {
        let before = rest[..position].chars().next_back();
        let after = rest[position + needle.len()..].chars().next();
        let head_free = before.is_none_or(|c| !c.is_alphanumeric() && c != '_');
        let tail_free = after.is_none_or(|c| !c.is_alphanumeric() && c != '_');
        if head_free && tail_free {
            out.push_str(&rest[..position]);
            out.push_str(replacement);
        } else {
            out.push_str(&rest[..position + needle.len()]);
        }
        rest = &rest[position + needle.len()..];
    }
    out.push_str(rest);
    out
}

/// Civil timestamp `YYYY-MM-DD-HHMMSSmmm` in UTC, for report folder names.
///
/// The milliseconds make two reports within the same second land in different
/// folders instead of silently overwriting each other.
pub fn utc_stamp() -> String {
    let (year, month, day, hour, minute, second, millis) = utc_parts();
    format!("{year:04}-{month:02}-{day:02}-{hour:02}{minute:02}{second:02}{millis:03}")
}

/// Civil date time `YYYY-MM-DD HH:MM:SS UTC`, for the report header.
pub fn utc_civil() -> String {
    let (year, month, day, hour, minute, second, _) = utc_parts();
    format!("{year:04}-{month:02}-{day:02} {hour:02}:{minute:02}:{second:02} UTC")
}

fn utc_parts() -> (u32, u32, u32, u32, u32, u32, u32) {
    parts_from(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default(),
    )
}

fn parts_from(elapsed: std::time::Duration) -> (u32, u32, u32, u32, u32, u32, u32) {
    let seconds = elapsed.as_secs();
    let days = (seconds / 86_400) as i64;
    let remainder = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    (
        year as u32,
        month,
        day,
        (remainder / 3600) as u32,
        (remainder % 3600 / 60) as u32,
        (remainder % 60) as u32,
        elapsed.subsec_millis(),
    )
}

/// Days since 1970-01-01 to civil date. Howard Hinnant's algorithm.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * month_prime + 2) / 5 + 1) as u32;
    let month = if month_prime < 10 {
        month_prime + 3
    } else {
        month_prime - 9
    } as u32;
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::civil_from_days;

    #[test]
    fn civil_from_days_is_exact_on_known_epochs() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_022), (2000, 3, 6));
        assert_eq!(civil_from_days(20_670), (2026, 8, 5));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }

    #[test]
    fn copy_ee_log_fails_cleanly_on_a_blocked_destination() {
        let dir = tempfile::tempdir().expect("temp dir");
        let source = dir.path().join("EE.log");
        fs::write(&source, "secrets\n").expect("source log");
        fs::create_dir(dir.path().join("EE.log (sensitive)")).expect("blocking dir");
        assert!(super::copy_ee_log(&source, dir.path()).is_err());
    }

    #[test]
    fn utc_stamp_matches_the_clock() {
        let clock = || {
            let (year, month, day, hour, minute, second, _) = super::parts_from(
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default(),
            );
            (
                year as i32,
                month as i32,
                day as i32,
                hour as i32,
                minute as i32,
                second as i32,
            )
        };
        let before = clock();
        let stamp = super::utc_stamp();
        let after = clock();
        let (year, rest) = stamp.split_once('-').expect("year");
        let (month, rest) = rest.split_once('-').expect("month");
        let (day, time) = rest.split_once('-').expect("day");
        let parse = |segment: &str| segment.parse::<i32>().expect("number");
        let fields = (
            parse(year),
            parse(month),
            parse(day),
            parse(&time[..2]),
            parse(&time[2..4]),
            parse(&time[4..6]),
        );
        assert!(
            before == fields || after == fields,
            "stamp's civil fields sit between clock reads before {before:?} and after {after:?}, stamp: {stamp}"
        );
    }
}
