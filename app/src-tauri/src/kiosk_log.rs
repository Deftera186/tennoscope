//! Detect the Ducat Kiosk's lifecycle in EE.log.
//!
//! The reward screen announces itself with a marker and dismisses with another, so
//! `RewardLogMachine` can key its whole session off log lines alone. The kiosk has open markers
//! -- the mode line, the SWF creation line, and one `PopulateGrid()` per repopulation -- and,
//! contrary to what this machine believed for its first day, a close marker too:
//! `InventoryTest.lua: DBG: HudVis 0` is the kiosk hiding itself, and it precedes every reopen
//! in a live 2026-08-23 log. Sessions therefore open, re-anchor, *and* close off the log; the
//! poller's miss streak is only the backstop for a log that stops cooperating.
//!
//! The markers were read off a live 2026-08-23 log:
//! - `InventoryTest.lua: InventoryTest - CurrMode: Selling Prime Parts`
//! - `Created /Lotus/Interface/InventoryTest.swf`
//! - `InventoryTest.lua: PopulateGrid()`
//! - `InventoryTest.lua: DBG: HudVis 0`

/// A kiosk event worth acting on: open the overlay, re-anchor it because the grid was
/// repopulated (open, filter change, basket edit), or close it because the game said so.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KioskLogEvent {
    KioskOpened,
    GridPopulated,
    KioskClosed,
}

const MODE_MARKER: &str = "InventoryTest - CurrMode: Selling Prime Parts";
const SWF_MARKER: &str = "/Lotus/Interface/InventoryTest.swf";
const POPULATE_MARKER: &str = "PopulateGrid()";
const CLOSE_MARKER: &str = "InventoryTest.lua: DBG: HudVis 0";

#[derive(Default)]
pub struct KioskLogMachine {
    open: bool,
    carry: Vec<u8>,
}

impl KioskLogMachine {
    /// Feed raw log bytes; complete lines only are processed, partial tails are carried over --
    /// same contract as `RewardLogMachine::observe_bytes`, because both machines are fed from the
    /// same byte stream.
    pub fn observe_bytes(&mut self, bytes: &[u8]) -> Vec<KioskLogEvent> {
        self.carry.extend_from_slice(bytes);
        let complete = self
            .carry
            .iter()
            .rposition(|byte| *byte == b'\n')
            .map_or(0, |index| index + 1);
        if complete == 0 {
            // A marker longer than this cannot be coming; drop the tail rather than grow forever
            // if a log stops ending in newlines.
            if self.carry.len() > 64 * 1024 {
                self.carry.clear();
            }
            return Vec::new();
        }
        let lines = self.carry.drain(..complete).collect::<Vec<_>>();
        String::from_utf8_lossy(&lines)
            .lines()
            .flat_map(|line| self.observe_line(line))
            .collect()
    }

    pub fn observe_line(&mut self, line: &str) -> Vec<KioskLogEvent> {
        let mut events = Vec::new();
        if !self.open && (line.contains(MODE_MARKER) || line.contains(SWF_MARKER)) {
            self.open = true;
            events.push(KioskLogEvent::KioskOpened);
            return events;
        }
        if self.open && line.contains(CLOSE_MARKER) {
            self.open = false;
            events.push(KioskLogEvent::KioskClosed);
            return events;
        }
        if self.open && line.contains(POPULATE_MARKER) && events.is_empty() {
            events.push(KioskLogEvent::GridPopulated);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODE_LINE: &str =
        "2026/08/23_12.00 InventoryTest.lua: InventoryTest - CurrMode: Selling Prime Parts";
    const SWF_LINE: &str = "Created /Lotus/Interface/InventoryTest.swf";
    const POPULATE_LINE: &str = "InventoryTest.lua: PopulateGrid()";

    #[test]
    fn opening_emits_opened_then_populated() {
        let mut m = KioskLogMachine::default();
        assert!(
            m.observe_line(MODE_LINE)
                .contains(&KioskLogEvent::KioskOpened)
        );
        assert_eq!(
            m.observe_line(POPULATE_LINE),
            vec![KioskLogEvent::GridPopulated]
        );
    }

    #[test]
    fn swf_marker_also_opens() {
        let mut m = KioskLogMachine::default();
        assert!(
            m.observe_line(SWF_LINE)
                .contains(&KioskLogEvent::KioskOpened)
        );
    }

    #[test]
    fn populate_before_open_is_ignored() {
        let mut m = KioskLogMachine::default();
        assert!(m.observe_line(POPULATE_LINE).is_empty());
    }

    #[test]
    fn repeated_open_markers_do_not_reopen() {
        let mut m = KioskLogMachine::default();
        let _ = m.observe_line(MODE_LINE);
        assert!(m.observe_line(MODE_LINE).is_empty());
    }

    /// `InventoryTest.lua: DBG: HudVis 0` is the kiosk hiding itself -- read off a live
    /// 2026-08-23 log, where it precedes every reopen by a beat. While a session is open that
    /// line is the close verdict, and the session that follows is a fresh open.
    #[test]
    fn hud_vis_zero_closes_an_open_session() {
        let mut m = KioskLogMachine::default();
        let _ = m.observe_line(MODE_LINE);
        assert_eq!(
            m.observe_line("InventoryTest.lua: DBG: HudVis 0"),
            vec![KioskLogEvent::KioskClosed]
        );
        assert!(
            m.observe_line(MODE_LINE)
                .contains(&KioskLogEvent::KioskOpened)
        );
    }

    #[test]
    fn hud_vis_zero_before_any_open_is_ignored() {
        let mut m = KioskLogMachine::default();
        assert!(
            m.observe_line("InventoryTest.lua: DBG: HudVis 0")
                .is_empty()
        );
    }

    #[test]
    fn hud_vis_one_is_not_a_close() {
        let mut m = KioskLogMachine::default();
        let _ = m.observe_line(MODE_LINE);
        assert!(
            m.observe_line("InventoryTest.lua: DBG: HudVis 1")
                .is_empty()
        );
    }

    #[test]
    fn observe_bytes_splits_on_newlines_across_chunks() {
        let mut m = KioskLogMachine::default();
        let line = MODE_LINE.as_bytes().to_vec();
        let split = line.len() - 10;
        assert!(
            m.observe_bytes(&line[..split]).is_empty(),
            "partial line is carried"
        );
        assert!(
            m.observe_bytes(&[&line[split..], b"\n"].concat())
                .contains(&KioskLogEvent::KioskOpened)
        );
        assert!(
            m.observe_bytes(&[POPULATE_LINE.as_bytes(), b"\n"].concat())
                .contains(&KioskLogEvent::GridPopulated),
            "a complete newline-terminated populate line emits"
        );
    }

    #[test]
    fn unrelated_lines_produce_nothing() {
        let mut m = KioskLogMachine::default();
        assert!(m.observe_line("Hello squad sync stuff").is_empty());
    }
}
