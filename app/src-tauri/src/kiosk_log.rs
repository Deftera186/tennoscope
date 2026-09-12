//! Detect the Ducat Kiosk's lifecycle in EE.log -- both edges of it.
//!
//! The game narrates this screen completely, which makes the log the fastest and steadiest
//! detector available: it costs no capture, no OCR, and it names the transition ~50ms after it
//! happens instead of a second or two later.
//!
//! Measured over all 57 kiosk sessions in one day's EE.log (2026-08-23):
//!
//! ```text
//! open   InventoryTest.lua: InventoryTest - CurrMode: Selling Prime Parts
//!        InventoryTest.lua: DBG: HudVis 1
//!        Created /Lotus/Interface/InventoryTest.swf
//!        Subscribing for /Lotus/Interface/InventoryTest.swf      <- the screen owns input
//!        InventoryTest.lua: PopulateGrid()
//! close  InventoryTest.lua: DBG: HudVis 0                        <- exactly once per session
//!        Subscribing for /Lotus/Interface/ThemedButtonBar.swf    <- input went elsewhere
//! ```
//!
//! Two facts from that census carry this module. First, `PopulateGrid()` runs **once per
//! session**, at the open -- sessions of 100s, 233s and 290s spent scrolling logged exactly one
//! -- so there is no such thing as a mid-session repopulate, and `HudVis 0` is unambiguous. An
//! earlier reading of this log took `HudVis 0` for the first beat of a repopulate cycle and gave
//! up on log-driven closes entirely; what it was actually looking at was a close immediately
//! followed by a fresh open, which is what rapid open/close testing looks like from here.
//!
//! Second, only two screens ever take input from the kiosk: `ThemedButtonBar` (the ordinary
//! exit, 52 times) and `ItemInfoPopup` (4). The popup case tears the kiosk down for real -- it
//! is followed by `HudVis 1` and a fresh `Created`/`Subscribing` pair when the player returns --
//! so treating a foreign subscription as a close is right in both cases, and the return re-opens
//! on its own. That line is a second, independent witness to the same transition: whichever
//! arrives first closes the session, and the other is ignored.
//!
//! What the log deliberately does *not* decide is whether the grid is readable. Presence and
//! readability were one question here for a while, keyed off the OCR miss streak, and every
//! hiccup in the reader -- a scroll the tracker could not measure, a frame that located no
//! labels -- read as "the kiosk closed" and tore the overlay down mid-session.

/// A kiosk event worth acting on: open the overlay, re-anchor it because the grid was
/// (re)populated, or take it down.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum KioskLogEvent {
    KioskOpened,
    GridPopulated,
    KioskClosed,
}

const MODE_MARKER: &str = "InventoryTest - CurrMode: Selling Prime Parts";
const SWF_MARKER: &str = "/Lotus/Interface/InventoryTest.swf";
const POPULATE_MARKER: &str = "PopulateGrid()";
/// The kiosk's own teardown line, written once per session as the screen goes away.
const CLOSE_MARKER: &str = "InventoryTest.lua: DBG: HudVis 0";
/// Input subscriptions name the screen that owns the keyboard. One for a screen that is not
/// the kiosk means the kiosk is no longer the screen in front of the player.
const SUBSCRIBE_MARKER: &str = "Subscribing for /Lotus/Interface/";

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

    /// The state a stretch of log ends in, folded without reporting the transitions. Used once
    /// at attach: the app can start with the kiosk already on screen, and the open marker for
    /// that session scrolled past before anything was watching.
    pub fn state_after(bytes: &[u8]) -> bool {
        let mut machine = Self::default();
        machine.observe_bytes(bytes);
        machine.open
    }

    /// Adopt a session discovered some other way than its own open marker, so the exit line
    /// that ends it is recognised when it arrives.
    pub fn adopt_open(&mut self) {
        self.open = true;
    }

    pub fn observe_line(&mut self, line: &str) -> Vec<KioskLogEvent> {
        if self.open {
            // Close before anything else: the exit line and the open lines both name the
            // screen, and a session that is ending has nothing else worth reporting.
            if line.contains(CLOSE_MARKER) || input_left_the_kiosk(line) {
                self.open = false;
                return vec![KioskLogEvent::KioskClosed];
            }
            if line.contains(POPULATE_MARKER) {
                return vec![KioskLogEvent::GridPopulated];
            }
            return Vec::new();
        }
        if line.contains(MODE_MARKER) || line.contains(SWF_MARKER) {
            self.open = true;
            return vec![KioskLogEvent::KioskOpened];
        }
        Vec::new()
    }
}

