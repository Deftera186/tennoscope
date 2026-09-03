//! Direct native-Wayland capture through the wlroots screencopy protocol.
//!
//! This path has no authorization UI. It is used only when the compositor advertises
//! `zwlr_screencopy_manager_v1`; other Wayland compositors fall back to the persisted portal grant.

use libwayshot_xcap::WayshotConnection;
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_registry;
use wayland_client::{Connection, Dispatch, QueueHandle};

use super::MonitorFrame;
use crate::overlay_window::WindowRect;

const SCREENCOPY_INTERFACE: &str = "zwlr_screencopy_manager_v1";
const XDG_OUTPUT_INTERFACE: &str = "zxdg_output_manager_v1";
const XDG_OUTPUT_MIN_VERSION: u32 = 3;

const AVAILABILITY_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
const CAPTURE_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(5);
const ERR_UNAVAILABLE: &str = "direct Wayland screen capture is unavailable";
const ERR_CAPTURE_FAILED: &str = "could not capture the game window";
const ERR_NO_OUTPUTS: &str = "no Warframe window found";

#[derive(Clone, Copy)]
struct AvailabilityState {
    available: bool,
    checked_at: std::time::Instant,
}

impl AvailabilityState {
    fn new(available: bool, checked_at: std::time::Instant) -> Self {
        Self {
            available,
            checked_at,
        }
    }

    fn should_probe(self, now: std::time::Instant) -> bool {
        !self.available
            && now.saturating_duration_since(self.checked_at) >= AVAILABILITY_RETRY_INTERVAL
    }

    fn record(&mut self, available: bool, checked_at: std::time::Instant) {
        self.available = available;
        self.checked_at = checked_at;
    }
}

fn probe_available() -> bool {
    let Ok(connection) = Connection::connect_to_env() else {
        return false;
    };
    let Ok((globals, _queue)) = registry_queue_init::<RegistryProbe>(&connection) else {
        return false;
    };
    globals.contents().with_list(|list| {
        has_screencopy_interface(list.iter().map(|global| global.interface.as_str()))
    })
}

fn collect_generation<T>(
    results: impl IntoIterator<Item = Result<T, &'static str>>,
) -> Result<Vec<T>, &'static str> {
    results.into_iter().collect()
}

pub(crate) fn has_screencopy_interface<I, S>(interfaces: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    interfaces
        .into_iter()
        .any(|interface| interface.as_ref() == SCREENCOPY_INTERFACE)
}

fn build_connection() -> Result<WayshotConnection, &'static str> {
    let connection = Connection::connect_to_env().map_err(|_| ERR_UNAVAILABLE)?;
    let (globals, queue) =
        registry_queue_init::<RegistryProbe>(&connection).map_err(|_| ERR_UNAVAILABLE)?;
    let has_xdg_output = globals.contents().with_list(|list| {
        list.iter().any(|global| {
            global.interface == XDG_OUTPUT_INTERFACE && global.version >= XDG_OUTPUT_MIN_VERSION
        })
    });
    if !has_xdg_output {
        return Err(ERR_UNAVAILABLE);
    }
    drop(queue);

    // `libwayshot-xcap` panics if xdg-output disappears before its second registry snapshot.
    // Preflighting covers a compositor that never offered it; catching the removal race keeps a
    // compositor restart from aborting this process. A fresh object also rediscovers every output
    // atomically, unlike the dependency's public `refresh_outputs`, which has the same panic path.
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        WayshotConnection::from_connection(connection)
    }))
    .map_err(|_| ERR_UNAVAILABLE)?
    .map_err(|_| ERR_UNAVAILABLE)
}

struct RegistryProbe;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for RegistryProbe {
    fn event(
        _state: &mut Self,
        _proxy: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _connection: &Connection,
        _queue: &QueueHandle<Self>,
    ) {
    }
}

/// Whether this compositor exposes direct wlroots screen capture.
///
/// A successful capability result is permanent for the process. A failed connection or registry
/// probe is retried at a bounded interval because a compositor can still be starting or restart;
/// the interval keeps the 400 ms capture path from opening a Wayland connection every poll.
pub fn available() -> bool {
    static AVAILABLE: std::sync::LazyLock<std::sync::Mutex<AvailabilityState>> =
        std::sync::LazyLock::new(|| {
            let now = std::time::Instant::now();
            std::sync::Mutex::new(AvailabilityState::new(probe_available(), now))
        });
    let now = std::time::Instant::now();
    let mut state = AVAILABLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.should_probe(now) {
        state.record(probe_available(), now);
    }
    state.available
}

/// A persistent direct connection. Output discovery and protocol setup happen once per reader,
/// rather than once per 400 ms capture poll.
pub struct DirectCapture {
    connection: Option<WayshotConnection>,
    retry_after: Option<std::time::Instant>,
}

