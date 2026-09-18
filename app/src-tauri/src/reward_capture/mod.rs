//! Where the game's rectangle and the game's pixels come from.

pub mod availability;
#[cfg(target_os = "linux")]
pub mod direct;
#[cfg(windows)]
pub mod dxgi;
pub mod geometry;
#[cfg(target_os = "linux")]
pub mod kwin;
#[cfg(target_os = "linux")]
pub mod portal;
pub mod x11;

use std::ffi::OsStr;
use std::sync::Mutex;

use crate::overlay_window::WindowRect;

/// Which display server this session is, which decides where frames can come from.
///
/// Not "which display server the game is on": a Wayland session runs XWayland too, so the game
/// may be an X11 client on a Wayland desktop. That is a separate question, answered by whether
/// X11 enumeration finds a window -- see `game_rect`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionKind {
    X11,
    Wayland,
}

impl SessionKind {
    /// A stable short name for the report header and the logs.
    pub const fn label(self) -> &'static str {
        match self {
            Self::X11 => "x11",
            Self::Wayland => "wayland",
        }
    }
}

/// Decide the session from the two variables that describe it.
///
/// `WAYLAND_DISPLAY` first: the compositor sets it for every client it accepts, so its presence
/// is positive evidence. `XDG_SESSION_TYPE` is the fallback rather than the primary because a
/// compositor started outside a session manager may not set it at all.
///
/// Split from `session_kind` so the decision is testable without touching the environment.
pub fn session_kind_from(
    wayland_display: Option<&OsStr>,
    session_type: Option<&OsStr>,
) -> SessionKind {
    if wayland_display.is_some_and(|display| !display.is_empty()) {
        return SessionKind::Wayland;
    }
    if session_type.is_some_and(|kind| kind.eq_ignore_ascii_case("wayland")) {
        return SessionKind::Wayland;
    }
    SessionKind::X11
}

/// The session this process is actually running in.
pub fn session_kind() -> SessionKind {
    session_kind_from(
        std::env::var_os("WAYLAND_DISPLAY").as_deref(),
        std::env::var_os("XDG_SESSION_TYPE").as_deref(),
    )
}

/// A whole-monitor capture, together with the geometry of the monitor it came from.
///
/// The origin travels with the pixels because the crop is computed against it: a game on a
/// monitor at a negative origin captures from a different place than its window rect alone
/// suggests, which is the bug the multi-monitor tests in `reward_ocr` pin down.
pub struct MonitorFrame {
    pub image: image::RgbaImage,
    pub origin_x: i32,
    pub origin_y: i32,
    pub width: u32,
    pub height: u32,
}

/// Where the game's window is, in the desktop's own coordinates.
///
/// Separate from the frame source because the two fail independently: on a Wayland session an
/// XWayland game still yields a real rect from X11 and must capture that drawable directly.
pub trait GameRectSource {
    fn game_rect(&mut self) -> Result<WindowRect, &'static str>;
}

/// Where the game's pixels come from.
pub trait GameFrameSource {
    /// Capture the monitor that contains `rect`.
    fn capture_monitor(&mut self, rect: WindowRect) -> Result<MonitorFrame, &'static str>;
}

/// Which source supplied the game's rectangle.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RectOrigin {
    X11,
    Wayland,
    Portal,
}

impl RectOrigin {
    pub const fn label(self) -> &'static str {
        match self {
            Self::X11 => "x11",
            Self::Wayland => "wayland",
            Self::Portal => "portal",
        }
    }
}

/// Which API supplied the captured pixels.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FrameBackend {
    X11,
    #[cfg(windows)]
    Dxgi,
    Wayland,
    Kwin,
    Portal,
}

impl FrameBackend {
    #[cfg(windows)]
    const NATIVE_WINDOW: Self = Self::Dxgi;
    #[cfg(not(windows))]
    const NATIVE_WINDOW: Self = Self::X11;

    pub const fn label(self) -> &'static str {
        match self {
            Self::X11 => "x11",
            #[cfg(windows)]
            Self::Dxgi => "dxgi",
            Self::Wayland => "wayland",
            Self::Kwin => "kwin",
            Self::Portal => "portal",
        }
    }
}

#[cfg(target_os = "linux")]
/// Which X11 surface supplies pixels once X11 has found the game.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum X11FrameTarget {
    Monitor,
    Window,
}

