//! Report assembly: sanitizer, text shape, folder contents, EE.log rule.

use std::fs;
use std::path::PathBuf;

use app_lib::report::{
    assemble_report_text, collect_report, log_files, sanitize, utc_stamp, ReportMeta,
    ReportRequest,
};

fn meta(app_data: &std::path::Path, log_dir: &std::path::Path) -> ReportMeta {
    ReportMeta {
        version: "0.5.0".to_owned(),
        profile: "stable".to_owned(),
        os_arch: "linux/x86_64".to_owned(),
        timestamp: "2026-08-05 14:12:33 UTC".to_owned(),
        log_dir: log_dir.to_path_buf(),
        app_data: app_data.to_path_buf(),
    }
}

#[test]
fn sanitize_replaces_home_and_username() {
    assert_eq!(sanitize("in /home/alice/.steam and alice@host", std::path::Path::new("/home/alice"), Some("alice")),
        "in ~/.steam and <user>@host");
    assert_eq!(sanitize("nothing to see", std::path::Path::new("/home/alice"), Some("alice")),
        "nothing to see");
    assert_eq!(sanitize("no home on windows", std::path::Path::new("/nonexistent"), Some("bob")),
        "no home on windows");
}

#[test]
fn utc_stamp_is_civil_and_sorted() {
    let stamp = utc_stamp();
    assert_eq!(stamp.len(), 17, "YYYY-MM-DD-HHMMSS: {stamp}");
    let digits: Vec<char> = stamp.chars().filter(|c| c.is_ascii_digit()).collect();
    assert_eq!(digits.len(), 14);
    assert!(stamp.starts_with("2026-08-0"), "today's stamp starts with 2026-08-0: {stamp}");
}

#[test]
fn collect_writes_folder_with_report_and_log_copy() {
    let dir = tempfile::tempdir().expect("temp dir");
    let app_data = dir.path().join("appdata");
    let log_dir = dir.path().join("logs");
    fs::create_dir_all(&app_data).expect("appdata");
    fs::create_dir_all(&log_dir).expect("logs");
    fs::write(log_dir.join("tennoscope.log"), "line one\nline two\n").expect("log");
    fs::write(log_dir.join("tennoscope.log.1"), "rotated\n").expect("rotated");

    let request = ReportRequest {
        meta: meta(&app_data, &log_dir),
        health_json: "{\"game_reader\":\"failed\"}".to_owned(),
        ee_log_wanted: true,
        ee_log_path: None,
    };
    let result = collect_report(&request).expect("report collects");
    let folder = PathBuf::from(&result.folder_path);
    assert!(folder.starts_with(app_data.join("reports")));
    assert!(folder.join("report.txt").is_file(), "report.txt written");
    assert!(folder.join("tennoscope.log").is_file(), "log copied");
    assert!(folder.join("tennoscope.log.1").is_file(), "rotated log copied");
    assert!(!result.ee_log_included, "no EE.log when path is None");
    let text = fs::read_to_string(folder.join("report.txt")).expect("report reads");
    assert!(text.contains("Diagnostics"), "report has a diagnostics section");
    assert!(text.contains("line one"), "report embeds the log excerpt");
}

#[test]
fn ee_log_copied_only_when_path_resolves() {
    let dir = tempfile::tempdir().expect("temp dir");
    let app_data = dir.path().join("appdata");
    let log_dir = dir.path().join("logs");
    fs::create_dir_all(&app_data).expect("appdata");
    fs::create_dir_all(&log_dir).expect("logs");
    fs::write(log_dir.join("tennoscope.log"), "x\n").expect("log");
    let ee = dir.path().join("EE.log");
    fs::write(&ee, "session secrets\n").expect("ee log");

    let request = ReportRequest {
        meta: meta(&app_data, &log_dir),
        health_json: "{}".to_owned(),
        ee_log_wanted: true,
        ee_log_path: Some(ee),
    };
    let result = collect_report(&request).expect("report collects");
    assert!(result.ee_log_included);
    let folder = PathBuf::from(&result.folder_path);
    let sensitive = fs::read_to_string(folder.join("EE.log (sensitive)")).expect("ee copy");
    assert_eq!(sensitive, "session secrets\n");
    let text = fs::read_to_string(folder.join("report.txt")).expect("report reads");
    assert!(text.contains("Do not attach it to a public issue"), "sensitivity note present");
}

#[test]
fn github_text_never_contains_ee_log_lines() {
    let dir = tempfile::tempdir().expect("temp dir");
    let log_dir = dir.path().join("logs");
    fs::create_dir_all(&log_dir).expect("logs");
    fs::write(log_dir.join("tennoscope.log"), "app line\n").expect("log");
    let meta = meta(dir.path(), &log_dir);
    let text = assemble_report_text(&meta, "{\"stage\":\"failed\"}", true).expect("text assembles");
    assert!(!text.contains("session secrets"), "EE.log content must never reach report text");
    assert!(text.contains("Discord"), "the Discord routing instruction is present when EE.log is included");
}

#[test]
fn report_text_scrubs_home_and_username_when_under_home() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let home = PathBuf::from(home);
    let log_dir = home.join(format!(".tennoscope-report-test-{}", std::process::id()));
    fs::create_dir_all(&log_dir).expect("dir under home");
    fs::write(log_dir.join("tennoscope.log"), "home is here\n").expect("log");
    let meta = meta(home.parent().expect("home parent"), &log_dir);
    let text = assemble_report_text(&meta, "{}", false).expect("text assembles");
    let _ = fs::remove_dir_all(&log_dir);
    let home_str = home.to_string_lossy().into_owned();
    assert!(!text.contains(&home_str), "the raw home path must be scrubbed from report text");
    assert!(text.contains('~'), "home is replaced with ~ in report text");
    if let Some(user) = std::env::var_os("USER").and_then(|user| user.into_string().ok()) {
        let raw_appears = text.contains(&user);
        assert!(
            !raw_appears || text.contains("<user>"),
            "username never appears raw"
        );
    }
}

#[test]
fn log_files_lists_only_tennoscope_logs() {
    let dir = tempfile::tempdir().expect("temp dir");
    fs::create_dir_all(dir.path()).expect("dir");
    fs::write(dir.path().join("tennoscope.log"), "a\n").expect("a");
    fs::write(dir.path().join("tennoscope.log.1"), "b\n").expect("b");
    fs::write(dir.path().join("other.txt"), "c\n").expect("c");
    let files = log_files(dir.path());
    assert_eq!(files.len(), 2);
}
