//! Portal ScreenCast + PipeWire: fallback capture for native Wayland compositors without wlroots
//! screencopy.
//!
//! `xcap`'s own Wayland branch is unusable for polling -- it calls the portal `Screenshot`
//! with `interactive: false, modal: true`, which is a permission dialog every two seconds.
//! ScreenCast restores an explicitly authorized grant at startup; gameplay communicates only with
//! that live session and never negotiates with the portal.

pub mod stream;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{LazyLock, Mutex, mpsc};
use std::{collections::HashMap, collections::hash_map::Entry};

use ashpd::desktop::screencast::{
    CursorMode, Screencast, SelectSourcesOptions, SourceType, StartCastOptions,
};
use ashpd::desktop::{CreateSessionOptions, PersistMode, Session};
use ashpd::enumflags2::BitFlags;

use self::stream::{NodeStream, frame_to_rgba};
use super::{GameFrameSource, MonitorFrame};
use crate::overlay_window::WindowRect;

/// One monitor the portal is willing to hand us, and where it sits.
#[derive(Clone, Copy, Debug)]
pub struct PortalStream {
    pub node_id: u32,
    pub position: Option<(i32, i32)>,
    pub size: Option<(i32, i32)>,
}

/// A stream's monitor rectangle, when it reported one.
///
/// For a borderless-fullscreen game this is the game rect. That is the whole reason the portal
/// can substitute for window enumeration: Wayland will never tell us where another client's
/// window is, but it does tell us where the monitor is, and a fullscreen game fills it.
pub fn stream_rect(stream: &PortalStream) -> Option<WindowRect> {
    let (x, y) = stream.position?;
    let (width, height) = stream.size?;
    if width <= 0 || height <= 0 {
        return None;
    }
    x.checked_add(width)?;
    y.checked_add(height)?;
    Some(WindowRect {
        x,
        y,
        width: u32::try_from(width).ok()?,
        height: u32::try_from(height).ok()?,
    })
}

/// Every portal node whose compositor metadata describes a usable logical monitor.
pub fn streams_with_geometry(streams: &[PortalStream]) -> Vec<(u32, WindowRect)> {
    streams
        .iter()
        .filter_map(|stream| Some((stream.node_id, stream_rect(stream)?)))
        .collect()
}

/// Pick the portal stream containing a known X11 game rectangle.
///
/// A hotplug race that leaves no match degrades to the first stream with valid geometry, because a
/// probably-wrong monitor OCR can reject is better than no capture. Native Wayland does not use
/// this fallback; it reads every valid stream through `streams_with_geometry`.
pub fn pick_stream(streams: &[PortalStream], rect: WindowRect) -> Option<&PortalStream> {
    if let Some(found) = streams.iter().find(|stream| {
        stream_rect(stream).is_some_and(|monitor| {
            let (Ok(width), Ok(height)) =
                (i32::try_from(monitor.width), i32::try_from(monitor.height))
            else {
                return false;
            };
            let (Some(right), Some(bottom)) =
                (monitor.x.checked_add(width), monitor.y.checked_add(height))
            else {
                return false;
            };
            rect.x >= monitor.x && rect.y >= monitor.y && rect.x < right && rect.y < bottom
        })
    }) {
        return Some(found);
    }
    log::debug!(
        "[DEBUG-capture] no portal stream contains the game rect at ({}, {}); \
         falling back to the first usable of {} stream(s)",
        rect.x,
        rect.y,
        streams.len()
    );
    streams.iter().find(|stream| stream_rect(stream).is_some())
}

/// Where an explicit grant is remembered between runs.
fn token_path() -> Option<PathBuf> {
    Some(
        dirs_next_config()?
            .join("tennoscope")
            .join("screencast.token"),
    )
}

/// `$XDG_CONFIG_HOME`, or `$HOME/.config`.
fn dirs_next_config() -> Option<PathBuf> {
    if let Some(config) = std::env::var_os("XDG_CONFIG_HOME").filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(config));
    }
    std::env::var_os("HOME")
        .filter(|value| !value.is_empty())
        .map(|home| PathBuf::from(home).join(".config"))
}

fn saved_token() -> Option<String> {
    let token = std::fs::read_to_string(token_path()?).ok()?;
    let token = token.trim().to_owned();
    (!token.is_empty()).then_some(token)
}

/// Forget the persisted grant so the next authorization starts from the chooser.
fn discard_token() -> std::io::Result<()> {
    let Some(path) = token_path() else {
        return Ok(());
    };
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Whether the player has completed capture setup before.
///
/// This controls only first-run UI. A token is not treated as a live or non-interactive runtime
/// capability: gameplay capture reads only an already-open worker session.
pub fn has_saved_grant() -> bool {
    saved_token().is_some()
}

/// Persist the grant at `0600`.
///
/// A restore token is a capability -- anything that can read it can re-open a screen cast of this
/// desktop -- so it does not get the default `0644`. `OpenOptions::mode` only applies to a file
/// this call creates, so a token file left behind at a broader mode is tightened explicitly. Both
/// happen before any byte is written, so the token is never briefly world-readable.
fn save_token(token: &str) {
    let Some(path) = token_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if save_token_at(&path, token).is_err() {
        log::debug!("[DEBUG-capture] could not persist the screencast token");
    }
}

fn save_token_at(path: &Path, token: &str) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
    file.set_len(0)?;
    file.write_all(token.as_bytes())
}