#[cfg(target_os = "linux")]
/// xcap's monitor path opens the desktop portal on Wayland, whose frame can use a different
/// coordinate space from the discovered XWayland window. Capture that window's drawable instead.
const fn x11_frame_target(session: SessionKind) -> X11FrameTarget {
    match session {
        SessionKind::X11 => X11FrameTarget::Monitor,
        SessionKind::Wayland => X11FrameTarget::Window,
    }
}

/// One native-Wayland pixel source, and the rectangle source that comes with it.
///
/// A rung is inert data so that [`WAYLAND_LADDER`] can be the single statement of precedence:
/// both the probing in `GameCapture::backend_availability` and the selection in
/// [`capture_sources`] walk it in order. Before, each expressed the order in its own way -- a
/// short-circuit chain and an `if` ladder -- and a backend added to one but not the other would
/// be probed and never chosen, or chosen and never probed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct WaylandRung {
    rect_origin: RectOrigin,
    frame_backend: FrameBackend,
}

/// The whole-monitor backends, ordered by what they cost the player.
///
/// A native Wayland client's window geometry no client may ask for, so every rung captures a
/// whole monitor. wlroots and KWin capture silently; the portal costs a permission grant and
/// lights a screen-sharing indicator for as long as it lives, so it is last. Gameplay capture
/// must never open a chooser mid-mission, which is why the portal rung is offered only when a
/// session the player already authorized is still held.
const WAYLAND_LADDER: [WaylandRung; 3] = [
    WaylandRung {
        rect_origin: RectOrigin::Wayland,
        frame_backend: FrameBackend::Wayland,
    },
    WaylandRung {
        // KWin reports logical output geometry, the same coordinate space wlroots reports.
        rect_origin: RectOrigin::Wayland,
        frame_backend: FrameBackend::Kwin,
    },
    WaylandRung {
        rect_origin: RectOrigin::Portal,
        frame_backend: FrameBackend::Portal,
    },
];

/// What this desktop can actually offer, decided before any capture is attempted.
///
/// Passed as data rather than probed inside the decision so precedence is testable without a
/// compositor, and so each backend is asked about exactly once per poll.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BackendAvailability {
    /// wlroots `zwlr_screencopy_manager_v1`.
    pub direct_wayland: bool,
    /// KWin's ScreenShot2 service, at a version this program can use.
    pub kwin: bool,
    /// A portal session the player already authorized, still held by the worker.
    pub portal_session: bool,
}

impl BackendAvailability {
    /// Nothing beyond X11 is on offer, and nothing will be asked.
    pub const NONE: Self = Self {
        direct_wayland: false,
        kwin: false,
        portal_session: false,
    };

    /// Whether one whole-monitor backend is on offer.
    ///
    /// Native-window capture is decided by window discovery before this struct is consulted.
    const fn offers(self, backend: FrameBackend) -> bool {
        match backend {
            FrameBackend::X11 => false,
            #[cfg(windows)]
            FrameBackend::Dxgi => false,
            FrameBackend::Wayland => self.direct_wayland,
            FrameBackend::Kwin => self.kwin,
            FrameBackend::Portal => self.portal_session,
        }
    }

    /// Record that one whole-monitor backend answered a probe.
    #[cfg(any(target_os = "linux", test))]
    const fn offer(&mut self, backend: FrameBackend) {
        match backend {
            FrameBackend::X11 => {}
            #[cfg(windows)]
            FrameBackend::Dxgi => {}
            FrameBackend::Wayland => self.direct_wayland = true,
            FrameBackend::Kwin => self.kwin = true,
            FrameBackend::Portal => self.portal_session = true,
        }
    }
}

/// Decide where native-Wayland geometry and pixels come from.
///
/// X11 wins whenever it finds the game: it provides the actual window rectangle, so an XWayland
/// Warframe never needs a Wayland backend at all. Otherwise the game is a native Wayland client
/// and the choice is the first rung of [`WAYLAND_LADDER`] this desktop offers.
pub fn capture_sources(
    session: SessionKind,
    x11_found: bool,
    available: BackendAvailability,
) -> Option<(RectOrigin, FrameBackend)> {
    if x11_found {
        return Some((RectOrigin::X11, FrameBackend::NATIVE_WINDOW));
    }
    if session == SessionKind::X11 {
        return None;
    }
    WAYLAND_LADDER
        .into_iter()
        .find(|rung| available.offers(rung.frame_backend))
        .map(|rung| (rung.rect_origin, rung.frame_backend))
}

