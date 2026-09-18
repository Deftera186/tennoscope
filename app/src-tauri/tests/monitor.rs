use std::{
    sync::{Arc, atomic::AtomicU64, mpsc},
    time::{Duration, Instant},
};

use app_lib::monitor::{
    LogMonitorDiagnostic, LogObservation, MonitorGeneration, MonitorInput, MonitorMachine,
    spawn_generation_worker, spawn_monitor_refresh_task,
};
use warframe_acquisition::AcquisitionError;

#[test]
fn startup_detection_triggers_once_and_absence_or_errors_publish_health() {
    let mut monitor = MonitorMachine::new(15);
    assert!(monitor.tick(MonitorInput::running(0, 7, None)).refresh);
    assert!(!monitor.tick(MonitorInput::running(1, 7, None)).refresh);
    assert_eq!(
        monitor
            .tick(MonitorInput::absent(2, false))
            .acquisition_health,
        Some(AcquisitionError::GameNotRunning)
    );
    assert_eq!(
        monitor
            .tick(MonitorInput::error(
                3,
                AcquisitionError::ProcessDiscoveryFailed
            ))
            .acquisition_health,
        Some(AcquisitionError::ProcessDiscoveryFailed)
    );
}

#[test]
fn an_absent_game_with_the_launcher_visible_is_reported_distinctly() {
    let mut monitor = MonitorMachine::new(15);

    assert_eq!(
        monitor
            .tick(MonitorInput::absent(0, true))
            .acquisition_health,
        Some(AcquisitionError::LauncherRunning)
    );
}

#[test]
fn complete_appended_line_triggers_once_and_cooldown_coalesces() {
    let mut monitor = MonitorMachine::new(15);
    monitor.tick(MonitorInput::running(
        0,
        7,
        Some(LogObservation::new("a", 10, Vec::new())),
    ));
    assert!(
        !monitor
            .tick(MonitorInput::running(
                2,
                7,
                Some(LogObservation::new(
                    "a",
                    35,
                    b"Inventory sync done\n".to_vec()
                ))
            ))
            .refresh
    );
    assert!(
        !monitor
            .tick(MonitorInput::running(
                10,
                7,
                Some(LogObservation::new(
                    "a",
                    60,
                    b"Inventory sync done\n".to_vec()
                ))
            ))
            .refresh
    );
    assert!(monitor.tick(MonitorInput::running(15, 7, None)).refresh);
    assert!(!monitor.tick(MonitorInput::running(16, 7, None)).refresh);
}

#[test]
fn tail_retains_boundaries_requires_newline_and_handles_rotation_and_large_growth() {
    let mut monitor = MonitorMachine::new(0);
    monitor.tick(MonitorInput::running(
        0,
        7,
        Some(LogObservation::new("a", 0, Vec::new())),
    ));
    assert!(
        !monitor
            .tick(MonitorInput::running(
                1,
                7,
                Some(LogObservation::new("a", 18, b"Inventory sync do".to_vec()))
            ))
            .refresh
    );
    assert!(
        monitor
            .tick(MonitorInput::running(
                2,
                7,
                Some(LogObservation::new("a", 21, b"ne\n".to_vec()))
            ))
            .refresh
    );
    assert!(
        !monitor
            .tick(MonitorInput::running(
                3,
                7,
                Some(LogObservation::new("b", 4, b"new\n".to_vec()))
            ))
            .refresh
    );
    let huge = vec![b'x'; 1024 * 1024 + 50];
    let result = monitor.tick(MonitorInput::running(
        4,
        7,
        Some(LogObservation::new("b", huge.len() as u64 + 4, huge)),
    ));
    assert!(!result.refresh);
    assert_eq!(monitor.log_offset(), 1024 * 1024 + 54);
}

#[test]
fn log_read_errors_are_published() {
    let mut monitor = MonitorMachine::new(0);
    monitor.tick(MonitorInput::running(0, 7, None));
    let result = monitor.tick(MonitorInput::running_with_log_error(1, 7));
    assert_eq!(result.acquisition_health, None);
    assert_eq!(result.log_health, Some(LogMonitorDiagnostic::ReadFailed));
}

