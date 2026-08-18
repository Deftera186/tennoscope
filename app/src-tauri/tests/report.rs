//! Report assembly: sanitizer, text shape, folder contents, EE.log rule.

use std::fs;
use std::path::PathBuf;

use app_lib::report::{
    EeLogState, ReportMeta, ReportRequest, assemble_report_text, collect_report, log_files,
    sanitize, utc_stamp,
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
    assert_eq!(
        sanitize(
            "in /home/alice/.steam and alice@host",
            std::path::Path::new("/home/alice"),
            Some("alice")
        ),
        "in ~/.steam and <user>@host"
    );
    assert_eq!(
        sanitize(
            "nothing to see",
            std::path::Path::new("/home/alice"),
            Some("alice")
        ),
        "nothing to see"
    );
    assert_eq!(
        sanitize(
            "no home on windows",
            std::path::Path::new("/nonexistent"),
            Some("bob")
        ),
        "no home on windows"
    );
}

#[test]
fn sanitize_ignores_embedded_fragments() {
    let home = std::path::Path::new("/home/alice");
    assert_eq!(
        sanitize("builder and alicex and alice", home, Some("alice")),
        "builder and alicex and <user>",
        "username only scrubbed at word boundaries"
    );
    assert_eq!(
        sanitize(
            "/home/alicebackup and /home/alice/store",
            home,
            Some("alice")
        ),
        "/home/alicebackup and ~/store",
        "home only scrubbed when followed by a separator or the end"
    );
}

#[test]
fn utc_stamp_is_civil_and_sorted() {
    let stamp = utc_stamp();
    assert_eq!(stamp.len(), 20, "YYYY-MM-DD-HHMMSSmmm: {stamp}");
    assert!(stamp.is_ascii(), "stamp is plain ASCII: {stamp}");
    let digits: Vec<char> = stamp.chars().filter(|c| c.is_ascii_digit()).collect();
    assert_eq!(digits.len(), 17);
    assert_eq!(stamp.chars().filter(|c| *c == '-').count(), 3);
    let (year, rest) = stamp.split_once('-').expect("year");
    assert_eq!(year.len(), 4);
    assert!(year.chars().all(|c| c.is_ascii_digit()));
    let (month, rest) = rest.split_once('-').expect("month");
    assert_eq!(month, "08", "month is zero-padded: {month}");
    let month: u32 = month.parse().expect("month number");
    assert!((1..=12).contains(&month));
    let (day, time) = rest.split_once('-').expect("day");
    let day: u32 = day.parse().expect("day number");
    assert!((1..=31).contains(&day));
    let (hour, rest) = time.split_at(2);
    let (minutes, seconds_ms) = rest.split_at(2);
    let (seconds, millis) = seconds_ms.split_at(2);
    assert_eq!(millis.len(), 3, "milliseconds present: {stamp}");
    let hour: u32 = hour.parse().expect("hour number");
    let minutes: u32 = minutes.parse().expect("minutes number");
    let seconds: u32 = seconds.parse().expect("seconds number");
    let millis: u32 = millis.parse().expect("millis number");
    assert!(hour < 24, "hour in range: {stamp}");
    assert!(minutes < 60, "minutes in range: {stamp}");
    assert!(seconds < 60, "seconds in range: {stamp}");
    assert!(millis < 1000, "millis in range: {stamp}");
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
    assert!(
        folder.join("tennoscope.log.1").is_file(),
        "rotated log copied"
    );
    assert!(!result.ee_log_included, "no EE.log when path is None");
    let text = fs::read_to_string(folder.join("report.txt")).expect("report reads");
    assert!(
        text.contains("Diagnostics"),
        "report has a diagnostics section"
    );
    assert!(
        !text.contains("line one"),
        "report.txt no longer embeds the log body; the raw log sits beside it"
    );
    assert_eq!(
        fs::read_to_string(folder.join("tennoscope.log")).unwrap(),
        "line one\nline two\n",
        "the raw log is still copied into the folder"
    );
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
    let sensitive = fs::read_to_string(folder.join("EE.log (sanitized)")).expect("ee copy");
    assert_eq!(sensitive, "session secrets\n");
    let text = fs::read_to_string(folder.join("report.txt")).expect("report reads");
    assert!(
        text.contains("You can attach it to a GitHub issue"),
        "sanitization note present"
    );
}

