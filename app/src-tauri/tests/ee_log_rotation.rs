use std::fs;

use app_lib::{
    MonitorInput, MonitorMachine, RewardLogEvent, RewardLogMachine, build_monitor_input,
    ee_log_rotation_keep_from, ee_log_session_start_utc, ee_log_stale_prefix_end,
};
use tempfile::TempDir;

/// The moment the monitor attached, in the 2026-08-22 incident's terms: 02:01:50 UTC.
const NOW: u64 = 1_787_364_110;

const NOW_HEADER: &str = "[UTC: Sat Aug 22 02:01:50 2026]";
const STALE_HEADER: &str = "[UTC: Fri Aug 21 15:01:50 2026]";

const STYANAX_RELIC: &str =
    "Resource load completed (/Lotus/Types/Game/Projections/T1VoidProjectionStyanaxPrimeDBronze)";
const GYRE_RELIC: &str =
    "Resource load completed (/Lotus/Types/Game/Projections/T1VoidProjectionGyrePrimeDBronze)";
const INVENTORY_SYNC: &str = "Hub.lua: Inventory sync done";
const REWARD_SCREEN: &str = "VoidProjections: OpenVoidProjectionRewardScreenRMI";

/// An EE.log shaped like the game writes them: one clock line, then uptime-prefixed lines.
fn ee_log(session_utc: &str, lines: &[(f64, &str)]) -> Vec<u8> {
    let mut bytes =
        format!("0.028 Sys [Diag]: Current time: {session_utc} {session_utc}\n").into_bytes();
    for (uptime, line) in lines {
        bytes.extend_from_slice(format!("{uptime:.3} Sys [Info]: {line}\n").as_bytes());
    }
    bytes
}

/// Attach the machine to `first` at `NOW`, then resolve the log path to `next` at `NOW + 20` --
/// the prefix flip of the incident, far enough past the refresh cooldown that a replayed
/// inventory marker alone decides whether a refresh fires.
fn flip_to(
    dir: &TempDir,
    first: Vec<u8>,
    next: Vec<u8>,
) -> (MonitorMachine, Vec<RewardLogEvent>, bool) {
    let first_path = dir.path().join("prefix-a/EE.log");
    let next_path = dir.path().join("prefix-b/EE.log");
    fs::create_dir_all(first_path.parent().unwrap()).unwrap();
    fs::create_dir_all(next_path.parent().unwrap()).unwrap();
    fs::write(&first_path, first).unwrap();
    fs::write(&next_path, next).unwrap();

    let mut machine = MonitorMachine::new(15);
    let mut rewards = RewardLogMachine::default();
    let (input, bytes) = build_monitor_input(&machine, NOW, 3469115, Some(first_path));
    machine.tick(input);
    assert!(rewards.observe_bytes(&bytes).is_empty());

    let (input, bytes) = build_monitor_input(&machine, NOW + 20, 3469115, Some(next_path));
    let result = machine.tick(input);
    (machine, rewards.observe_bytes(&bytes), result.refresh)
}

/// The 2026-08-22 report: a seventeen-second launcher process, an EE.log path that flipped
/// between Wine prefixes, and the morning's fissure replayed as if it were live. The replay armed
/// the poller, ran the reward pipeline against a screen that did not exist, and left health
/// degraded for a game that was never running.
#[test]
fn a_prefix_flip_to_a_stale_ee_log_replays_nothing() {
    let dir = TempDir::new().unwrap();
    let (_, events, refresh) = flip_to(
        &dir,
        ee_log(STALE_HEADER, &[(1.0, "Cache warmed")]),
        ee_log(
            STALE_HEADER,
            &[
                (10.0, STYANAX_RELIC),
                (11.0, GYRE_RELIC),
                (20.6, INVENTORY_SYNC),
                (182.1, REWARD_SCREEN),
            ],
        ),
    );

    assert!(
        events.is_empty(),
        "a log from a session that ended hours before the process was attached must not emit \
         reward events, got {events:?}"
    );
    assert!(
        !refresh,
        "an inventory marker replayed from a stale log must not request a refresh"
    );
}

/// A replacement log that belongs to the tracked process -- same shape as the stale one, but its
/// session is this one. Rotation handling exists for exactly this file, and the freshness gate
/// must not swallow it.
#[test]
fn a_replacement_ee_log_from_the_live_session_still_flows() {
    let dir = TempDir::new().unwrap();
    let (_, events, refresh) = flip_to(
        &dir,
        ee_log(NOW_HEADER, &[(1.0, "Cache warmed")]),
        ee_log(
            NOW_HEADER,
            &[
                (10.0, STYANAX_RELIC),
                (11.0, GYRE_RELIC),
                (20.6, INVENTORY_SYNC),
                (182.1, REWARD_SCREEN),
            ],
        ),
    );

    assert!(matches!(
        events.first(),
        Some(RewardLogEvent::BaselineRequested { .. })
    ));
    assert!(
        refresh,
        "a live inventory marker must still request a refresh"
    );
}