/// Did a screen that is not the kiosk just take the keyboard? The kiosk's own subscription line
/// carries the same prefix, so the SWF name is what separates "the kiosk is up" from "something
/// else is".
fn input_left_the_kiosk(line: &str) -> bool {
    line.contains(SUBSCRIBE_MARKER) && !line.contains(SWF_MARKER)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MODE_LINE: &str =
        "2026/08/23_12.00 InventoryTest.lua: InventoryTest - CurrMode: Selling Prime Parts";
    const SWF_LINE: &str = "Created /Lotus/Interface/InventoryTest.swf";
    const POPULATE_LINE: &str = "InventoryTest.lua: PopulateGrid()";
    const HUD_VIS_ZERO_LINE: &str = "75584.766 Script [Info]: InventoryTest.lua: DBG: HudVis 0";
    const SUBSCRIBE_LINE: &str = "75583.344 Input [Info]: Subscribing for /Lotus/Interface/InventoryTest.swf with input filter /EE/Types/Input/MenuInputFilter";

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

    /// The exit line, and the census behind trusting it: 57 of 57 sessions ended with exactly
    /// one of these, and no session contained a second one.
    #[test]
    fn the_hud_vis_zero_line_closes_the_session() {
        let mut m = KioskLogMachine::default();
        let _ = m.observe_line(MODE_LINE);
        assert_eq!(
            m.observe_line(HUD_VIS_ZERO_LINE),
            vec![KioskLogEvent::KioskClosed]
        );
    }

    /// The second witness: input moving to any other screen. It lands ~50ms after the exit
    /// line, so in practice it is the redundant one -- but a dropped or reordered line must not
    /// leave the overlay stranded over a screen the player has left.
    #[test]
    fn input_moving_to_another_screen_closes_the_session() {
        for taker in [
            "75584.812 Input [Info]: Subscribing for /Lotus/Interface/ThemedButtonBar.swf with input filter /EE/Types/Input/MenuInputFilter",
            "43625.777 Input [Info]: Subscribing for /Lotus/Interface/ItemInfoPopup.swf with input filter /Lotus/Types/Input/ItemInfoPopupInputFilter",
        ] {
            let mut m = KioskLogMachine::default();
            let _ = m.observe_line(MODE_LINE);
            assert_eq!(
                m.observe_line(taker),
                vec![KioskLogEvent::KioskClosed],
                "{taker}"
            );
        }
    }

    /// The kiosk's own subscription is not a foreign one. It arrives inside every open sequence
    /// (`Created` then `Subscribing`), and reading it as a close would end each session
    /// milliseconds after it began.
    #[test]
    fn the_kiosks_own_subscription_does_not_close_it() {
        let mut m = KioskLogMachine::default();
        let _ = m.observe_line(MODE_LINE);
        assert!(m.observe_line(SUBSCRIBE_LINE).is_empty());
        assert_eq!(
            m.observe_line(POPULATE_LINE),
            vec![KioskLogEvent::GridPopulated],
            "the session is still open and still re-anchoring"
        );
    }

    /// A close re-arms the machine, and the popup round trip is the case that needs it: the
    /// game tears the kiosk down for the popup and builds a fresh one when the player comes
    /// back, with no mode line in between -- only `Created`/`Subscribing`.
    #[test]
    fn the_screen_reopens_after_a_close() {
        let mut m = KioskLogMachine::default();
        let _ = m.observe_line(MODE_LINE);
        assert_eq!(
            m.observe_line(HUD_VIS_ZERO_LINE),
            vec![KioskLogEvent::KioskClosed]
        );
        assert_eq!(
            m.observe_line(SWF_LINE),
            vec![KioskLogEvent::KioskOpened],
            "the popup's return builds the screen again"
        );
        assert_eq!(
            m.observe_line(POPULATE_LINE),
            vec![KioskLogEvent::GridPopulated]
        );
    }

    /// Nothing about a closed kiosk is worth reporting, and the exit line repeats in the log
    /// for screens we never opened.
    #[test]
    fn close_markers_before_any_open_are_ignored() {
        let mut m = KioskLogMachine::default();
        assert!(m.observe_line(HUD_VIS_ZERO_LINE).is_empty());
        assert!(
            m.observe_line(
                "75585.0 Input [Info]: Subscribing for /Lotus/Interface/TopMenu.swf with input filter x"
            )
            .is_empty()
        );
    }

    /// The whole transcript of one real session, replayed line for line, in the order the game
    /// wrote it (EE.log 2026-08-23, t=75583.320 to t=75584.812).
    #[test]
    fn a_real_session_transcript_opens_populates_and_closes() {
        let mut m = KioskLogMachine::default();
        let transcript = [
            "75583.320 Script [Info]: InventoryTest.lua: USE SLOW UPDATE TOUCH BUTTONS\tfalse",
            "75583.320 Script [Info]: InventoryTest.lua: InventoryTest - CurrMode: Selling Prime Parts",
            "75583.320 Script [Info]: InventoryTest.lua: DBG: HudVis 1",
            "75583.320 Sys [Info]: Created /Lotus/Interface/Notifications.swf",
            "75583.328 Sys [Info]: Created /Lotus/Interface/InventoryTest.swf",
            "75583.344 Input [Info]: Subscribing for /Lotus/Interface/InventoryTest.swf with input filter /EE/Types/Input/MenuInputFilter",
            "75583.344 Script [Info]: InventoryTest.lua: PopulateGrid()",
            "75583.570 Script [Info]: InventoryTest.lua: PopulateGrid complete",
            "75584.766 Game [Info]: Saving profile took 2.86ms",
            "75584.766 Script [Info]: InventoryTest.lua: DBG: HudVis 0",
            "75584.812 Input [Info]: Subscribing for /Lotus/Interface/ThemedButtonBar.swf with input filter /EE/Types/Input/MenuInputFilter",
        ];
        let events: Vec<KioskLogEvent> = transcript
            .iter()
            .flat_map(|line| m.observe_line(line))
            .collect();
        assert_eq!(
            events,
            vec![
                KioskLogEvent::KioskOpened,
                KioskLogEvent::GridPopulated,
                KioskLogEvent::KioskClosed,
            ],
            "one open, one populate, one close -- the shape of every session in the census"
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
