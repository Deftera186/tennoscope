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

// GitHub rejects an issue body over 65,536 characters, and the clipboard report is written to be
// pasted into one. The folder's own report.txt is a file, so it keeps the full excerpt.
pub const LOG_EXCERPT_BYTES: usize = 40 * 1024;

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

pub fn collect_report(request: &ReportRequest) -> Result<CollectedReport, String> {
    let reports = request.meta.app_data.join("reports");
    // `utc_stamp`, not `meta.timestamp`: the meta's stamp is display text a caller chooses, and a
    // path is not the place to find out it contained a colon.
    let folder = reports.join(utc_stamp());
    fs::create_dir_all(&folder)
        .map_err(|error| format!("could not create the report folder: {error}"))?;
    let ee_log_included = request.ee_log_wanted
        && request
            .ee_log_path
            .as_ref()
            .is_some_and(|path| path.is_file());
    if ee_log_included {
        let source = request.ee_log_path.as_ref().expect("checked above");
        fs::copy(source, folder.join("EE.log (sensitive)"))
            .map_err(|error| format!("could not copy EE.log: {error}"))?;
    }
    let report_text = assemble_report_text(&request.meta, &request.health_json, ee_log_included)?;
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
    prune_reports(&reports);
    Ok(CollectedReport {
        report_text,
        folder_path: folder,
        ee_log_included,
    })
}

/// Keep the newest few report folders. Each one holds up to 20 MiB of logs plus a possibly huge
/// EE.log, and a frustrated player presses **Save logs** more than once.
fn prune_reports(reports: &Path) {
    const KEEP: usize = 5;
    let Ok(entries) = fs::read_dir(reports) else {
        return;
    };
    // The names are `YYYY-MM-DD-HHMMSS`, so lexical order is chronological order.
    let mut folders: Vec<PathBuf> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    folders.sort();
    for stale in folders.iter().rev().skip(KEEP) {
        let _ = fs::remove_dir_all(stale);
    }
}

pub fn assemble_report_text(
    meta: &ReportMeta,
    health_json: &str,
    ee_log_included: bool,
) -> Result<String, String> {
    // Windows names these differently, and a sanitizer that reads only the Unix pair is inert
    // there -- `C:\\Users\\TheirRealName\\...` would reach a public issue verbatim.
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let username = std::env::var_os("USER")
        .or_else(|| std::env::var_os("USERNAME"))
        .and_then(|user| user.into_string().ok());
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
    // Deliberately not "attach the folder": the folder can hold EE.log, which must never reach a
    // public issue. Name the files that are safe instead.
    text.push_str("If you used Save logs, attach the tennoscope.log files from the saved folder. Nothing is sent anywhere — this text only leaves the machine by your own paste or attach.\n");
    if ee_log_included {
        text.push_str("EE.log is also in that folder. It contains IPs, email addresses and account handles. Do not attach it to a public issue — send it to the maintainer on Discord (@deftera).\n");
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
        out = out.replace(home, "~");
    }
    if let Some(username) = username.filter(|username| !username.is_empty()) {
        out = out.replace(username, "<user>");
    }
    out
}

/// Civil timestamp `YYYY-MM-DD-HHMMSS` in UTC, for report folder names.
pub fn utc_stamp() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0);
    let days = (seconds / 86_400) as i64;
    let remainder = seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}-{month:02}-{day:02}-{:02}{:02}{:02}",
        remainder / 3600,
        remainder % 3600 / 60,
        remainder % 60
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
    use super::civil_from_days;

    #[test]
    fn civil_from_days_is_exact_on_known_epochs() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(11_022), (2000, 3, 6));
        assert_eq!(civil_from_days(20_670), (2026, 8, 5));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
    }
}
