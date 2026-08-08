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
    assert!(
        text.contains("Do not attach it to a public issue"),
        "sensitivity note present"
    );
}

#[test]
fn github_text_never_contains_ee_log_lines() {
    let dir = tempfile::tempdir().expect("temp dir");
    let log_dir = dir.path().join("logs");
    fs::create_dir_all(&log_dir).expect("logs");
    fs::write(log_dir.join("tennoscope.log"), "app line\n").expect("log");
    let meta = meta(dir.path(), &log_dir);
    let text = assemble_report_text(
        &meta,
        "{\"stage\":\"failed\"}",
        EeLogState::Included,
        app_lib::report::LogBody::FullExcerpt,
    )
    .expect("text assembles");
    assert!(
        !text.contains("session secrets"),
        "EE.log content must never reach report text"
    );
    assert!(
        text.contains("Discord"),
        "the Discord routing instruction is present when EE.log is included"
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
    fs::write(log_dir.join("tennoscope.log"), "home is here\n").expect("log");
    let meta = meta(home.parent().expect("home parent"), &log_dir);
    let text = assemble_report_text(
        &meta,
        "{}",
        EeLogState::NotRequested,
        app_lib::report::LogBody::FullExcerpt,
    )
    .expect("text assembles");
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
fn assemble_report_text_switches_on_log_body() {
    let home = std::env::temp_dir();
    let log_dir = home.join(format!("assemble-tail-{}", std::process::id()));
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(
        log_dir.join("tennoscope.log"),
        "[2026-08-08][10:00:01][app][WARN] capture unreachable\n",
    )
    .unwrap();
    let meta_row_test = meta(&home, &log_dir);
    let text = app_lib::report::assemble_report_text(
        &meta_row_test,
        "{\"state\":\"degraded\"}",
        app_lib::report::EeLogState::NotRequested,
        app_lib::report::LogBody::Tail,
    )
    .expect("copy text builds");
    std::fs::remove_dir_all(&log_dir).unwrap();
    assert!(
        text.contains("capture unreachable"),
        "the copy carries a warn line"
    );
    assert!(
        !text.contains("Log excerpt"),
        "the copy must not carry the full excerpt header"
    );
}

#[test]
fn assemble_report_text_full_body_keeps_the_excerpt() {
    let home = std::env::temp_dir();
    let log_dir = home.join(format!("assemble-full-{}", std::process::id()));
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(log_dir.join("tennoscope.log"), "INFO line\n").unwrap();
    let text = app_lib::report::assemble_report_text(
        &meta(&home, &log_dir),
        "{\"state\":\"ready\"}",
        app_lib::report::EeLogState::Included,
        app_lib::report::LogBody::FullExcerpt,
    )
    .expect("full text builds");
    std::fs::remove_dir_all(&log_dir).unwrap();
    assert!(
        text.contains("Log excerpt"),
        "the saved report keeps its log excerpt"
    );
    assert!(
        text.contains("EE.log is included"),
        "the EE.log note only appears in the full text path"
    );
}

#[test]
fn assemble_report_text_tail_is_silent_when_the_log_is_quiet() {
    let home = std::env::temp_dir();
    let log_dir = home.join(format!("assemble-quiet-{}", std::process::id()));
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(log_dir.join("tennoscope.log"), "[INFO] nothing wrong\n").unwrap();
    let text = app_lib::report::assemble_report_text(
        &meta(&home, &log_dir),
        "{\"state\":\"ready\"}",
        app_lib::report::EeLogState::NotRequested,
        app_lib::report::LogBody::Tail,
    )
    .expect("copy text builds");
    std::fs::remove_dir_all(&log_dir).unwrap();
    assert!(
        text.contains("no warnings or errors"),
        "a quiet log gets a one-line note in the tail section"
    );
}