#[derive(Debug, Eq, PartialEq)]
struct CaptureChoice {
    rect_origin: RectOrigin,
    frame_backend: FrameBackend,
    x11_rect: Option<WindowRect>,
}

fn capture_choice(
    session: SessionKind,
    x11_rect: Result<WindowRect, &'static str>,
    available: BackendAvailability,
) -> Result<CaptureChoice, &'static str> {
    match x11_rect {
        Ok(rect) => Ok(CaptureChoice {
            rect_origin: RectOrigin::X11,
            frame_backend: FrameBackend::NATIVE_WINDOW,
            x11_rect: Some(rect),
        }),
        Err(reason) => {
            let Some((rect_origin, frame_backend)) = capture_sources(session, false, available)
            else {
                // On a Wayland session the X11 lookup failing is expected and uninformative;
                // what the player needs to know is that no pixel source is usable at all.
                return Err(if session == SessionKind::Wayland {
                    NO_WAYLAND_BACKEND
                } else {
                    reason
                });
            };
            Ok(CaptureChoice {
                rect_origin,
                frame_backend,
                x11_rect: None,
            })
        }
    }
}

/// What a native-Wayland player sees when nothing can capture the screen.
///
/// Names the one action that fixes it, because every silent backend is either absent or
/// unauthorized at this point and the portal has no session to fall back on.
pub const NO_WAYLAND_BACKEND: &str =
    "Desktop capture permission is required for native Wayland Warframe";

/// Everything about a capture that decides which pixels get read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CaptureShape {
    pub origin: RectOrigin,
    pub frame_backend: FrameBackend,
    pub rect: WindowRect,
    pub monitor_x: i32,
    pub monitor_y: i32,
    pub monitor_width: u32,
    pub monitor_height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CaptureLogLevel {
    Info,
    Debug,
}

fn capture_log_level(changed: bool) -> CaptureLogLevel {
    if changed {
        CaptureLogLevel::Info
    } else {
        CaptureLogLevel::Debug
    }
}

fn capture_shapes_changed(previous: &[CaptureShape], current: &[CaptureShape]) -> bool {
    previous != current
}

/// Update one capture instance's Info-suppression state and report whether its shape list changed.
fn update_capture_shapes(previous: &mut Vec<CaptureShape>, current: Vec<CaptureShape>) -> bool {
    let changed = capture_shapes_changed(previous, &current);
    *previous = current;
    changed
}

/// The latest successful capture geometry for Task 11's process-wide report header.
///
/// This snapshot never decides log level: each `GameCapture` owns its own suppression state.
static LATEST_CAPTURE_SHAPE_FOR_REPORT: Mutex<Option<CaptureShape>> = Mutex::new(None);

fn capture_sources_from(
    snapshot: &Mutex<Option<CaptureShape>>,
) -> Option<(&'static str, &'static str)> {
    let latest = {
        *snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    };
    latest.map(|shape| (shape.origin.label(), shape.frame_backend.label()))
}

/// Which rectangle source and pixel backend last produced a frame, for the report header.
pub fn last_capture_sources() -> (Option<&'static str>, Option<&'static str>) {
    capture_sources_from(&LATEST_CAPTURE_SHAPE_FOR_REPORT)
        .map_or((None, None), |(rect, frame)| (Some(rect), Some(frame)))
}

/// Record the capture geometry.
///
/// At Info when the shape changes, so a stable-build report carries the one line that
/// distinguishes "no window" from "wrong monitor" from "captured a helper window" -- the
/// distinction the 2026-08-22 report could not make. At Debug otherwise, because the poller
/// re-captures the same monitor every 400ms and a line per poll would evict the history.
fn trace_capture(shape: CaptureShape, visible: &geometry::VisibleRegion, changed: bool) {
    let mut latest = LATEST_CAPTURE_SHAPE_FOR_REPORT
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *latest = Some(shape);
    drop(latest);
    let line = capture_geometry_line(shape, visible);
    match capture_log_level(changed) {
        CaptureLogLevel::Info => log::info!("{line}"),
        CaptureLogLevel::Debug => log::debug!("{line}"),
    }
}

