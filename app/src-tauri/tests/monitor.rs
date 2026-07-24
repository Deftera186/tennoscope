use app_lib::{LogMonitorDiagnostic, LogObservation, MonitorInput, MonitorMachine};
use warframe_acquisition::AcquisitionError;

#[test]
fn startup_detection_triggers_once_and_absence_or_errors_publish_health() {
    let mut monitor = MonitorMachine::new(15);
    assert!(monitor.tick(MonitorInput::running(0, 7, None)).refresh);
    assert!(!monitor.tick(MonitorInput::running(1, 7, None)).refresh);
    assert_eq!(
        monitor.tick(MonitorInput::absent(2)).acquisition_health,
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