/// The long-session case: the log's header is eleven hours old because the session is, and the
/// interesting lines are at the far end of it. The gate drops the stale prefix and keeps the tail
/// rather than judging the whole file by its header.
#[test]
fn a_long_session_log_keeps_only_its_recent_lines() {
    let dir = TempDir::new().unwrap();
    let eleven_hours = 39_600.0;
    let long_session = ee_log(
        STALE_HEADER,
        &[
            (100.0, STYANAX_RELIC),
            (eleven_hours, INVENTORY_SYNC),
            (eleven_hours + 0.5, GYRE_RELIC),
        ],
    );
    let len = long_session.len() as u64;
    let (machine, events, refresh) = flip_to(
        &dir,
        ee_log(STALE_HEADER, &[(1.0, "Cache warmed")]),
        long_session,
    );

    // The first relic load is eleven hours stale and must not reach the reward machine; the one
    // survivor leaves loaded_relics below the baseline threshold of two.
    assert!(
        events.is_empty(),
        "only the fresh tail of a long session may emit events, got {events:?}"
    );
    assert!(refresh, "the kept tail carries a live inventory marker");
    assert_eq!(machine.log_offset(), len);
}

/// Line-level freshness: walls are session start plus uptime, bare lines inherit the uptime above
/// them, and an unterminated tail counts as a line.
#[test]
fn stale_prefix_ends_at_the_first_line_reaching_the_floor() {
    let session = 1_000_000_u64;
    let bytes = b"10.000 Sys [Info]: first\n\
                  20.000 Sys [Info]: second\n\
                  bare tail without an uptime\n\
                  40.000 Sys [Info]: fourth\n\
                  50.000 Sys [Info]: unterminated"[..]
        .to_vec();
    let offset_of = |needle: &[u8]| {
        bytes
            .windows(needle.len())
            .position(|window| window == needle)
            .unwrap()
    };

    assert_eq!(ee_log_stale_prefix_end(&bytes, session, session), Some(0));
    assert_eq!(
        ee_log_stale_prefix_end(&bytes, session, session + 25),
        Some(offset_of(b"40.000"))
    );
    assert_eq!(
        ee_log_stale_prefix_end(&bytes, session, session + 45),
        Some(offset_of(b"50.000"))
    );
    assert_eq!(
        ee_log_stale_prefix_end(&bytes, session, session + 100),
        None
    );
}

/// The clock line: both real formats parse, the bracketed UTC time is the one taken, and a log
/// without a readable clock says nothing about when it happened.
#[test]
fn session_start_reads_the_utc_clock_line() {
    let header = b"0.028 Sys [Diag]: Current time: Sat Aug 22 05:01:50 2026 \
                   [UTC: Sat Aug 22 02:01:50 2026]\n"[..]
        .to_vec();
    assert_eq!(ee_log_session_start_utc(&header), Some(NOW));

    let january = b"0.031 Sys [Diag]: Current time: Mon Jan 02 10:04:05 2023 \
                    [UTC: Mon Jan 02 08:04:05 2023]"[..]
        .to_vec();
    assert_eq!(ee_log_session_start_utc(&january), Some(1_672_646_645));

    assert_eq!(ee_log_session_start_utc(b"no clock line at all"), None);
    assert_eq!(
        ee_log_session_start_utc(b"[UTC: Sat Xyz 22 02:01:50 2026]"),
        None
    );
}

/// The fallbacks: without a clock line the file's own creation time decides, and a file that
/// cannot be placed in time at all is treated as stale rather than replayed.
#[test]
fn rotation_keep_from_falls_back_to_the_file_creation_time() {
    let bytes = b"20.000 Sys [Info]: no clock line\n".to_vec();
    assert_eq!(
        ee_log_rotation_keep_from(&bytes, Some(970), Some(1_000)),
        Some(0)
    );
    assert_eq!(
        ee_log_rotation_keep_from(&bytes, Some(500), Some(1_000)),
        None
    );
    assert_eq!(ee_log_rotation_keep_from(&bytes, None, Some(1_000)), None);
    assert_eq!(ee_log_rotation_keep_from(&bytes, None, None), Some(0));
}

/// Attachment time is the freshness floor's anchor: it appears when the pid is adopted and
/// disappears with the process, so the next session starts a new clock.
#[test]
fn attached_since_tracks_the_current_process_only() {
    let mut monitor = MonitorMachine::new(15);
    assert_eq!(monitor.attached_since(), None);
    monitor.tick(MonitorInput::running(100, 7, None));
    assert_eq!(monitor.attached_since(), Some(100));
    monitor.tick(MonitorInput::running(150, 7, None));
    assert_eq!(monitor.attached_since(), Some(100));
    monitor.tick(MonitorInput::absent(200, false));
    assert_eq!(monitor.attached_since(), None);
    monitor.tick(MonitorInput::running(300, 9, None));
    assert_eq!(monitor.attached_since(), Some(300));
}