/// The negotiated result of a screencast grant: which PipeWire nodes may be read, and where the
/// monitor behind each one sits.
///
/// It retains the portal `Session`, because that handle is the only thing that can end the cast.
/// ashpd has no `Drop` impl for `Session` -- it is closed only by an explicit `Close` call -- so
/// without this the compositor's screen-sharing indicator stays lit for the whole process
/// lifetime, long after the last fissure ended. `Drop` makes that call.
pub struct PortalSession {
    streams: Vec<PortalStream>,
    /// `Option` only so `Drop` can take ownership: `close` consumes the session, and `drop` gets
    /// `&mut self`. It is `Some` for the whole useful life of the value.
    session: Option<Session<Screencast>>,
}

impl Drop for PortalSession {
    fn drop(&mut self) {
        let Some(session) = self.session.take() else {
            return;
        };
        // A nested `block_on` panics with "cannot start a runtime from within a runtime", and a
        // panic in `Drop` aborts the process. Every path today drops this on the poller's plain
        // thread with no ambient runtime, so the close happens inline.
        if tokio::runtime::Handle::try_current().is_err() {
            close_blocking(session);
            return;
        }
        // Dropped inside a runtime. The close moves to a fresh thread -- which has no ambient
        // runtime, so it can block -- rather than being skipped: skipping it leaves the cast live
        // and the compositor's screen-sharing indicator lit for the rest of the process, which is
        // the exact leak retaining the session was meant to fix.
        //
        // `Builder::spawn` rather than `thread::spawn`, because the latter panics when a thread
        // cannot be created and a panic here would abort.
        if let Err(error) = std::thread::Builder::new()
            .name("tennoscope-cast-close".to_owned())
            .spawn(move || close_blocking(session))
        {
            log::warn!(
                "[DEBUG-capture] could not spawn a thread to close the screencast session \
                 ({error}); the screen-sharing indicator may stay lit until this process exits"
            );
        }
    }
}

/// Close a cast from a thread that has no ambient runtime.
///
/// Blocking is safe here by construction: both callers have already established that no runtime is
/// driving the current thread. `warn`, not `debug`, on failure -- a cast the compositor still
/// believes is live is a visible indicator the user cannot account for, so it belongs in a report.
fn close_blocking(session: Session<Screencast>) {
    let Ok(runtime) = portal_runtime() else {
        log::warn!(
            "[DEBUG-capture] no runtime to close the screencast session with; \
             the screen-sharing indicator may stay lit until this process exits"
        );
        return;
    };
    if runtime.block_on(session.close()).is_err() {
        log::warn!(
            "[DEBUG-capture] could not close the screencast session; \
             the screen-sharing indicator may stay lit until this process exits"
        );
    }
}

/// The runtime ashpd's cached D-Bus connection is built on.
///
/// It has to outlive that connection. ashpd caches one `zbus::Connection` for the life of the
/// process (`proxy.rs`'s `static SESSION`), and zbus with the tokio feature spawns that
/// connection's background tasks onto whatever runtime was current when it was built. A runtime
/// built per call takes those tasks with it when it drops: measured, the first handshake succeeds
/// and every later one waits five seconds for a reply nothing will ever read. The poller is a
/// plain thread with no timeout on this path, so that is a silent permanent stall.
///
/// Multi-thread rather than current-thread, for two reasons that both come down to those
/// background tasks needing a driver of their own. A current-thread runtime only advances tasks
/// while some thread sits inside `block_on`, so the connection's socket reader stops between
/// calls. And its `block_on` takes exclusive hold of the single scheduler core, so a
/// `PortalSession::drop` closing a cast would queue behind an interactive handshake for up to
/// `HANDSHAKE_DEADLINE` -- on the frame worker's thread, that is a minute-long capture stall. One
/// worker thread is enough: the D-Bus work here is entirely IO-bound.
fn portal_runtime() -> Result<&'static tokio::runtime::Runtime, &'static str> {
    static RUNTIME: LazyLock<Result<tokio::runtime::Runtime, &'static str>> = LazyLock::new(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .thread_name("tennoscope-portal")
            .enable_all()
            .build()
            .map_err(|_| "could not start the portal runtime")
    });
    RUNTIME.as_ref().map_err(|reason| *reason)
}

/// How long an explicit authorization handshake may take before the calling thread is returned.
///
/// Generous on purpose: a person may need to notice the desktop dialog, read it, and choose one or
/// more monitors. A bound is still required because wlroots may leave `slurp` waiting indefinitely.
const HANDSHAKE_DEADLINE: std::time::Duration = std::time::Duration::from_secs(60);