#[test]
fn github_text_never_contains_ee_log_lines() {
    let dir = tempfile::tempdir().expect("temp dir");
    let log_dir = dir.path().join("logs");
    fs::create_dir_all(&log_dir).expect("logs");
    fs::write(log_dir.join("tennoscope.log"), "app line\n").expect("log");
    let meta = meta(dir.path(), &log_dir);
    let text = assemble_report_text(&meta, "{\"stage\":\"failed\"}", EeLogState::Included)
        .expect("text assembles");
    assert!(
        !text.contains("session secrets"),
        "EE.log content must never reach report text"
    );
    assert!(
        text.contains("GitHub issue"),
        "the attach-to-issue instruction is present when EE.log is included"
    );
}

#[test]
fn report_text_scrubs_home_and_username_when_under_home() {
    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let home = PathBuf::from(home);
    let log_dir = home.join(format!(".tennoscope-report-test-{}", std::process::id()));
    fs::create_dir_all(&log_dir).expect("dir under home");
    fs::write(
        log_dir.join("tennoscope.log"),
        format!("[WARN] app log lives under {}\n", log_dir.display()),
    )
    .expect("log");
    let meta = meta(home.parent().expect("home parent"), &log_dir);
    let text = assemble_report_text(&meta, "{}", EeLogState::NotRequested).expect("text assembles");
    let _ = fs::remove_dir_all(&log_dir);
    let home_str = home.to_string_lossy().into_owned();
    assert!(
        !text.contains(&home_str),
        "the raw home path must be scrubbed from report text"
    );
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

/// Each report folder holds up to 20 MiB of logs plus a possibly huge EE.log, and pressing
/// **Save logs** is what a player does when nothing is working -- repeatedly.
#[test]
fn old_report_folders_are_pruned() {
    let dir = tempfile::tempdir().expect("temp dir");
    let app_data = dir.path().join("appdata");
    let log_dir = dir.path().join("logs");
    fs::create_dir_all(&log_dir).expect("logs");
    fs::write(log_dir.join("tennoscope.log"), "line\n").expect("log");
    let reports = app_data.join("reports");
    fs::create_dir_all(&reports).expect("reports");
    for stamp in [
        "2020-01-01-000001",
        "2020-01-01-000002",
        "2020-01-01-000003",
    ] {
        fs::create_dir_all(reports.join(stamp)).expect("old report");
    }

    collect_report(&ReportRequest {
        meta: meta(&app_data, &log_dir),
        health_json: "{}".to_owned(),
        ee_log_wanted: false,
        ee_log_path: None,
    })
    .expect("collect");

    let mut names: Vec<String> = fs::read_dir(&reports)
        .expect("read reports")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    assert_eq!(names.len(), 4, "three old plus the new one: {names:?}");

    for stamp in ["2020-01-01-000004", "2020-01-01-000005"] {
        fs::create_dir_all(reports.join(stamp)).expect("old report");
    }
    collect_report(&ReportRequest {
        meta: meta(&app_data, &log_dir),
        health_json: "{}".to_owned(),
        ee_log_wanted: false,
        ee_log_path: None,
    })
    .expect("collect");
    let names: Vec<String> = fs::read_dir(&reports)
        .expect("read reports")
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names.len(), 5, "the newest five survive: {names:?}");
    assert!(
        !names.iter().any(|name| name == "2020-01-01-000001"),
        "the oldest is gone: {names:?}"
    );
}

fn write_log(log_dir: &std::path::Path, lines: &[&str]) {
    std::fs::create_dir_all(log_dir).unwrap();
    std::fs::write(log_dir.join("tennoscope.log"), lines.join("\n")).unwrap();
}