fn capture_geometry_line(shape: CaptureShape, visible: &geometry::VisibleRegion) -> String {
    format!(
        "[DEBUG-capture] rect source {} capture backend {} window={},{} {}x{} monitor={},{} {}x{} region={:?}",
        shape.origin.label(),
        shape.frame_backend.label(),
        shape.rect.x,
        shape.rect.y,
        shape.rect.width,
        shape.rect.height,
        shape.monitor_x,
        shape.monitor_y,
        shape.monitor_width,
        shape.monitor_height,
        visible,
    )
}

fn prepare_captures(
    captures: Vec<(WindowRect, MonitorFrame)>,
) -> Result<Vec<(WindowRect, MonitorFrame, geometry::VisibleRegion)>, &'static str> {
    let prepared = captures
        .into_iter()
        .filter_map(|(rect, monitor)| {
            let visible = geometry::visible_region(
                rect,
                monitor.origin_x,
                monitor.origin_y,
                monitor.width,
                monitor.height,
            )?;
            Some((rect, monitor, visible))
        })
        .collect::<Vec<_>>();
    if prepared.is_empty() {
        Err("the game window is not on any monitor")
    } else {
        Ok(prepared)
    }
}

/// The live capture path: rect from whichever source can answer, pixels from whichever backend
/// this session supports.
///
/// Holds its backends rather than rebuilding them per poll, because the portal session is
/// expensive to negotiate and must not be renegotiated every two seconds.
pub struct GameCapture {
    session: SessionKind,
    x11: x11::X11Capture,
    last_shapes: Vec<CaptureShape>,
    #[cfg(windows)]
    dxgi: dxgi::DxgiCapture,
    #[cfg(target_os = "linux")]
    direct: direct::DirectCapture,
    #[cfg(target_os = "linux")]
    kwin: kwin::KwinCapture,
    #[cfg(target_os = "linux")]
    portal: portal::PortalCapture,
}

impl Default for GameCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl GameCapture {
    pub fn new() -> Self {
        Self {
            session: session_kind(),
            x11: x11::X11Capture::new(),
            last_shapes: Vec::new(),
            #[cfg(windows)]
            dxgi: dxgi::DxgiCapture::new(),
            #[cfg(target_os = "linux")]
            direct: direct::DirectCapture::new(),
            #[cfg(target_os = "linux")]
            kwin: kwin::KwinCapture::new(),
            #[cfg(target_os = "linux")]
            portal: portal::PortalCapture::new(),
        }
    }

    /// What this desktop can offer, asked only when it can still matter.
    ///
    /// An XWayland game or an X11 session never reaches a Wayland backend, so probing one would
    /// put a D-Bus round trip and a Wayland handshake in a poll that had already succeeded.
    ///
    /// Walks [`WAYLAND_LADDER`] and stops at the first rung that answers, so whichever backend
    /// wins, the ones below it are never probed and a wlroots desktop never touches D-Bus. The
    /// order lives in the ladder, not here.
    #[cfg(target_os = "linux")]
    fn backend_availability(&mut self) -> BackendAvailability {
        let mut available = BackendAvailability::NONE;
        for rung in WAYLAND_LADDER {
            if self.probe(rung.frame_backend) {
                available.offer(rung.frame_backend);
                break;
            }
        }
        available
    }

    /// Ask one whole-monitor backend whether it can capture right now.
    #[cfg(target_os = "linux")]
    fn probe(&mut self, backend: FrameBackend) -> bool {
        match backend {
            // X11 is decided by window discovery, never probed as a whole-monitor backend.
            FrameBackend::X11 => false,
            FrameBackend::Wayland => self.direct.available(),
            FrameBackend::Kwin => self.kwin.available(),
            FrameBackend::Portal => portal::PortalCapture::has_live_session(),
        }
    }

    /// Every plausible game rectangle and its window-sized frame.
    pub fn capture_candidates(&mut self) -> Result<Vec<CapturedFrame>, &'static str> {
        let x11_rect = self.x11.game_rect();
        // Only a native-Wayland game needs a whole-monitor backend; anything else is answered.
        let needs_wayland_backend = x11_rect.is_err() && self.session == SessionKind::Wayland;

        #[cfg(target_os = "linux")]
        let available = if needs_wayland_backend {
            self.backend_availability()
        } else {
            BackendAvailability::NONE
        };
        #[cfg(not(target_os = "linux"))]
        let available = {
            let _ = needs_wayland_backend;
            BackendAvailability::NONE
        };

        let CaptureChoice {
            rect_origin,
            frame_backend,
            x11_rect,
        } = capture_choice(self.session, x11_rect, available)?;