impl PortalSession {
    /// Open the chooser and create or replace the persisted monitor grant.
    pub fn authorize() -> Result<Self, &'static str> {
        Self::open(None, "the screen capture prompt went unanswered")
    }

    /// Re-open the monitor grant selected earlier. This is the returning-launch path; gameplay
    /// capture itself never negotiates with the portal.
    pub fn restore() -> Result<Self, &'static str> {
        let token = saved_token().ok_or("screen capture has not been set up")?;
        Self::open(
            Some(&token),
            "the saved screen capture grant could not be restored",
        )
    }

    fn open(
        restore_token: Option<&str>,
        timeout_reason: &'static str,
    ) -> Result<Self, &'static str> {
        let runtime = portal_runtime()?;
        runtime.block_on(Self::negotiate(restore_token, timeout_reason))
    }

    /// Negotiate a cast, keeping ownership of the session across every fallible phase.
    ///
    /// The deadline is applied per await rather than by wrapping the whole handshake in a single
    /// `timeout`. That distinction is the whole point: `ashpd`'s `Session` does not close on drop,
    /// so a future cancelled while it owned the session would abandon a cast the portal still
    /// considers live -- a screen-sharing indicator the user cannot turn off. Timing out inside the
    /// handshake instead lets every exit run `close_unused`.
    async fn negotiate(
        restore_token: Option<&str>,
        timeout_reason: &'static str,
    ) -> Result<Self, &'static str> {
        let deadline = tokio::time::Instant::now() + HANDSHAKE_DEADLINE;
        let proxy = tokio::time::timeout_at(deadline, Screencast::new())
            .await
            .map_err(|_| timeout_reason)?
            .map_err(|_| "the desktop portal is unavailable")?;
        let session = tokio::time::timeout_at(
            deadline,
            proxy.create_session(CreateSessionOptions::default()),
        )
        .await
        .map_err(|_| timeout_reason)?
        .map_err(|_| "the portal refused a screencast session")?;

        // `CursorMode::Hidden` is not cosmetic: the pointer rests over the reward cards while
        // the player is choosing, and a composited cursor lands inside an OCR crop.
        let options = SelectSourcesOptions::default()
            .set_multiple(true)
            .set_cursor_mode(CursorMode::Hidden)
            .set_sources(BitFlags::from(SourceType::Monitor))
            .set_persist_mode(PersistMode::ExplicitlyRevoked)
            .set_restore_token(restore_token);
        // From here on the session exists, so failures close it instead of returning early.
        let selected = match tokio::time::timeout_at(
            deadline,
            proxy.select_sources(&session, options),
        )
        .await
        {
            Err(_) => Err(timeout_reason),
            Ok(Err(_)) => Err("the portal refused the monitor selection"),
            Ok(Ok(request)) => request
                .response()
                .map_err(|_| "the portal rejected the monitor selection"),
        };
        if let Err(reason) = selected {
            close_unused(session).await;
            return Err(reason);
        }

        let started = match tokio::time::timeout_at(
            deadline,
            proxy.start(&session, None, StartCastOptions::default()),
        )
        .await
        {
            Err(_) => Err(timeout_reason),
            Ok(Err(_)) => Err("the portal could not start the cast"),
            Ok(Ok(request)) => request
                .response()
                .map_err(|_| "the portal could not start the cast"),
        };
        let started = match started {
            Ok(started) => started,
            Err(reason) => {
                close_unused(session).await;
                return Err(reason);
            }
        };

        if let Some(token) = started.restore_token() {
            save_token(token);
        }

        let streams = started
            .streams()
            .iter()
            .map(|stream| PortalStream {
                node_id: stream.pipe_wire_node_id(),
                position: stream.position(),
                size: stream.size(),
            })
            .collect::<Vec<_>>();
        if streams.is_empty() {
            // A grant can resolve to no monitors when its outputs changed. `Start` already
            // succeeded, so close the otherwise-live cast before returning the actionable error.
            close_unused(session).await;
            return Err("the portal returned no monitors");
        }
        // Position, not just size: position is the half that decides which monitor gets picked, so
        // it is the half a report needs when a wrong-monitor read is being diagnosed.
        log::info!(
            "[DEBUG-capture] portal streams={} first_node={} position={:?} size={:?}",
            streams.len(),
            streams[0].node_id,
            streams[0].position,
            streams[0].size
        );
        Ok(Self {
            streams,
            session: Some(session),
        })
    }

    pub fn streams(&self) -> &[PortalStream] {
        &self.streams
    }
}

/// End a cast that was started but will not be read.
///
/// Only for the failure paths that come after `Start` succeeded. The happy path hands the session
/// to `PortalSession`, whose `Drop` closes it instead.
async fn close_unused(session: Session<Screencast>) {
    if session.close().await.is_err() {
        log::warn!("[DEBUG-capture] could not close an unused screencast session");
    }
}

/// How long a single poll will wait for a frame.
///
/// The cold run measured the first frame arriving well after the handshake returned, so a poll has
/// to wait rather than assume a buffer is ready. Two seconds because the poller's own interval is
/// two seconds: waiting longer would stack polls up behind each other.
const FRAME_DEADLINE: std::time::Duration = std::time::Duration::from_secs(2);

/// A live portal session and its PipeWire streams.
///
/// PipeWire's local main loop is not `Send`, so this value never leaves the dedicated worker
/// thread that created it. Other threads exchange owned images with that worker over channels.
struct LivePortalCapture {
    session: PortalSession,
    streams: HashMap<u32, NodeStream>,
}

impl LivePortalCapture {
    fn from_session(session: PortalSession) -> Self {
        Self {
            session,
            streams: HashMap::new(),
        }
    }