#[test]
fn error_tail_keeps_only_warn_and_error_lines_in_order() {
    let dir = std::env::temp_dir().join(format!("report-tail-order-{}", std::process::id()));
    write_log(
        &dir,
        &[
            "[2026-08-08][10:00:00][app][INFO] reader ready",
            "[2026-08-08][10:00:01][app][WARN] capture unreachable",
            "[2026-08-08][10:00:02][app][ERROR] schema_validation failed",
            "[2026-08-08][10:00:03][app][DEBUG] probing window",
        ],
    );
    let tail = app_lib::report::log_error_tail(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(
        tail,
        vec![
            "[2026-08-08][10:00:01][app][WARN] capture unreachable".to_owned(),
            "[2026-08-08][10:00:02][app][ERROR] schema_validation failed".to_owned(),
        ]
    );
}

#[test]
fn error_tail_collapses_consecutive_duplicates() {
    let dir = std::env::temp_dir().join(format!("report-tail-dedupe-{}", std::process::id()));
    write_log(
        &dir,
        &[
            "[2026-08-08][10:00:01][monitor][WARN] EE.log not found; retrying",
            "[2026-08-08][10:00:02][monitor][WARN] EE.log not found; retrying",
            "[2026-08-08][10:00:03][monitor][WARN] EE.log not found; retrying",
            "[2026-08-08][10:00:04][monitor][WARN] EE.log not found; retrying",
        ],
    );
    let tail = app_lib::report::log_error_tail(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(
        tail.len(),
        1,
        "per-second warnings collapse to a single line"
    );
}

#[test]
fn error_tail_caps_at_twenty_lines_and_reads_the_last_window() {
    let dir = std::env::temp_dir().join(format!("report-tail-cap-{}", std::process::id()));
    let lines: Vec<String> = (0..25)
        .map(|i| format!("[2026-08-08][10:00:{i:02}][app][WARN] fault {i}"))
        .collect();
    write_log(&dir, &lines.iter().map(String::as_str).collect::<Vec<_>>());
    let tail = app_lib::report::log_error_tail(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(tail.len(), app_lib::report::LOG_TAIL_LINES);
    assert!(
        tail.last().unwrap().ends_with("fault 24"),
        "the newest line is the last in the returned list"
    );
}

#[test]
fn error_tail_window_starts_at_a_line_boundary() {
    let dir = std::env::temp_dir().join(format!("report-tail-boundary-{}", std::process::id()));
    let mut content = "x".repeat(app_lib::report::LOG_TAIL_WINDOW_BYTES + 1);
    content.push_str("[WARN] junk embedded in a giant line");
    content.push('\n');
    content.push_str("[2026-08-08][10:00:01][app][WARN] capture unreachable");
    write_log(&dir, &[&content]);
    let tail = app_lib::report::log_error_tail(&dir);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(
        tail,
        vec!["[2026-08-08][10:00:01][app][WARN] capture unreachable".to_owned()],
        "a window cut inside a line must not leak a truncated fragment"
    );
}


const ROW_JSON: &str = r#"{
  "game_reader": {"state": "degraded", "message": "Warframe is not running", "last_success": null},
  "log_monitor": {"state": "degraded", "message": "Waiting for Warframe", "last_success": null},
  "capture": {"state": "ready", "message": "Reward observer ready", "last_success": null},
  "catalog": {"state": "ready", "message": "Catalog ready", "last_success": "2026-07-27T00:00:00Z"},
  "market": {"state": "ready", "message": "Market ready", "last_success": null},
  "collection_prices": {"state": "degraded", "message": "Collection price dump has not loaded yet", "last_success": null},
  "database": {"state": "ready", "message": "SQLite database available", "last_success": null},
  "market_account": {"state": "idle", "message": "Not linked", "last_success": null},
  "acquisition_stages": [
    {"stage": "schema_validation", "state": "failed", "message": "Inventory snapshot was invalid"},
    {"stage": "memory_permission", "state": "ready", "message": "memory read ready"}
  ]
}"#;

#[test]
fn assemble_report_text_renders_human_readable_rows_only() {
    let home = std::env::temp_dir();
    let log_dir = home.join(format!("assemble-rows-{}", std::process::id()));
    std::fs::create_dir_all(&log_dir).unwrap();
    let text = app_lib::report::assemble_report_text(
        &meta(&home, &log_dir),
        ROW_JSON,
        app_lib::report::EeLogState::NotRequested,
    )
    .expect("text builds");
    let _ = std::fs::remove_dir_all(&log_dir);
    assert!(text.contains("Game reader: degraded — Warframe is not running"));
    assert!(text.contains("EE.log: degraded — Waiting for Warframe"));
    assert!(text.contains("Catalog: ready — Catalog ready"));
    assert!(text.contains("Market account: idle — Not linked"));
    assert!(!text.contains("game_reader"), "raw keys must not appear");
    assert!(
        !text.contains("last_success"),
        "the stamp is a report row, not a dump"
    );
    assert!(text.contains("Diagnostics"));
    assert!(
        !text.contains("Log file:"),
        "no filesystem provenance in the paste"
    );
}

#[test]
fn assemble_report_text_lists_only_broken_acquisition_stages() {
    let home = std::env::temp_dir();
    let log_dir = home.join(format!("assemble-stages-{}", std::process::id()));
    std::fs::create_dir_all(&log_dir).unwrap();
    let text = app_lib::report::assemble_report_text(
        &meta(&home, &log_dir),
        ROW_JSON,
        app_lib::report::EeLogState::NotRequested,
    )
    .expect("text builds");
    std::fs::remove_dir_all(&log_dir).unwrap();
    assert!(text.contains("schema_validation: failed — Inventory snapshot was invalid"));
    assert!(
        !text.contains("memory_permission"),
        "ready stages stay out of the report"
    );
}

#[test]
fn assemble_report_text_only_mentions_ee_log_when_included() {
    let home = std::env::temp_dir();
    let log_dir = home.join(format!("assemble-ee-note-{}", std::process::id()));
    std::fs::create_dir_all(&log_dir).unwrap();
    let quiet = app_lib::report::assemble_report_text(
        &meta(&home, &log_dir),
        ROW_JSON,
        app_lib::report::EeLogState::NotRequested,
    )
    .expect("copy text builds");
    assert!(
        !quiet.contains("Notes"),
        "copy text carries no notes section"
    );

    let included = app_lib::report::assemble_report_text(
        &meta(&home, &log_dir),
        ROW_JSON,
        app_lib::report::EeLogState::Included,
    )
    .expect("folder text builds");
    std::fs::remove_dir_all(&log_dir).unwrap();
    assert!(included.contains("EE.log is included in the report folder"));
}

#[test]
fn sanitize_ee_log_strips_ipv4_addresses() {
    let input = "Connected to 192.168.1.100:6695\nServer: 203.0.113.42\n";
    let output = app_lib::report::sanitize_ee_log(input);
    assert!(!output.contains("192.168.1.100"));
    assert!(!output.contains("203.0.113.42"));
    assert!(output.contains("[redacted-ip]"));
}

#[test]
fn sanitize_ee_log_strips_ipv6_addresses() {
    let input = "Host: 2001:0db8:85a3:0000:0000:8a2e:0370:7334 connected\n";
    let output = app_lib::report::sanitize_ee_log(input);
    assert!(!output.contains("2001:0db8"));
    assert!(output.contains("[redacted-ip]"));
}

#[test]
fn sanitize_ee_log_strips_email_addresses() {
    let input = "Account: player@example.com logged in\nContact: user.name+tag@domain.co.uk\n";
    let output = app_lib::report::sanitize_ee_log(input);
    assert!(!output.contains("player@example.com"));
    assert!(!output.contains("user.name+tag@domain.co.uk"));
    assert!(output.contains("[redacted-email]"));
}

#[test]
fn sanitize_ee_log_preserves_non_pii_content() {
    let input = "Inventory sync done\nLoaded relic: Lith A1\nMission: 4 players\n";
    let output = app_lib::report::sanitize_ee_log(input);
    assert_eq!(input, output);
}