        #[cfg(target_os = "linux")]
        let captures = match frame_backend {
            FrameBackend::X11 => {
                let rect = x11_rect.ok_or("no Warframe window found")?;
                let frame = match x11_frame_target(self.session) {
                    X11FrameTarget::Monitor => self.x11.capture_monitor(rect)?,
                    X11FrameTarget::Window => self.x11.capture_window(rect)?,
                };
                vec![(rect, frame)]
            }
            FrameBackend::Wayland => self.direct.capture_monitors()?,
            // A KWin failure stays a KWin failure: falling through to the portal here would
            // trade a fixable authorization error for a permission prompt mid-mission.
            FrameBackend::Kwin => self.kwin.capture_monitors()?,
            FrameBackend::Portal => self.portal.capture_monitors()?,
        };
        #[cfg(windows)]
        let captures = {
            let rect = x11_rect.ok_or("no Warframe window found")?;
            vec![(rect, self.dxgi.capture_monitor(rect)?)]
        };
        // Other non-Linux platforms retain xcap monitor capture. Windows never compiles it.
        #[cfg(not(any(target_os = "linux", windows)))]
        let captures = {
            let rect = x11_rect.ok_or("no Warframe window found")?;
            let monitor = self.x11.capture_monitor(rect)?;
            vec![(rect, monitor)]
        };

        let prepared = prepare_captures(captures)?
            .into_iter()
            .map(|(rect, monitor, visible)| {
                let shape = CaptureShape {
                    origin: rect_origin,
                    frame_backend,
                    rect,
                    monitor_x: monitor.origin_x,
                    monitor_y: monitor.origin_y,
                    monitor_width: monitor.width,
                    monitor_height: monitor.height,
                };
                (rect, monitor, visible, shape)
            })
            .collect::<Vec<_>>();
        let current_shapes = prepared
            .iter()
            .map(|(_, _, _, shape)| *shape)
            .collect::<Vec<_>>();
        let changed = update_capture_shapes(&mut self.last_shapes, current_shapes);
        let candidates = prepared
            .into_iter()
            .map(|(rect, monitor, visible, shape)| {
                trace_capture(shape, &visible, changed);
                CapturedFrame {
                    rect,
                    image: geometry::window_frame_from_monitor(
                        &monitor.image,
                        monitor.width,
                        monitor.height,
                        rect,
                        visible,
                    ),
                    rect_origin,
                    frame_backend,
                }
            })
            .collect();
        Ok(candidates)
    }
}

/// Whether the current session exposes a capturable Warframe X11/XWayland window.
///
/// This only probes window discovery; it does not capture a frame or retain mutable capture state.
pub fn x11_game_window_available() -> bool {
    x11::X11Capture::new().game_rect().is_ok()
}

/// One OCR candidate with the exact geometry and sources that produced it.
pub struct CapturedFrame {
    pub rect: WindowRect,
    pub image: image::DynamicImage,
    pub rect_origin: RectOrigin,
    pub frame_backend: FrameBackend,
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::sync::Mutex;

    use crate::overlay_window::WindowRect;

    use super::{
        BackendAvailability, CaptureShape, FrameBackend, MonitorFrame, RectOrigin, SessionKind,
        WAYLAND_LADDER, capture_choice, capture_shapes_changed, capture_sources,
        capture_sources_from, prepare_captures, session_kind_from, update_capture_shapes,
    };
    #[cfg(target_os = "linux")]
    use super::{X11FrameTarget, x11_frame_target};

    fn shape(origin: RectOrigin, backend: FrameBackend, x: i32) -> CaptureShape {
        CaptureShape {
            origin,
            frame_backend: backend,
            rect: WindowRect {
                x,
                y: 0,
                width: 1920,
                height: 1080,
            },
            monitor_x: x,
            monitor_y: 0,
            monitor_width: 1920,
            monitor_height: 1080,
        }
    }

    fn monitor_frame(x: i32, width: u32, height: u32) -> MonitorFrame {
        MonitorFrame {
            image: image::RgbaImage::new(width.max(1), height.max(1)),
            origin_x: x,
            origin_y: 0,
            width,
            height,
        }
    }