    fn capture_node(
        &mut self,
        node_id: u32,
        monitor: WindowRect,
    ) -> Result<MonitorFrame, &'static str> {
        let frame = read_cached_node(
            &mut self.streams,
            node_id,
            &mut NodeStream::open,
            |stream| stream.next_frame(FRAME_DEADLINE),
        )?;
        Ok(monitor_frame(frame_to_rgba(&frame), monitor))
    }

    fn capture_monitors(&mut self) -> Result<Vec<(WindowRect, MonitorFrame)>, &'static str> {
        let monitors = streams_with_geometry(self.session.streams());
        if monitors.is_empty() {
            return Err("no Warframe window found");
        }

        let mut captured = Vec::with_capacity(monitors.len());
        let mut last_error = None;
        for (node_id, rect) in monitors {
            match self.capture_node(node_id, rect) {
                Ok(frame) => captured.push((rect, frame)),
                // A teardown is reported even when another monitor produced a frame: the session
                // is gone, so returning success here would leave it marked live and never
                // re-prompt. Any other reason is per-node and must not fail a usable capture.
                Err(ERR_SESSION_ENDED) => return Err(ERR_SESSION_ENDED),
                Err(reason) => last_error = Some(reason),
            }
        }
        if captured.is_empty() {
            Err(last_error.unwrap_or(ERR_NO_FRAME))
        } else {
            Ok(captured)
        }
    }

    fn capture_monitor(&mut self, rect: WindowRect) -> Result<MonitorFrame, &'static str> {
        let stream =
            pick_stream(self.session.streams(), rect).ok_or("the portal returned no monitors")?;
        let monitor = stream_rect(stream).unwrap_or(rect);
        self.capture_node(stream.node_id, monitor)
    }
}

/// Whether the worker currently holds an authorized session.
///
/// Published as a flag rather than answered by the worker, because the worker serializes frame
/// reads and a status poll must not queue behind a compositor frame deadline.
static SESSION_LIVE: AtomicBool = AtomicBool::new(false);
static SESSION_DIAGNOSTIC: Mutex<Option<&'static str>> = Mutex::new(None);
pub(super) const ERR_SESSION_ENDED: &str = "the screen capture session ended";
/// A frame that did not arrive inside the deadline. Retryable on the same session, unlike
/// [`ERR_SESSION_ENDED`].
pub(super) const ERR_NO_FRAME: &str = "could not capture the game window";
/// No session was ever installed on the worker, as opposed to one that ended.
///
/// Distinct from [`ERR_SESSION_ENDED`] because the two describe opposite histories and only one of
/// them is a transition: a session that ended is a capability the user had and lost, which is worth
/// latching as a diagnostic, while this is the ordinary state before setup has ever run. Reporting
/// the latter as the former both misdescribes it to the user and publishes a teardown that never
/// happened.
pub(super) const ERR_NO_SESSION: &str =
    "screen capture permission is not set up for this app session";

fn publish_session_diagnostic(error: &'static str) {
    *SESSION_DIAGNOSTIC
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(error);
}

/// Returns a terminal portal transition once so Diagnostics can explain why authorization is
/// offered again, without putting status reads onto the frame-capture worker.
pub fn take_session_diagnostic() -> Option<&'static str> {
    SESSION_DIAGNOSTIC
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .take()
}

struct WorkerCapture<T> {
    capture: Option<T>,
    live: &'static AtomicBool,
}

impl<T> WorkerCapture<T> {
    const fn new(live: &'static AtomicBool) -> Self {
        Self {
            capture: None,
            live,
        }
    }

    fn install(&mut self, capture: T) {
        let previous = self.capture.replace(capture);
        drop(previous);
        self.live.store(true, Ordering::Release);
        SESSION_DIAGNOSTIC
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }

    /// Only the tests read this directly; production code consults the published
    /// `SESSION_LIVE` flag through `PortalCapture::has_live_session`.
    #[cfg(test)]
    const fn has_live_session(&self) -> bool {
        self.capture.is_some()
    }

    fn close(&mut self) {
        drop(self.capture.take());
        self.live.store(false, Ordering::Release);
    }

    fn record_capture_result<U>(
        &mut self,
        result: Result<U, &'static str>,
    ) -> Result<U, &'static str> {
        if matches!(result, Err(ERR_SESSION_ENDED)) {
            self.close();
            publish_session_diagnostic(ERR_SESSION_ENDED);
        }
        result
    }
}