impl Default for DirectCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl DirectCapture {
    pub const fn new() -> Self {
        Self {
            connection: None,
            retry_after: None,
        }
    }

    /// Whether this reader should attempt wlroots capture now.
    ///
    /// The compositor-level probe stays positive while the protocol is advertised. A reader that
    /// cannot obtain frames backs off briefly so KWin or an already-authorized portal session can
    /// take over instead of losing every poll to the same broken direct path.
    pub fn available(&self) -> bool {
        direct_retry_ready(self.retry_after, std::time::Instant::now()) && available()
    }

    fn connection(&mut self) -> Result<&mut WayshotConnection, &'static str> {
        if self.connection.is_none() {
            self.connection = Some(build_connection()?);
        }
        self.connection.as_mut().ok_or(ERR_UNAVAILABLE)
    }

    /// Capture one complete output generation.
    fn capture_generation(
        connection: &mut WayshotConnection,
    ) -> Result<Vec<(WindowRect, MonitorFrame)>, &'static str> {
        if connection.get_all_outputs().is_empty() {
            return Err(ERR_NO_OUTPUTS);
        }

        let outputs = connection.get_all_outputs().to_vec();
        collect_generation(outputs.into_iter().map(|output| {
            let region = output.logical_region.inner;
            let rect = WindowRect {
                x: region.position.x,
                y: region.position.y,
                width: region.size.width,
                height: region.size.height,
            };
            let image = connection
                .screenshot_outputs(std::slice::from_ref(&output), false)
                .map_err(|_| ERR_CAPTURE_FAILED)?;
            Ok((
                rect,
                MonitorFrame {
                    image: image.to_rgba8(),
                    origin_x: rect.x,
                    origin_y: rect.y,
                    width: rect.width,
                    height: rect.height,
                },
            ))
        }))
    }

    fn capture_once(&mut self) -> Result<Vec<(WindowRect, MonitorFrame)>, &'static str> {
        let connection = self.connection()?;
        Self::capture_generation(connection)
    }

    /// Capture every output as an OCR candidate. Native Wayland does not expose another client's
    /// window geometry; a borderless Warframe window occupies one of these monitor rectangles.
    ///
    /// Any protocol failure drops the whole connection and retries one freshly discovered output
    /// generation. `libwayshot-xcap` itself uses an unbounded blocking dispatch loop, so a call
    /// already stuck inside that dependency remains a residual risk; no thread is abandoned here.
    pub fn capture_monitors(&mut self) -> Result<Vec<(WindowRect, MonitorFrame)>, &'static str> {
        let first = self.capture_once();
        if first.is_ok() {
            self.retry_after = None;
            return first;
        }

        self.connection = None;
        let retry = self.capture_once();
        match retry {
            Ok(frames) => {
                self.retry_after = None;
                Ok(frames)
            }
            Err(_) => {
                self.connection = None;
                self.retry_after = Some(std::time::Instant::now() + CAPTURE_RETRY_INTERVAL);
                // The first failure describes the established path. Rebuilding can fail for a
                // secondary reason, which must not overwrite the cause that triggered recovery.
                first
            }
        }
    }
}

fn direct_retry_ready(retry_after: Option<std::time::Instant>, now: std::time::Instant) -> bool {
    retry_after.is_none_or(|deadline| now >= deadline)
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn negative_probe_retries_after_interval_and_success_stays_cached() {
        let start = std::time::Instant::now();
        let mut state = AvailabilityState::new(false, start);

        assert!(!state.should_probe(start + AVAILABILITY_RETRY_INTERVAL / 2));
        assert!(state.should_probe(start + AVAILABILITY_RETRY_INTERVAL));

        state.record(true, start + AVAILABILITY_RETRY_INTERVAL);
        assert!(!state.should_probe(start + AVAILABILITY_RETRY_INTERVAL * 100));
    }

    #[test]
    fn repeated_capture_failure_temporarily_yields_to_lower_priority_backends() {
        let now = std::time::Instant::now();
        let retry_after = Some(now + CAPTURE_RETRY_INTERVAL);

        assert!(!direct_retry_ready(retry_after, now));
        assert!(!direct_retry_ready(
            retry_after,
            now + CAPTURE_RETRY_INTERVAL / 2
        ));
        assert!(direct_retry_ready(
            retry_after,
            now + CAPTURE_RETRY_INTERVAL
        ));
    }

    #[test]
    fn one_failed_output_makes_the_generation_fail_closed() {
        let result = collect_generation([Ok(1_u8), Err(ERR_CAPTURE_FAILED)]);
        assert_eq!(result, Err(ERR_CAPTURE_FAILED));
    }
}