#[test]
fn startup_refresh_and_log_failure_are_independent_outputs() {
    let mut monitor = MonitorMachine::new(15);
    let result = monitor.tick(MonitorInput::running_with_log_error(0, 7));
    assert!(result.refresh);
    assert_eq!(result.acquisition_health, None);
    assert_eq!(result.log_health, Some(LogMonitorDiagnostic::ReadFailed));
}

#[test]
fn ready_log_monitor_degrades_when_game_disappears_or_discovery_fails() {
    let mut absent = MonitorMachine::new(0);
    assert_eq!(
        absent
            .tick(MonitorInput::running(
                0,
                7,
                Some(LogObservation::new("a", 0, Vec::new()))
            ))
            .log_health,
        Some(LogMonitorDiagnostic::Ready)
    );
    assert_eq!(
        absent.tick(MonitorInput::absent(1, false)).log_health,
        Some(LogMonitorDiagnostic::Unavailable)
    );

    let mut failed = MonitorMachine::new(0);
    failed.tick(MonitorInput::running(
        0,
        7,
        Some(LogObservation::new("a", 0, Vec::new())),
    ));
    assert_eq!(
        failed
            .tick(MonitorInput::error(
                1,
                AcquisitionError::ProcessDiscoveryFailed
            ))
            .log_health,
        Some(LogMonitorDiagnostic::Unavailable)
    );
}

#[test]
fn inventory_refresh_work_never_blocks_the_log_monitor_thread() {
    let (release, wait) = mpsc::channel::<()>();
    let started = Instant::now();

    let worker = spawn_monitor_refresh_task(move || {
        wait.recv().unwrap();
    });

    assert!(started.elapsed() < Duration::from_millis(100));
    release.send(()).unwrap();
    worker.join().unwrap();
}

#[test]
fn monitor_generation_stops_synchronously_and_restarts_once_retired() {
    let current = Arc::new(AtomicU64::new(0));
    let (started_tx, started_rx) = mpsc::channel();
    let (retired_tx, retired_rx) = mpsc::channel();

    let first = MonitorGeneration::new(1, Arc::clone(&current));
    let first_handle =
        spawn_generation_worker(first.clone(), started_tx.clone(), retired_tx.clone());
    assert_eq!(started_rx.recv_timeout(Duration::from_secs(1)).unwrap(), 1);

    first.request_stop();
    first_handle.join().unwrap();
    assert_eq!(retired_rx.recv_timeout(Duration::from_secs(1)).unwrap(), 1);

    let second = MonitorGeneration::new(2, current);
    let second_handle = spawn_generation_worker(second.clone(), started_tx, retired_tx);
    assert_eq!(started_rx.recv_timeout(Duration::from_secs(1)).unwrap(), 2);
    second.request_stop();
    second_handle.join().unwrap();
}

#[test]
fn stale_generation_rejects_late_publication() {
    let current = Arc::new(AtomicU64::new(0));
    let old = MonitorGeneration::new(7, Arc::clone(&current));
    let next = MonitorGeneration::new(8, current);
    let mut publications = Vec::new();

    assert!(!old.publish(|| publications.push("old")));
    assert!(next.publish(|| publications.push("next")));
    assert_eq!(publications, ["next"]);
}

#[test]
fn retired_generation_rejects_post_work_publication() {
    let current = Arc::new(AtomicU64::new(0));
    let generation = MonitorGeneration::new(11, current);
    let (work_started_tx, work_started_rx) = mpsc::channel();
    let (finish_work_tx, finish_work_rx) = mpsc::channel();
    let (publication_tx, publication_rx) = mpsc::channel();
    let worker_generation = generation.clone();

    let worker = spawn_monitor_refresh_task(move || {
        work_started_tx.send(()).unwrap();
        finish_work_rx.recv().unwrap();
        worker_generation.publish(|| publication_tx.send(()).unwrap());
    });
    work_started_rx
        .recv_timeout(Duration::from_secs(1))
        .unwrap();
    generation.request_stop();
    finish_work_tx.send(()).unwrap();
    worker.join().unwrap();

    assert!(publication_rx.try_recv().is_err());
}