enum WorkerRequest {
    Install(PortalSession, mpsc::SyncSender<Result<(), &'static str>>),
    CaptureMonitor(
        WindowRect,
        mpsc::SyncSender<Result<MonitorFrame, &'static str>>,
    ),
    CaptureMonitors(mpsc::SyncSender<Result<Vec<(WindowRect, MonitorFrame)>, &'static str>>),
    Close(mpsc::SyncSender<Result<(), &'static str>>),
}

fn with_live_capture<T>(
    capture: &mut WorkerCapture<LivePortalCapture>,
    run: impl FnOnce(&mut LivePortalCapture) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    let result = capture.capture.as_mut().ok_or(ERR_NO_SESSION).and_then(run);
    capture.record_capture_result(result)
}

fn run_worker(receiver: mpsc::Receiver<WorkerRequest>) {
    let mut capture = WorkerCapture::new(&SESSION_LIVE);
    while let Ok(request) = receiver.recv() {
        match request {
            WorkerRequest::Install(session, reply) => {
                capture.install(LivePortalCapture::from_session(session));
                let _ = reply.send(Ok(()));
            }
            WorkerRequest::CaptureMonitor(rect, reply) => {
                let result = with_live_capture(&mut capture, |live| live.capture_monitor(rect));
                let _ = reply.send(result);
            }
            WorkerRequest::CaptureMonitors(reply) => {
                let result = with_live_capture(&mut capture, LivePortalCapture::capture_monitors);
                let _ = reply.send(result);
            }
            WorkerRequest::Close(reply) => {
                capture.close();
                let _ = reply.send(Ok(()));
            }
        }
    }
    capture.close();
}

fn worker_sender() -> Result<&'static mpsc::Sender<WorkerRequest>, &'static str> {
    static WORKER: LazyLock<Result<mpsc::Sender<WorkerRequest>, &'static str>> =
        LazyLock::new(|| {
            let (sender, receiver) = mpsc::channel();
            std::thread::Builder::new()
                .name("tennoscope-capture-portal".to_owned())
                .spawn(move || run_worker(receiver))
                .map(|_| sender)
                .map_err(|_| "could not start the screen capture worker")
        });
    WORKER.as_ref().map_err(|error| *error)
}

fn worker_call<T>(
    request: impl FnOnce(mpsc::SyncSender<Result<T, &'static str>>) -> WorkerRequest,
) -> Result<T, &'static str> {
    let (reply, response) = mpsc::sync_channel(0);
    worker_sender()?
        .send(request(reply))
        .map_err(|_| "the screen capture worker stopped")?;
    response
        .recv()
        .map_err(|_| "the screen capture worker stopped")?
}

fn authorize_with_restore_fallback<T>(
    has_token: bool,
    restore: impl FnOnce() -> Result<T, &'static str>,
    discard: impl FnOnce() -> std::io::Result<()>,
    authorize: impl FnOnce() -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    if !has_token {
        return authorize();
    }
    match restore() {
        Ok(session) => Ok(session),
        Err(_) => {
            discard().map_err(|_| "the saved screen capture grant could not be replaced")?;
            authorize()
        }
    }
}

/// Channel-only handle used by gameplay capture.
///
/// Runtime capture can only ask the worker for frames. Interactive portal negotiation runs on the
/// command's blocking thread and sends the completed session here afterward, so a chooser never
/// occupies the serialized frame worker.
pub struct PortalCapture;

impl Default for PortalCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl PortalCapture {
    pub const fn new() -> Self {
        Self
    }

    pub fn authorize() -> Result<(), &'static str> {
        let session = authorize_with_restore_fallback(
            has_saved_grant(),
            PortalSession::restore,
            discard_token,
            PortalSession::authorize,
        )?;
        worker_call(|reply| WorkerRequest::Install(session, reply))
    }

    /// Whether the worker already holds a session the player authorized.
    ///
    /// Capture selection asks this instead of assuming: the portal is only a usable backend
    /// when a session already exists, because negotiating one would open a desktop chooser
    /// in the middle of a mission.
    ///
    /// Reads the published flag rather than messaging the worker, so a poll that lands during a
    /// frame read answers immediately instead of waiting out the capture.
    pub fn has_live_session() -> bool {
        SESSION_LIVE.load(Ordering::Acquire)
    }

    /// End the live session, turning off the desktop's screen-sharing indicator.
    pub fn close() -> Result<(), &'static str> {
        worker_call(WorkerRequest::Close)
    }

    pub fn capture_monitors(&mut self) -> Result<Vec<(WindowRect, MonitorFrame)>, &'static str> {
        worker_call(WorkerRequest::CaptureMonitors)
    }
}

fn monitor_frame(image: image::RgbaImage, monitor: WindowRect) -> MonitorFrame {
    MonitorFrame {
        image,
        origin_x: monitor.x,
        origin_y: monitor.y,
        width: monitor.width,
        height: monitor.height,
    }
}

fn read_cached_node<S, T>(
    cache: &mut HashMap<u32, S>,
    node_id: u32,
    open: &mut impl FnMut(u32) -> Result<S, &'static str>,
    read: impl FnOnce(&S) -> Result<T, &'static str>,
) -> Result<T, &'static str> {
    if let Entry::Vacant(entry) = cache.entry(node_id) {
        entry.insert(open(node_id)?);
    }
    // The reason is preserved rather than flattened to one message: a torn-down session and a
    // frame that merely did not arrive in time need opposite handling upstream, and collapsing
    // both into "could not capture" is what kept a dead session marked live.
    let Some(stream) = cache.get(&node_id) else {
        return Err(ERR_NO_FRAME);
    };
    match read(stream) {
        Ok(frame) => Ok(frame),
        Err(reason) => {
            // Evicted either way, so the next poll reopens the node instead of retrying a stream
            // that already failed once.
            cache.remove(&node_id);
            Err(reason)
        }
    }
}