    /// Mutation caught: `collect::<Result<Vec<_>, _>>()` discarded the valid second monitor when
    /// the first selected stream had zero logical width after a compositor metadata change.
    #[test]
    fn capture_preparation_skips_invalid_geometry_and_keeps_valid_candidates() {
        let invalid = WindowRect {
            x: 0,
            y: 0,
            width: 0,
            height: 1080,
        };
        let valid = WindowRect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };

        let prepared = prepare_captures(vec![
            (invalid, monitor_frame(0, 0, 1080)),
            (valid, monitor_frame(1920, 1920, 1080)),
        ])
        .expect("the valid monitor must survive");

        assert_eq!(prepared.len(), 1);
        assert_eq!(prepared[0].0, valid);
    }

    #[test]
    fn capture_preparation_reports_no_monitor_when_every_candidate_is_invalid() {
        let invalid = WindowRect {
            x: 0,
            y: 0,
            width: 0,
            height: 1080,
        };
        assert_eq!(
            prepare_captures(vec![(invalid, monitor_frame(0, 0, 1080))]).err(),
            Some("the game window is not on any monitor")
        );
    }

    /// Mutation caught: using `.lock().ok()?` would turn one panic while holding the snapshot
    /// lock into an unknown backend for every subsequent report.
    #[test]
    fn the_report_backend_recovers_a_poisoned_local_snapshot() {
        let snapshot = Mutex::new(Some(shape(RectOrigin::Portal, FrameBackend::Portal, 0)));
        let _ = std::panic::catch_unwind(|| {
            let _guard = snapshot.lock().unwrap();
            panic!("poison the local report snapshot");
        });

        assert_eq!(capture_sources_from(&snapshot), Some(("portal", "portal")));
    }

    /// Mutation caught: sharing suppression state between `GameCapture` instances would suppress
    /// a second instance's first capture and let one instance hide another's real transition.
    #[test]
    fn capture_instances_track_log_worthy_changes_independently() {
        let x = shape(RectOrigin::X11, FrameBackend::X11, 0);
        let y = shape(RectOrigin::X11, FrameBackend::X11, 1920);
        let mut first = Vec::new();
        let mut second = Vec::new();

        assert!(update_capture_shapes(&mut first, vec![x]));
        assert!(update_capture_shapes(&mut second, vec![x]));
        assert!(!update_capture_shapes(&mut first, vec![x]));
        assert!(!update_capture_shapes(&mut second, vec![x]));
        assert!(update_capture_shapes(&mut second, vec![y]));
        assert!(update_capture_shapes(&mut first, vec![y]));
    }

    /// Mutation caught: treating `last_shapes` as an ever-seen cache would suppress the return to
    /// layout A even though the immediately previous poll observed layout B.
    #[test]
    fn returning_to_a_previous_candidate_set_is_a_new_geometry_change() {
        let a = vec![shape(RectOrigin::Portal, FrameBackend::Portal, 0)];
        let b = vec![
            shape(RectOrigin::Portal, FrameBackend::Portal, 0),
            shape(RectOrigin::Portal, FrameBackend::Portal, 1920),
        ];
        assert!(capture_shapes_changed(&[], &a));
        assert!(capture_shapes_changed(&a, &b));
        assert!(capture_shapes_changed(&b, &a));
        assert!(!capture_shapes_changed(&a, &a));
    }

    /// `WAYLAND_DISPLAY` is the reliable signal: it is set by the compositor for its own
    /// clients and absent on a pure X11 login. `XDG_SESSION_TYPE` is set by the session
    /// manager and is the fallback, because a bare compositor may leave it unset.
    #[test]
    fn a_wayland_socket_means_a_wayland_session() {
        assert_eq!(
            session_kind_from(Some(OsStr::new("wayland-1")), None),
            SessionKind::Wayland
        );
    }

    #[test]
    fn the_session_type_decides_when_no_socket_is_named() {
        assert_eq!(
            session_kind_from(None, Some(OsStr::new("wayland"))),
            SessionKind::Wayland
        );
        assert_eq!(
            session_kind_from(None, Some(OsStr::new("x11"))),
            SessionKind::X11
        );
    }

    /// An empty variable is not a session. Exported-but-empty is common in launcher scripts,
    /// and treating it as Wayland would send an X11 session down the portal path.
    #[test]
    fn an_empty_socket_is_not_a_wayland_session() {
        assert_eq!(
            session_kind_from(Some(OsStr::new("")), None),
            SessionKind::X11
        );
    }

    /// Nothing set at all: assume X11, which is what every pre-Wayland install is and what the
    /// xcap path already handles.
    #[test]
    fn nothing_set_falls_back_to_x11() {
        assert_eq!(session_kind_from(None, None), SessionKind::X11);
    }

    fn game_rect() -> WindowRect {
        WindowRect {
            x: 120,
            y: 80,
            width: 1920,
            height: 1080,
        }
    }

    /// Every backend on offer, so a test that means "X11 still wins" says so unambiguously.
    const ALL_BACKENDS: BackendAvailability = BackendAvailability {
        direct_wayland: true,
        kwin: true,
        portal_session: true,
    };

    const ONLY_KWIN: BackendAvailability = BackendAvailability {
        direct_wayland: false,
        kwin: true,
        portal_session: true,
    };

    const ONLY_PORTAL: BackendAvailability = BackendAvailability {
        direct_wayland: false,
        kwin: false,
        portal_session: true,
    };

    /// Mutation caught: converting the lookup to `Option` would replace this dependency reason
    /// with `no Warframe window found` on a session that has no portal fallback.
    #[test]
    fn x11_preserves_the_missing_executable_reason() {
        assert_eq!(
            capture_choice(
                SessionKind::X11,
                Err("xwininfo is not installed"),
                BackendAvailability::NONE
            )
            .unwrap_err(),
            "xwininfo is not installed"
        );
    }

    /// Mutation caught: replacing every X11 lookup failure with the dependency reason would lose
    /// the established successful-lookup/no-match diagnostic.
    #[test]
    fn x11_preserves_the_no_window_reason() {
        assert_eq!(
            capture_choice(
                SessionKind::X11,
                Err("no Warframe window found"),
                BackendAvailability::NONE
            )
            .unwrap_err(),
            "no Warframe window found"
        );
    }

    /// Mutation caught: propagating X11 lookup errors on Wayland would prevent native-Wayland
    /// capture even though any native backend can supply geometry and pixels.
    #[test]
    fn wayland_uses_the_available_native_backend_for_any_x11_lookup_error() {
        for reason in ["xwininfo is not installed", "no Warframe window found"] {
            let direct = capture_choice(SessionKind::Wayland, Err(reason), ALL_BACKENDS).unwrap();
            assert_eq!(direct.rect_origin, RectOrigin::Wayland);
            assert_eq!(direct.frame_backend, FrameBackend::Wayland);
            assert_eq!(direct.x11_rect, None);

            let kwin = capture_choice(SessionKind::Wayland, Err(reason), ONLY_KWIN).unwrap();
            assert_eq!(kwin.rect_origin, RectOrigin::Wayland);
            assert_eq!(kwin.frame_backend, FrameBackend::Kwin);

            let portal = capture_choice(SessionKind::Wayland, Err(reason), ONLY_PORTAL).unwrap();
            assert_eq!(portal.rect_origin, RectOrigin::Portal);
            assert_eq!(portal.frame_backend, FrameBackend::Portal);
            assert_eq!(portal.x11_rect, None);
        }
    }

    /// Catches a native-Wayland player with no usable backend being told the X11 lookup failed,
    /// which names a tool they do not have and an action that would not help.
    #[test]
    fn a_wayland_session_with_no_backend_names_the_permission_it_needs() {
        assert_eq!(
            capture_choice(
                SessionKind::Wayland,
                Err("no Warframe window found"),
                BackendAvailability::NONE
            )
            .unwrap_err(),
            "Desktop capture permission is required for native Wayland Warframe"
        );
    }

    #[cfg(not(windows))]
    /// Regression: an XWayland window already exposes pixels through X11. Routing it through the
    /// desktop portal is what produced a compositor chooser over live gameplay.
    #[test]
    fn wayland_uses_direct_x11_capture_when_x11_finds_the_game() {
        let rect = game_rect();
        let choice = capture_choice(SessionKind::Wayland, Ok(rect), ALL_BACKENDS).unwrap();
        assert_eq!(choice.rect_origin, RectOrigin::X11);
        assert_eq!(choice.frame_backend, FrameBackend::X11);
        assert_eq!(choice.x11_rect, Some(rect));
    }

    #[cfg(target_os = "linux")]
    /// Regression for issue #7: xcap's monitor capture enters the desktop portal on Wayland and
    /// can return desktop-sized pixels that do not share the XWayland window's coordinate space.
    /// The discovered game drawable must therefore be captured directly.
    #[test]
    fn xwayland_captures_the_game_drawable_instead_of_the_monitor() {
        assert_eq!(
            x11_frame_target(SessionKind::Wayland),
            X11FrameTarget::Window
        );
        assert_eq!(x11_frame_target(SessionKind::X11), X11FrameTarget::Monitor);
    }

    #[cfg(not(windows))]
    /// An X11 session must keep using X11 even when every Wayland backend is available.
    #[test]
    fn an_x11_session_never_reaches_for_a_wayland_backend() {
        assert_eq!(
            capture_sources(SessionKind::X11, true, ALL_BACKENDS),
            Some((RectOrigin::X11, FrameBackend::X11))
        );
        assert_eq!(capture_sources(SessionKind::X11, false, ALL_BACKENDS), None);
    }

    #[cfg(not(windows))]
    /// A Wayland session with an XWayland game keeps the real window rectangle.
    #[test]
    fn a_wayland_session_prefers_a_real_x11_rect() {
        assert_eq!(
            capture_sources(SessionKind::Wayland, true, ALL_BACKENDS),
            Some((RectOrigin::X11, FrameBackend::X11))
        );
    }

    /// Window discovery must never select xcap pixels on Windows, even if Wayland variables
    /// leaked into the process environment or a native-Wayland backend was offered.
    #[cfg(windows)]
    #[test]
    fn a_windows_window_uses_dxgi_for_its_pixels() {
        for session in [SessionKind::X11, SessionKind::Wayland] {
            let rect = game_rect();
            let choice = capture_choice(session, Ok(rect), ALL_BACKENDS).unwrap();
            assert_eq!(choice.rect_origin, RectOrigin::X11);
            assert_eq!(choice.frame_backend, FrameBackend::Dxgi);
            assert_eq!(choice.x11_rect, Some(rect));
            assert_eq!(
                capture_sources(session, true, ALL_BACKENDS),
                Some((RectOrigin::X11, FrameBackend::Dxgi))
            );
        }
    }

    /// The whole point of the KWin backend: a silent capture path must be chosen ahead of the
    /// portal, and the portal must never be chosen without a session the player already granted.
    #[test]
    fn a_native_wayland_game_prefers_every_silent_backend_over_the_portal() {
        let table = [
            (
                BackendAvailability {
                    direct_wayland: true,
                    kwin: true,
                    portal_session: true,
                },
                Some((RectOrigin::Wayland, FrameBackend::Wayland)),
            ),
            (ONLY_KWIN, Some((RectOrigin::Wayland, FrameBackend::Kwin))),
            (
                ONLY_PORTAL,
                Some((RectOrigin::Portal, FrameBackend::Portal)),
            ),
            (BackendAvailability::NONE, None),
        ];

        for (available, expected) in table {
            assert_eq!(
                capture_sources(SessionKind::Wayland, false, available),
                expected,
                "availability {available:?} chose the wrong backend"
            );
        }
    }

    /// Every rung must be reachable, and reachable only when its own flag is set.
    ///
    /// This is the invariant that keeps one ladder honest: `offers` is what the probe loop uses to
    /// record an answer and what `capture_sources` uses to select, so a new backend wired into one
    /// and not the other -- probed and never chosen, or chosen and never probed -- fails here.
    #[test]
    fn each_ladder_rung_is_selected_by_exactly_its_own_offer() {
        for rung in WAYLAND_LADDER {
            let mut available = BackendAvailability::NONE;
            available.offer(rung.frame_backend);
            assert!(
                available.offers(rung.frame_backend),
                "{:?} was offered but reads as unavailable",
                rung.frame_backend
            );
            assert_eq!(
                capture_sources(SessionKind::Wayland, false, available),
                Some((rung.rect_origin, rung.frame_backend)),
                "{:?} is probed but never selected",
                rung.frame_backend
            );
            for other in WAYLAND_LADDER {
                assert_eq!(
                    other.frame_backend == rung.frame_backend,
                    available.offers(other.frame_backend),
                    "{:?} leaked into {:?}'s availability",
                    other.frame_backend,
                    rung.frame_backend
                );
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn direct_wayland_requires_the_wlroots_screencopy_global() {
        assert!(super::direct::has_screencopy_interface([
            "wl_compositor",
            "zwlr_screencopy_manager_v1",
        ]));
        assert!(!super::direct::has_screencopy_interface([
            "wl_compositor",
            "wl_shm",
        ]));
    }
}
