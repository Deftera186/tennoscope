#![cfg(windows)]

use warframe_acquisition::{GameProcess, MemoryReader, ProcessDiscovery, WindowsProc};

/// CI has no Warframe, which makes "nothing found" the assertion worth having: discovery must
/// report an empty result rather than an error, because the app treats an error as a health
/// problem to surface and absence as the ordinary "the game is not running yet" state.
#[test]
fn discovery_reports_absence_rather_than_failure_when_the_game_is_not_running() {
    assert_eq!(WindowsProc::new().discover(), Ok(None));
}

/// PID 0 is the System Idle Process: it always exists and `OpenProcess` never opens it. Which
/// refusal Windows picks is not worth pinning here -- the unit tests cover the mapping from each
/// error code -- but a refusal must never surface as an empty-but-successful region list, because
/// the scanner would read that as "the game has no memory worth scanning" and give up quietly.
#[test]
fn a_process_that_cannot_be_opened_fails_rather_than_reporting_no_regions() {
    let adapter = WindowsProc::new();
    assert!(adapter.readable_regions(&GameProcess::new(0)).is_err());
}

/// An empty read must not open a handle or touch the process -- the scanner issues them at region
/// boundaries and would otherwise turn every boundary into an `OpenProcess` call.
#[test]
fn an_empty_read_succeeds_without_touching_the_process() {
    let adapter = WindowsProc::new();
    assert_eq!(
        adapter.read_at(&GameProcess::new(0), 0x1000, &mut []),
        Ok(0)
    );
}