impl GameFrameSource for PortalCapture {
    fn capture_monitor(&mut self, rect: WindowRect) -> Result<MonitorFrame, &'static str> {
        worker_call(|reply| WorkerRequest::CaptureMonitor(rect, reply))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ERR_NO_FRAME, ERR_SESSION_ENDED, HANDSHAKE_DEADLINE, PortalStream, WorkerCapture,
        authorize_with_restore_fallback, monitor_frame, pick_stream, read_cached_node,
        save_token_at, stream_rect, streams_with_geometry, with_live_capture,
    };
    use crate::overlay_window::WindowRect;
    use std::cell::Cell;
    use std::rc::Rc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[test]
    fn saving_over_an_existing_token_tightens_its_permissions_before_rewriting() {
        use std::os::unix::fs::PermissionsExt;

        let directory = tempfile::tempdir().expect("a temporary token directory");
        let path = directory.path().join("screencast.token");
        std::fs::write(&path, "old token").expect("the broad token file is created");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("the test can make the existing token broadly readable");

        save_token_at(&path, "new sensitive token").expect("the token is rewritten");

        let mode = std::fs::metadata(&path)
            .expect("the rewritten token has metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(
            std::fs::read_to_string(&path).expect("the rewritten token is readable"),
            "new sensitive token"
        );
    }

    /// A first-time grant is a human reading a dialog, so the bound has to be generous rather than
    /// machine-tight. Pinned as a floor because the failure mode of a tight value is silent: a
    /// legitimate grant is abandoned mid-prompt, the user clicks, and the click lands on a
    /// handshake nobody is waiting for any more.
    #[test]
    fn the_handshake_deadline_leaves_room_for_a_human() {
        assert!(
            HANDSHAKE_DEADLINE >= std::time::Duration::from_secs(30),
            "a first-time screen-capture prompt needs tens of seconds, not {HANDSHAKE_DEADLINE:?}"
        );
    }

    /// Runtime capture has no negotiation fallback. Without a live session, it returns before the
    /// supplied capture operation can run; only explicit authorization or startup restoration can
    /// install a completed session on the worker.
    #[test]
    fn runtime_capture_without_a_live_session_cannot_reach_capture_or_negotiation() {
        static LIVE: AtomicBool = AtomicBool::new(false);
        let mut capture = WorkerCapture::new(&LIVE);
        let mut operation_ran = false;
        let result = with_live_capture(&mut capture, |_| {
            operation_ran = true;
            Ok(())
        });

        assert_eq!(
            result.unwrap_err(),
            "screen capture permission is not set up for this app session"
        );
        assert!(!operation_ran);
    }

    #[test]
    fn failed_restore_discards_the_stale_token_then_opens_the_chooser() {
        let events = std::cell::RefCell::new(Vec::new());
        let session = authorize_with_restore_fallback(
            true,
            || {
                events.borrow_mut().push("restore");
                Err("the saved screen capture grant could not be restored")
            },
            || {
                events.borrow_mut().push("discard");
                Ok(())
            },
            || {
                events.borrow_mut().push("authorize");
                Ok(71_u32)
            },
        )
        .expect("the chooser replaces the stale grant");

        assert_eq!(session, 71);
        assert_eq!(*events.borrow(), ["restore", "discard", "authorize"]);
    }

    #[test]
    fn live_status_is_published_without_waiting_for_worker_requests() {
        static LIVE: AtomicBool = AtomicBool::new(false);
        let mut capture = WorkerCapture::new(&LIVE);
        capture.install(71_u32);

        assert!(LIVE.load(Ordering::Acquire));
    }

    #[derive(Debug)]
    struct DropProbe(Rc<Cell<usize>>);

    impl Drop for DropProbe {
        fn drop(&mut self) {
            self.0.set(self.0.get() + 1);
        }
    }

    /// Closing must both drop the session and clear the published flag: selection reads the flag,
    /// so a stale `true` would keep advertising the portal after its session ended.
    #[test]
    fn closing_worker_capture_drops_the_session_and_clears_status() {
        static LIVE: AtomicBool = AtomicBool::new(false);
        let drops = Rc::new(Cell::new(0));
        let mut capture = WorkerCapture::new(&LIVE);
        capture.install(DropProbe(Rc::clone(&drops)));
        assert!(capture.has_live_session());
        assert!(LIVE.load(Ordering::Acquire));

        capture.close();

        assert_eq!(drops.get(), 1);
        assert!(!capture.has_live_session());
        assert!(!LIVE.load(Ordering::Acquire));
    }

    #[test]
    fn installing_a_replacement_drops_the_previous_session() {
        static LIVE: AtomicBool = AtomicBool::new(false);
        let drops = Rc::new(Cell::new(0));
        let mut capture = WorkerCapture::new(&LIVE);
        capture.install(DropProbe(Rc::clone(&drops)));

        capture.install(DropProbe(Rc::clone(&drops)));

        assert_eq!(drops.get(), 1);
        assert!(capture.has_live_session());
        assert!(LIVE.load(Ordering::Acquire));
    }

    #[test]
    fn terminal_capture_loss_drops_the_session_and_publishes_diagnostics() {
        static LIVE: AtomicBool = AtomicBool::new(false);
        let drops = Rc::new(Cell::new(0));
        let mut capture = WorkerCapture::new(&LIVE);
        capture.install(DropProbe(Rc::clone(&drops)));

        let transition =
            capture.record_capture_result::<()>(Err("the screen capture session ended"));

        assert_eq!(transition, Err("the screen capture session ended"));
        assert_eq!(drops.get(), 1);
        assert!(!LIVE.load(Ordering::Acquire));
        assert_eq!(
            super::take_session_diagnostic(),
            Some("the screen capture session ended")
        );
    }

    fn stream(node_id: u32, position: (i32, i32), size: (i32, i32)) -> PortalStream {
        PortalStream {
            node_id,
            position: Some(position),
            size: Some(size),
        }
    }

    /// The portal reports each stream's monitor position and size. For a borderless-fullscreen
    /// game that rectangle IS the game rect, in the same coordinates the overlay is placed in.
    #[test]
    fn a_stream_rect_is_its_monitor_geometry() {
        let rect = stream_rect(&stream(71, (1920, 0), (1920, 1080)))
            .expect("a stream with position and size has a rect");
        assert_eq!(
            rect,
            WindowRect {
                x: 1920,
                y: 0,
                width: 1920,
                height: 1080
            }
        );
    }

    /// A stream with no geometry is not usable as a rect, and must not be silently treated as
    /// the origin -- that would crop the wrong monitor.
    #[test]
    fn a_stream_without_geometry_has_no_rect() {
        assert!(
            stream_rect(&PortalStream {
                node_id: 71,
                position: None,
                size: Some((1920, 1080)),
            })
            .is_none()
        );
    }

    /// Mutation caught: converting zero to `u32` succeeds, but a zero-area stream cannot contain
    /// pixels and must not enter native candidate preparation.
    #[test]
    fn zero_sized_streams_have_no_rect() {
        for size in [(0, 1080), (1920, 0), (0, 0)] {
            assert!(
                stream_rect(&PortalStream {
                    node_id: 71,
                    position: Some((0, 0)),
                    size: Some(size),
                })
                .is_none(),
                "accepted invalid portal size {size:?}"
            );
        }
    }

    /// Mutation caught: one malformed stream must not hide the valid monitor later in the portal
    /// list. The overflowing endpoint also must not claim a known X11 rect.
    #[test]
    fn mixed_invalid_metadata_keeps_the_valid_monitor_and_cannot_overflow_containment() {
        let streams = [
            stream(10, (0, 0), (0, 1080)),
            stream(11, (0, 0), (1920, 0)),
            stream(12, (0, 0), (-1, 1080)),
            stream(13, (i32::MAX - 10, 0), (20, 1080)),
            stream(14, (1920, 0), (1920, 1080)),
        ];
        let valid = WindowRect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };

        assert_eq!(streams_with_geometry(&streams), vec![(14, valid)]);
        assert_eq!(
            pick_stream(&streams, valid).map(|stream| stream.node_id),
            Some(14)
        );

        let overflow_area = WindowRect {
            x: i32::MAX - 5,
            y: 0,
            width: 1,
            height: 1,
        };
        assert_eq!(
            pick_stream(&streams, overflow_area).map(|stream| stream.node_id),
            Some(14),
            "fallback selected malformed metadata instead of the usable monitor"
        );
    }

    /// When X11 did give us a rect, the stream containing it is the right one -- no guessing.
    #[test]
    fn a_known_rect_picks_the_stream_that_contains_it() {
        let streams = [
            stream(70, (0, 0), (1920, 1080)),
            stream(71, (1920, 0), (1920, 1080)),
        ];
        let game = WindowRect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let picked = pick_stream(&streams, game).expect("the second monitor matches");
        assert_eq!(picked.node_id, 71);
    }

    /// A rect whose origin sits on the first monitor must not pick the second.
    #[test]
    fn a_rect_on_the_first_monitor_picks_the_first_stream() {
        let streams = [
            stream(70, (0, 0), (1920, 1080)),
            stream(71, (1920, 0), (1920, 1080)),
        ];
        let game = WindowRect {
            x: 10,
            y: 10,
            width: 1920,
            height: 1080,
        };
        assert_eq!(pick_stream(&streams, game).unwrap().node_id, 70);
    }

    /// Mutation caught: selecting only `streams.first()` would return node 70 and native Wayland
    /// OCR would never inspect a reward screen shown on node 71.
    #[test]
    fn every_portal_stream_with_valid_geometry_becomes_a_native_candidate() {
        let streams = [
            stream(70, (0, 0), (1920, 1080)),
            PortalStream {
                node_id: 99,
                position: None,
                size: Some((1920, 1080)),
            },
            stream(71, (1920, 0), (1920, 1080)),
        ];

        assert_eq!(
            streams_with_geometry(&streams),
            vec![
                (
                    70,
                    WindowRect {
                        x: 0,
                        y: 0,
                        width: 1920,
                        height: 1080,
                    },
                ),
                (
                    71,
                    WindowRect {
                        x: 1920,
                        y: 0,
                        width: 1920,
                        height: 1080,
                    },
                ),
            ]
        );
    }

    /// Mutation caught: assigning `frame.width` and `frame.height` to `MonitorFrame` treats a
    /// fractional-scale framebuffer as a larger desktop and moves every logical crop.
    #[test]
    fn portal_frames_keep_logical_geometry_while_resampling_physical_pixels() {
        use image::GenericImageView;

        let rect = WindowRect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let physical =
            image::RgbaImage::from_pixel(2880, 1620, image::Rgba([200_u8, 200_u8, 200_u8, 255_u8]));
        let monitor = monitor_frame(physical, rect);

        assert_eq!(
            (
                monitor.origin_x,
                monitor.origin_y,
                monitor.width,
                monitor.height
            ),
            (1920, 0, 1920, 1080),
            "portal metadata, not framebuffer dimensions, defines desktop geometry"
        );
        assert_eq!(monitor.image.dimensions(), (2880, 1620));

        let visible = crate::reward_ocr::visible_region_for(
            rect,
            monitor.origin_x,
            monitor.origin_y,
            monitor.width,
            monitor.height,
        )
        .expect("the logical monitor contains its fullscreen rect");
        let frame = crate::reward_ocr::window_frame_from_monitor_for(
            &monitor.image,
            monitor.width,
            monitor.height,
            rect,
            visible,
        );
        assert_eq!(frame.dimensions(), (1920, 1080));
        assert_eq!(frame.to_rgba8().get_pixel(1900, 1070)[0], 200);
    }

    /// Mutation caught: retaining only one attached node would reopen node 70 after reading node
    /// 71 on every poll. A timeout must evict node 71 alone, leaving node 70 reusable.
    #[test]
    fn node_cache_reuses_each_live_stream_and_evicts_only_the_dead_node() {
        use std::cell::RefCell;
        use std::collections::HashMap;

        let mut cache = HashMap::new();
        let opened = RefCell::new(Vec::new());
        let mut open = |node_id| {
            opened.borrow_mut().push(node_id);
            Ok(node_id)
        };

        assert_eq!(
            read_cached_node(&mut cache, 70, &mut open, |stream| Ok(*stream)),
            Ok(70)
        );
        assert_eq!(
            read_cached_node(&mut cache, 71, &mut open, |stream| Ok(*stream)),
            Ok(71)
        );
        assert_eq!(
            read_cached_node(&mut cache, 70, &mut open, |stream| Ok(*stream)),
            Ok(70)
        );
        assert_eq!(
            *opened.borrow(),
            vec![70, 71],
            "both live nodes should open once"
        );

        assert_eq!(
            read_cached_node(&mut cache, 71, &mut open, |_| Err::<u32, _>(ERR_NO_FRAME)),
            Err(ERR_NO_FRAME)
        );
        assert!(cache.contains_key(&70), "the healthy node was evicted");
        assert!(!cache.contains_key(&71), "the dead node was retained");

        assert_eq!(
            read_cached_node(&mut cache, 71, &mut open, |stream| Ok(*stream)),
            Ok(71)
        );
        assert_eq!(*opened.borrow(), vec![70, 71, 71]);
    }

    /// Mutation caught: flattening every read failure to one message. A torn-down session must
    /// stay distinguishable from a frame that merely did not arrive, because only the former may
    /// clear the live session and re-offer authorization.
    #[test]
    fn node_cache_preserves_a_session_teardown_reason() {
        use std::collections::HashMap;

        let mut cache = HashMap::new();
        let mut open = |node_id| Ok(node_id);

        assert_eq!(
            read_cached_node(&mut cache, 70, &mut open, |stream| Ok(*stream)),
            Ok(70)
        );
        assert_eq!(
            read_cached_node(&mut cache, 70, &mut open, |_| Err::<u32, _>(
                ERR_SESSION_ENDED
            )),
            Err(ERR_SESSION_ENDED),
            "a teardown must not be reported as a missing frame"
        );
        assert!(!cache.contains_key(&70), "the dead node was retained");
    }

    /// The documented degradation, pinned so it stays deliberate rather than incidental: a rect
    /// the portal has no matching monitor for -- an output hotplugged away between the rect query
    /// and the handshake -- still yields a stream, because a probably-wrong monitor the card
    /// reader will reject beats capturing nothing. `pick_stream` logs this case.
    #[test]
    fn a_rect_matching_no_stream_still_yields_the_first_stream() {
        let streams = [stream(70, (0, 0), (1920, 1080))];
        let vanished = WindowRect {
            x: 3840,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert_eq!(pick_stream(&streams, vanished).unwrap().node_id, 70);
    }

    /// Half a geometry is not a monitor. A stream that reports a position but no size cannot be
    /// containment-tested, so it must be skipped rather than treated as a match -- matching on
    /// position alone would send an unbounded monitor every rect to its right.
    #[test]
    fn a_stream_missing_its_size_is_not_a_containment_match() {
        let streams = [
            PortalStream {
                node_id: 70,
                position: Some((0, 0)),
                size: None,
            },
            stream(71, (0, 0), (1920, 1080)),
        ];
        let game = WindowRect {
            x: 10,
            y: 10,
            width: 1920,
            height: 1080,
        };
        // 71, not the sizeless 70 that sits at the same position and comes first.
        assert_eq!(pick_stream(&streams, game).unwrap().node_id, 71);
    }

    /// The portal's size fields are signed. A negative one is not a rectangle, and must not wrap
    /// into a huge `u32` -- that would be a monitor that contains everything.
    #[test]
    fn a_negative_size_has_no_rect() {
        assert!(
            stream_rect(&PortalStream {
                node_id: 71,
                position: Some((0, 0)),
                size: Some((-1920, 1080)),
            })
            .is_none()
        );
    }

    /// The exact boundary, as its own case. A monitor at x=0 of width 1920 spans 0..=1919, so
    /// x=1920 belongs to the next monitor. An inclusive comparison here would claim every rect on
    /// the second monitor for the first and send every capture to the wrong output.
    #[test]
    fn a_rect_on_the_exact_boundary_belongs_to_the_next_monitor() {
        let streams = [
            stream(70, (0, 0), (1920, 1080)),
            stream(71, (1920, 0), (1920, 1080)),
        ];
        let on_boundary = WindowRect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        // The first pixel the second monitor owns, and the last one. Both must resolve to 71,
        // and each catches a different mistake: an inclusive high-side comparison hands
        // `on_boundary` to monitor 70, while an over-strict one matches no monitor at all and
        // `last_pixel` falls back to `streams.first()`, which is also 70.
        //
        // The last-owned-pixel case sits on the SECOND monitor deliberately. On the first it
        // would discriminate nothing: a no-match fallback returns 70 too, so the assertion
        // would pass whether the comparison was right or not.
        let last_pixel = WindowRect {
            x: 3839,
            y: 1079,
            width: 1,
            height: 1,
        };
        assert_eq!(pick_stream(&streams, on_boundary).unwrap().node_id, 71);
        assert_eq!(pick_stream(&streams, last_pixel).unwrap().node_id, 71);
    }

    /// The runtime must be one instance for the life of the process: ashpd caches its D-Bus
    /// connection in a `static`, and a per-call runtime would drop the connection's background
    /// tasks and leave it inert. Pointer equality is the honest part of that testable without a
    /// live portal; Task 6's harness covers the handshake itself.
    #[test]
    fn the_portal_runtime_is_built_once() {
        let first = super::portal_runtime().expect("the portal runtime builds");
        let second = super::portal_runtime().expect("the second call reuses it");
        assert!(
            std::ptr::eq(first, second),
            "portal_runtime handed out two different runtimes"
        );
    }
}
