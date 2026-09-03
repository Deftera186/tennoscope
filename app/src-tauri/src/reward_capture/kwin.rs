//! KWin ScreenShot2 capture with raw FD transport. KWin authorizes the installed desktop entry,
//! captures silently and no desktop chooser or screen-sharing indicator ever appears.
//!
//! The decoding half is deliberately transport-independent: version gating, size arithmetic,
//! stride padding and Qt's pixel formats are the parts that are easy to get wrong, so they are
//! testable against literal byte fixtures rather than against a running desktop.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::os::fd::{AsFd, OwnedFd};
use std::time::{Duration, Instant};

use image::RgbaImage;
use wayland_client::globals::GlobalListContents;
use wayland_client::protocol::{wl_output, wl_registry};
use wayland_client::{
    Connection as WaylandConnection, Dispatch, Proxy as WaylandProxy, QueueHandle, delegate_noop,
};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};
use zbus::zvariant;

use super::MonitorFrame;
use crate::overlay_window::WindowRect;

/// Raw ScreenShot2 replies gained the `scale` metadata field in version 4. The decoder needs that
/// field to turn physical image dimensions into the compositor's logical output coordinates.
pub const MIN_SCREENSHOT2_VERSION: u32 = 4;

/// Bounded under a small number of 400 ms polls so a dead KWin call cannot wedge capture forever.
const CAPTURE_DEADLINE: Duration = Duration::from_secs(2);
const CAPTURE_RETRY_INTERVAL: Duration = Duration::from_secs(5);
const AVAILABILITY_RETRY_INTERVAL: Duration = Duration::from_secs(5);

/// The result type KWin names for the only reply shape this decoder understands.
const RAW_RESULT_TYPE: &str = "raw";

/// Qt's `QImage::Format` values that KWin is known to hand out.
///
/// Named rather than inlined because the numbers are an external ABI: they come from Qt, not
/// from this program, and a silent renumbering must fail loudly instead of decoding garbage.
const QIMAGE_FORMAT_ARGB32: u32 = 5;
const QIMAGE_FORMAT_ARGB32_PREMULTIPLIED: u32 = 6;
const QIMAGE_FORMAT_RGBX8888: u32 = 16;
const QIMAGE_FORMAT_RGBA8888: u32 = 17;
const QIMAGE_FORMAT_RGBA8888_PREMULTIPLIED: u32 = 18;

/// How many bytes one pixel occupies in every format this decoder accepts.
const BYTES_PER_PIXEL: usize = 4;

pub const ERR_UNAVAILABLE: &str = "KWin's screenshot service is not available on this desktop";
const ERR_TOO_OLD: &str =
    "KWin's screenshot service does not report the image information TennoScope needs";
const ERR_TIMEOUT: &str = "KWin took too long to capture the screen";
pub const ERR_RESULT_TYPE: &str = "KWin returned an unsupported screenshot result type";
pub const ERR_REPLY_FIELDS: &str = "KWin returned incomplete screenshot metadata";
pub const ERR_FORMAT: &str = "KWin returned an unsupported screenshot format";
pub const ERR_EMPTY: &str = "KWin returned an empty screenshot";
pub const ERR_STRIDE: &str = "KWin returned a screenshot stride shorter than one row";
pub const ERR_TOO_LARGE: &str = "KWin returned a screenshot too large to address";
pub const ERR_TRUNCATED: &str = "KWin returned a truncated screenshot";
pub const ERR_SCALE: &str = "KWin returned a screenshot scale that does not match its size";

/// A ScreenShot2 interface version this program is willing to call.
///
/// Constructing one is the only way to reach the capture path, so "is this desktop capable?"
/// is answered once, at a named boundary, instead of being re-derived at each call site.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct KwinApiVersion(u32);

impl KwinApiVersion {
    pub const fn get(self) -> u32 {
        self.0
    }
}

/// Decide whether an advertised ScreenShot2 version carries the raw metadata this decoder needs.
///
/// `None` means the service is absent, which is the ordinary case outside KWin. A present older
/// interface is reported separately because its raw reply lacks the `scale` field required to map
/// physical image dimensions into logical output coordinates.
pub fn screenshot2_capability(advertised: Option<u32>) -> Result<KwinApiVersion, &'static str> {
    match advertised {
        None => Err(ERR_UNAVAILABLE),
        Some(version) if version < MIN_SCREENSHOT2_VERSION => Err(ERR_TOO_OLD),
        Some(version) => Ok(KwinApiVersion(version)),
    }
}

/// The pixel layouts KWin hands out, split by how the four bytes must be read.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PixelFormat {
    /// A native-endian `0xAARRGGBB` word. Byte positions differ per architecture, so this is
    /// decoded through `u32::from_ne_bytes` and shifts rather than by indexing bytes.
    Argb32 { premultiplied: bool },
    /// Byte-ordered `[R, G, B, X]`; `X` carries no alpha and is replaced with opaque.
    Rgbx8888,
    /// Byte-ordered `[R, G, B, A]`.
    Rgba8888 { premultiplied: bool },
}

impl PixelFormat {
    fn from_qimage(format: u32) -> Result<Self, &'static str> {
        match format {
            QIMAGE_FORMAT_ARGB32 => Ok(Self::Argb32 {
                premultiplied: false,
            }),
            QIMAGE_FORMAT_ARGB32_PREMULTIPLIED => Ok(Self::Argb32 {
                premultiplied: true,
            }),
            QIMAGE_FORMAT_RGBX8888 => Ok(Self::Rgbx8888),
            QIMAGE_FORMAT_RGBA8888 => Ok(Self::Rgba8888 {
                premultiplied: false,
            }),
            QIMAGE_FORMAT_RGBA8888_PREMULTIPLIED => Ok(Self::Rgba8888 {
                premultiplied: true,
            }),
            _ => Err(ERR_FORMAT),
        }
    }
}

/// The reply header KWin sends alongside the pixel file descriptor.
///
/// Held as the raw wire values, unvalidated: validation is a separate, tested step so that no
/// caller can accidentally allocate against numbers a compositor chose.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenshotMetadata {
    pub result_type: String,
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub scale: f64,
}

/// Metadata that has been proven safe to allocate and index against.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ValidatedMetadata {
    format: PixelFormat,
    width: u32,
    height: u32,
    stride: usize,
    row_bytes: usize,
    total_bytes: usize,
    logical_width: u32,
    logical_height: u32,
}

/// Compute how many bytes one row occupies and how many the whole buffer must contain.
///
/// Split out and taking `usize` so the overflow guards can be exercised at their own boundary.
/// A 64-bit host cannot overflow these from `u32` metadata, but a 32-bit one can, and an
/// unchecked product there would wrap to a small allocation that is then indexed past its end.
fn buffer_extent(
    width: usize,
    height: usize,
    stride: usize,
) -> Result<(usize, usize), &'static str> {
    if width == 0 || height == 0 {
        return Err(ERR_EMPTY);
    }
    let row_bytes = width.checked_mul(BYTES_PER_PIXEL).ok_or(ERR_TOO_LARGE)?;
    if stride < row_bytes {
        return Err(ERR_STRIDE);
    }
    // The last row needs only its pixels: a compositor is free to stop the buffer before the
    // trailing padding, and demanding `stride * height` would reject a valid frame.
    let total_bytes = stride
        .checked_mul(height - 1)
        .and_then(|full_rows| full_rows.checked_add(row_bytes))
        .ok_or(ERR_TOO_LARGE)?;
    Ok((row_bytes, total_bytes))
}

/// Check every number in a reply before a single byte is allocated or read.
///
/// The order matters: shape first, then arithmetic. A zero dimension is a clearer error than
/// the overflow-free "0 bytes" buffer it would otherwise produce, and an unsupported format is
/// worth reporting even when the geometry happens to be sane.
fn validate(meta: &ScreenshotMetadata) -> Result<ValidatedMetadata, &'static str> {
    if meta.result_type != RAW_RESULT_TYPE {
        return Err(ERR_RESULT_TYPE);
    }
    let format = PixelFormat::from_qimage(meta.format)?;
    if meta.width == 0 || meta.height == 0 {
        return Err(ERR_EMPTY);
    }

    let (row_bytes, total_bytes) = buffer_extent(
        meta.width as usize,
        meta.height as usize,
        meta.stride as usize,
    )?;
    let stride = meta.stride as usize;

    let (logical_width, logical_height) = logical_size(meta.width, meta.height, meta.scale)?;

    Ok(ValidatedMetadata {
        format,
        width: meta.width,
        height: meta.height,
        stride,
        row_bytes,
        total_bytes,
        logical_width,
        logical_height,
    })
}

/// Convert a physical pixel size into the compositor's logical size.
///
/// Logical geometry is what window rectangles and crops are expressed in, so a fractional-scale
/// output must report the smaller logical size even though its pixels arrive at full density.
fn logical_size(width: u32, height: u32, scale: f64) -> Result<(u32, u32), &'static str> {
    if !scale.is_finite() || scale <= 0.0 {
        return Err(ERR_SCALE);
    }
    let logical = |physical: u32| -> Result<u32, &'static str> {
        let value = (f64::from(physical) / scale).round();
        if !(1.0..=f64::from(u32::MAX)).contains(&value) {
            return Err(ERR_SCALE);
        }
        Ok(value as u32)
    };
    Ok((logical(width)?, logical(height)?))
}

/// Undo Qt's premultiplication so colour survives a translucent pixel.
///
/// Screenshots are normally opaque, in which case this is the identity; it exists so that a
/// premultiplied format is decoded correctly rather than merely accepted.
#[inline]
fn unpremultiply(channel: u8, alpha: u8) -> u8 {
    match alpha {
        0 => 0,
        255 => channel,
        alpha => {
            let value = (u32::from(channel) * 255 + u32::from(alpha) / 2) / u32::from(alpha);
            value.min(255) as u8
        }
    }
}

/// Read one four-byte pixel into straight RGBA.
#[inline]
fn pixel_to_rgba(format: PixelFormat, bytes: [u8; BYTES_PER_PIXEL]) -> [u8; 4] {
    match format {
        PixelFormat::Argb32 { premultiplied } => {
            // Qt stores ARGB32 as one machine word, so the channels are extracted by shifting
            // the native-endian value rather than by assuming where each byte landed.
            let word = u32::from_ne_bytes(bytes);
            let alpha = (word >> 24) as u8;
            let red = (word >> 16) as u8;
            let green = (word >> 8) as u8;
            let blue = word as u8;
            if premultiplied {
                [
                    unpremultiply(red, alpha),
                    unpremultiply(green, alpha),
                    unpremultiply(blue, alpha),
                    alpha,
                ]
            } else {
                [red, green, blue, alpha]
            }
        }
        PixelFormat::Rgbx8888 => [bytes[0], bytes[1], bytes[2], u8::MAX],
        PixelFormat::Rgba8888 { premultiplied } => {
            let alpha = bytes[3];
            if premultiplied {
                [
                    unpremultiply(bytes[0], alpha),
                    unpremultiply(bytes[1], alpha),
                    unpremultiply(bytes[2], alpha),
                    alpha,
                ]
            } else {
                [bytes[0], bytes[1], bytes[2], alpha]
            }
        }
    }
}

/// Turn a validated reply and its bytes into one RGBA image.
///
/// Rows are converted straight into the destination buffer: no encoded image, no Qt handle and
/// no intermediate full-frame copy, because this runs on every capture poll while a mission is
/// on screen.
fn decode_image(meta: &ValidatedMetadata, bytes: &[u8]) -> Result<RgbaImage, &'static str> {
    if bytes.len() < meta.total_bytes {
        return Err(ERR_TRUNCATED);
    }

    let mut pixels = Vec::with_capacity(meta.row_bytes * meta.height as usize);
    for row in 0..meta.height as usize {
        let start = row * meta.stride;
        let source = &bytes[start..start + meta.row_bytes];
        for pixel in source.chunks_exact(BYTES_PER_PIXEL) {
            let word = [pixel[0], pixel[1], pixel[2], pixel[3]];
            pixels.extend_from_slice(&pixel_to_rgba(meta.format, word));
        }
    }

    RgbaImage::from_raw(meta.width, meta.height, pixels).ok_or(ERR_TOO_LARGE)
}

/// Decode a whole-monitor ScreenShot2 reply into the frame the capture pipeline consumes.
///
/// `origin_x`/`origin_y` are the output's logical position, which travels with the pixels so a
/// monitor at a negative origin still crops correctly. The image keeps its physical density
/// while the frame reports logical geometry -- the same split the portal backend already uses,
/// so downstream resampling needs no new case.
pub fn decode_monitor_frame(
    meta: &ScreenshotMetadata,
    bytes: &[u8],
    origin_x: i32,
    origin_y: i32,
) -> Result<MonitorFrame, &'static str> {
    let validated = validate(meta)?;
    let image = decode_image(&validated, bytes)?;
    Ok(MonitorFrame {
        image,
        origin_x,
        origin_y,
        width: validated.logical_width,
        height: validated.logical_height,
    })
}

/// The logical rectangle an output occupies, without decoding its pixels.
///
/// Used while choosing which monitor holds the game: picking a screen must not cost a full
/// frame conversion.
pub fn logical_rect(
    meta: &ScreenshotMetadata,
    origin_x: i32,
    origin_y: i32,
) -> Result<WindowRect, &'static str> {
    let validated = validate(meta)?;
    Ok(WindowRect {
        x: origin_x,
        y: origin_y,
        width: validated.logical_width,
        height: validated.logical_height,
    })
}

// --- Transport -------------------------------------------------------------------------------

const SCREENSHOT2_SERVICE: &str = "org.kde.KWin.ScreenShot2";
const SCREENSHOT2_PATH: &str = "/org/kde/KWin/ScreenShot2";
const SCREENSHOT2_INTERFACE: &str = "org.kde.KWin.ScreenShot2";
const CAPTURE_SCREEN_METHOD: &str = "CaptureScreen";
const VERSION_PROPERTY: &str = "Version";

/// KWin's own name for "your desktop entry does not claim the restricted interface".
const NOT_AUTHORIZED_ERROR: &str = "org.kde.KWin.ScreenShot2.Error.NoAuthorized";

/// The option keys KWin reads out of the `CaptureScreen` vardict.
const OPTION_INCLUDE_CURSOR: &str = "include-cursor";
const OPTION_NATIVE_RESOLUTION: &str = "native-resolution";
const OPTION_HIDE_CALLER_WINDOWS: &str = "hide-caller-windows";

/// The revision that began hiding the calling program's own windows.
const HIDE_CALLER_WINDOWS_VERSION: u32 = 5;

/// A ceiling on how much a compositor may push down the pipe before we treat it as hostile.
///
/// A 16K frame at 4 bytes per pixel is about 500 MiB; anything past this is not a screenshot,
/// and reading to end-of-file without a bound would let a broken compositor exhaust memory.
const MAX_FRAME_BYTES: u64 = 512 * 1024 * 1024;

pub const ERR_NO_OUTPUTS: &str = "KWin reported no usable monitors";
pub const ERR_NOT_AUTHORIZED: &str =
    "KWin screen capture was not authorized; run the installed TennoScope build";
pub const ERR_CAPTURE_FAILED: &str = "KWin could not capture the screen";
pub const ERR_READER_LOST: &str = "the KWin screenshot reader stopped unexpectedly";

/// One compositor output: the name `CaptureScreen` expects, and where it sits.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KwinOutput {
    pub name: String,
    pub rect: WindowRect,
}

/// An output still being assembled from Wayland events.
///
/// Wayland delivers a monitor's name, position and size as separate events, so an output is
/// only usable once all three have arrived; this holds the partial state until then.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct PendingOutput {
    name: Option<String>,
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
}

/// Turn accumulated Wayland state into outputs worth calling `CaptureScreen` on.
///
/// Incomplete entries are dropped rather than defaulted: an output with no logical size would
/// otherwise become a zero-sized or origin-anchored rectangle, and a guessed rectangle silently
/// crops the wrong region forever. Repeated names collapse to their first occurrence, because a
/// name is what identifies a screen to KWin -- capturing it twice would only duplicate work.
fn finish_outputs(pending: &[PendingOutput]) -> Result<Vec<KwinOutput>, &'static str> {
    let mut outputs: Vec<KwinOutput> = Vec::with_capacity(pending.len());

    for candidate in pending {
        let (Some(name), Some((x, y)), Some((width, height))) =
            (candidate.name.as_ref(), candidate.position, candidate.size)
        else {
            continue;
        };
        if name.is_empty() || width <= 0 || height <= 0 {
            continue;
        }
        if outputs.iter().any(|output| &output.name == name) {
            continue;
        }
        outputs.push(KwinOutput {
            name: name.clone(),
            rect: WindowRect {
                x,
                y,
                // Both are positive here, so the cast cannot wrap.
                width: width as u32,
                height: height as u32,
            },
        });
    }

    if outputs.is_empty() {
        return Err(ERR_NO_OUTPUTS);
    }
    Ok(outputs)
}

/// The options this program sends with every screen capture.
///
/// Expressed as data rather than built inline so the version gating is testable without a
/// compositor. The cursor is excluded because it lands on top of reward cards and confuses OCR;
/// logical resolution is requested so the returned pixels already match the geometry the crop is
/// computed in; caller windows are hidden so this program's own overlay never captures itself.
fn capture_options(version: KwinApiVersion) -> Vec<(&'static str, bool)> {
    let mut options = vec![
        (OPTION_INCLUDE_CURSOR, false),
        (OPTION_NATIVE_RESOLUTION, false),
    ];
    if version.get() >= HIDE_CALLER_WINDOWS_VERSION {
        options.push((OPTION_HIDE_CALLER_WINDOWS, true));
    }
    options
}
/// Drain the pipe while KWin is producing the reply so a full pipe cannot deadlock the method.
/// The reader has its own deadline, so joining it cannot leak or strand one thread per poll.
fn capture_then_read<T>(
    capture: impl FnOnce() -> Result<T, &'static str>,
    read: impl FnOnce() -> Result<Vec<u8>, &'static str> + Send,
) -> Result<(T, Vec<u8>), &'static str> {
    std::thread::scope(|scope| {
        let reader = scope.spawn(read);
        let metadata = capture();
        let bytes = reader.join().map_err(|_| ERR_CAPTURE_FAILED)?;
        match (metadata, bytes) {
            (Err(error), _) => Err(error),
            (_, Err(error)) => Err(error),
            (Ok(metadata), Ok(bytes)) => Ok((metadata, bytes)),
        }
    })
}

/// Read until KWin closes its duplicated writer, without abandoning a blocked reader thread.
///
/// `max_bytes` is a parameter so the ceiling is reachable in tests without allocating the real
/// `MAX_FRAME_BYTES` budget.
fn read_frame_until(
    read_end: OwnedFd,
    deadline: Instant,
    max_bytes: u64,
) -> Result<Vec<u8>, &'static str> {
    use rustix::event::{PollFd, PollFlags, poll};

    rustix::fs::fcntl_setfl(&read_end, rustix::fs::OFlags::NONBLOCK)
        .map_err(|_| ERR_CAPTURE_FAILED)?;
    let mut bytes: Vec<u8> = Vec::new();
    let mut chunk = [0_u8; 64 * 1024];
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ERR_TIMEOUT);
        }
        let timeout =
            rustix::event::Timespec::try_from(remaining).map_err(|_| ERR_CAPTURE_FAILED)?;
        let mut fds = [PollFd::new(&read_end, PollFlags::IN)];
        match poll(&mut fds, Some(&timeout)) {
            Ok(0) => return Err(ERR_TIMEOUT),
            Err(error) if error == rustix::io::Errno::INTR => continue,
            Err(_) => return Err(ERR_CAPTURE_FAILED),
            Ok(_) => {}
        }

        match rustix::io::read(&read_end, &mut chunk) {
            Ok(0) => return Ok(bytes),
            Ok(count) => {
                // Checked against the ceiling before the copy: a compositor that never closes
                // its writer would otherwise grow this buffer until the process is killed.
                if bytes.len() as u64 + count as u64 > max_bytes {
                    return Err(ERR_TOO_LARGE);
                }
                bytes.extend_from_slice(&chunk[..count]);
            }
            Err(error) if error == rustix::io::Errno::AGAIN => continue,
            Err(_) => return Err(ERR_CAPTURE_FAILED),
        }
    }
}

/// Collects `wl_output` and `xdg_output` state for one connection.
#[derive(Default)]
struct OutputDiscovery {
    pending: Vec<PendingOutput>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for OutputDiscovery {
    fn event(
        _state: &mut Self,
        _registry: &wl_registry::WlRegistry,
        _event: wl_registry::Event,
        _data: &GlobalListContents,
        _connection: &WaylandConnection,
        _queue: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<wl_output::WlOutput, usize> for OutputDiscovery {
    fn event(
        state: &mut Self,
        _output: &wl_output::WlOutput,
        event: wl_output::Event,
        index: &usize,
        _connection: &WaylandConnection,
        _queue: &QueueHandle<Self>,
    ) {
        // `wl_output` version 4 carries the connector name, which is what KWin matches on.
        if let wl_output::Event::Name { name } = event
            && let Some(pending) = state.pending.get_mut(*index)
        {
            pending.name = Some(name);
        }
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, usize> for OutputDiscovery {
    fn event(
        state: &mut Self,
        _output: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        index: &usize,
        _connection: &WaylandConnection,
        _queue: &QueueHandle<Self>,
    ) {
        let Some(pending) = state.pending.get_mut(*index) else {
            return;
        };
        match event {
            // Older compositors report the name here instead of on `wl_output`.
            zxdg_output_v1::Event::Name { name } => {
                pending.name.get_or_insert(name);
            }
            zxdg_output_v1::Event::LogicalPosition { x, y } => pending.position = Some((x, y)),
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                pending.size = Some((width, height));
            }
            _ => {}
        }
    }
}

delegate_noop!(OutputDiscovery: ignore zxdg_output_manager_v1::ZxdgOutputManagerV1);

/// Ask the compositor which monitors exist and where they are.
///
/// xdg-output is the only protocol that reports logical position and size, which is the
/// coordinate space window rectangles live in; `wl_output` alone would give physical modes and
/// force this code to reconstruct the layout.
fn discover_outputs(connection: &WaylandConnection) -> Result<Vec<KwinOutput>, &'static str> {
    let (globals, mut queue) =
        wayland_client::globals::registry_queue_init::<OutputDiscovery>(connection)
            .map_err(|_| ERR_NO_OUTPUTS)?;
    let handle = queue.handle();

    let manager: zxdg_output_manager_v1::ZxdgOutputManagerV1 = globals
        .bind(&handle, 2..=3, ())
        .map_err(|_| ERR_NO_OUTPUTS)?;

    let mut state = OutputDiscovery::default();
    let mut outputs = Vec::new();
    globals.contents().with_list(|list| {
        for global in list {
            if global.interface == wl_output::WlOutput::interface().name {
                let index = outputs.len();
                let version = global.version.min(4);
                let output: wl_output::WlOutput =
                    globals
                        .registry()
                        .bind(global.name, version, &handle, index);
                outputs.push(output);
                state.pending.push(PendingOutput::default());
            }
        }
    });

    for (index, output) in outputs.iter().enumerate() {
        manager.get_xdg_output(output, &handle, index);
    }

    // Two round trips: one for the requests above to reach the compositor, one for every
    // name/position/size event they produce to come back.
    queue.roundtrip(&mut state).map_err(|_| ERR_NO_OUTPUTS)?;
    queue.roundtrip(&mut state).map_err(|_| ERR_NO_OUTPUTS)?;

    finish_outputs(&state.pending)
}

/// Translate a D-Bus failure into something a player can act on.
///
/// Timeouts invalidate the session just like other capture failures. Authorization remains the
/// most useful named failure because it tells an installed user which setup state is broken.
fn is_timeout_error(error: &zbus::Error) -> bool {
    matches!(
        error,
        zbus::Error::InputOutput(source) if source.kind() == std::io::ErrorKind::TimedOut
    )
}

fn map_dbus_error(error: &zbus::Error) -> &'static str {
    if is_timeout_error(error) {
        return ERR_TIMEOUT;
    }
    if let zbus::Error::MethodError(name, _, _) = error
        && name.as_str() == NOT_AUTHORIZED_ERROR
    {
        return ERR_NOT_AUTHORIZED;
    }
    ERR_CAPTURE_FAILED
}

/// Pull the reply vardict apart into the metadata the decoder validates.
fn metadata_from_reply(
    reply: &HashMap<String, zvariant::OwnedValue>,
) -> Result<ScreenshotMetadata, &'static str> {
    let field = |key: &str| reply.get(key).ok_or(ERR_REPLY_FIELDS);
    let number = |key: &str| -> Result<u32, &'static str> {
        u32::try_from(field(key)?).map_err(|_| ERR_REPLY_FIELDS)
    };

    let result_type = <&str>::try_from(field("type")?)
        .map_err(|_| ERR_REPLY_FIELDS)?
        .to_owned();
    let scale = f64::try_from(field("scale")?).map_err(|_| ERR_SCALE)?;

    Ok(ScreenshotMetadata {
        result_type,
        format: number("format")?,
        width: number("width")?,
        height: number("height")?,
        stride: number("stride")?,
        scale,
    })
}

/// A live connection to KWin's screenshot service, plus the monitor layout it applies to.
struct KwinSession {
    dbus: zbus::blocking::Connection,
    version: KwinApiVersion,
    outputs: Vec<KwinOutput>,
}

#[derive(Clone, Copy)]
struct AvailabilityState {
    available: bool,
    checked_at: Instant,
}

impl AvailabilityState {
    fn new(available: bool, checked_at: Instant) -> Self {
        Self {
            available,
            checked_at,
        }
    }

    fn should_probe(self, now: Instant) -> bool {
        !self.available
            && now.saturating_duration_since(self.checked_at) >= AVAILABILITY_RETRY_INTERVAL
    }

    fn record(&mut self, available: bool, checked_at: Instant) {
        self.available = available;
        self.checked_at = checked_at;
    }
}

fn kwin_authorizable(appimage: Option<&OsStr>) -> bool {
    appimage.is_none()
}

fn probe_available() -> bool {
    KwinSession::establish().is_ok()
}

fn collect_generation<T>(
    results: impl IntoIterator<Item = Result<T, &'static str>>,
) -> Result<Vec<T>, &'static str> {
    let mut values = Vec::new();
    let mut failure = None;
    for result in results {
        match result {
            Ok(value) if failure.is_none() => values.push(value),
            Ok(_) => {}
            Err(error) => failure = Some(rank_failure(failure, error)),
        }
    }
    failure.map_or(Ok(values), Err)
}

/// Whether KWin offers a compatible ScreenShot2 service and advertises at least one output.
///
/// Positive availability is cached permanently. Negative results are retried at a bounded interval
/// because the D-Bus name and compositor may appear after startup or return after a restart.
/// AppImage runtimes are unavailable without probing: KDE compares the desktop entry `Exec` with
/// the ephemeral AppRun executable path, so it can never authorize the capture call.
pub fn available_cached() -> bool {
    if !kwin_authorizable(std::env::var_os("APPIMAGE").as_deref()) {
        return false;
    }
    static AVAILABLE: std::sync::LazyLock<std::sync::Mutex<AvailabilityState>> =
        std::sync::LazyLock::new(|| {
            let now = Instant::now();
            std::sync::Mutex::new(AvailabilityState::new(probe_available(), now))
        });
    let now = Instant::now();
    let mut state = AVAILABLE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if state.should_probe(now) {
        state.record(probe_available(), now);
    }
    state.available
}

pub struct KwinCapture {
    session: Option<KwinSession>,
    authorization_available: bool,
    retry_after: Option<Instant>,
}

impl KwinSession {
    fn establish() -> Result<Self, &'static str> {
        if !kwin_authorizable(std::env::var_os("APPIMAGE").as_deref()) {
            return Err(ERR_UNAVAILABLE);
        }
        let dbus = zbus::blocking::connection::Builder::session()
            .map_err(|_| ERR_UNAVAILABLE)?
            .method_timeout(CAPTURE_DEADLINE)
            .build()
            .map_err(|_| ERR_UNAVAILABLE)?;
        let version = screenshot2_version(&dbus)?;
        let wayland = WaylandConnection::connect_to_env().map_err(|_| ERR_NO_OUTPUTS)?;
        let outputs = discover_outputs(&wayland)?;
        Ok(Self {
            dbus,
            version,
            outputs,
        })
    }

    fn capture_generation(&self) -> Result<Vec<(WindowRect, MonitorFrame)>, &'static str> {
        collect_generation(self.outputs.iter().map(|output| {
            capture_output(&self.dbus, self.version, output).map(|frame| (output.rect, frame))
        }))
    }
}

impl Default for KwinCapture {
    fn default() -> Self {
        Self::new()
    }
}

impl KwinCapture {
    pub const fn new() -> Self {
        Self {
            session: None,
            authorization_available: true,
            retry_after: None,
        }
    }

    /// Cheap capability check for settings diagnostics. Negative results are re-probed after the
    /// bounded interval rather than being latched for the process lifetime. An authorization
    /// refusal disables this instance so its owner can select the portal on the next poll.
    pub fn available(&self) -> bool {
        self.authorization_available
            && kwin_retry_ready(self.retry_after, Instant::now())
            && available_cached()
    }

    fn capture_once(&mut self) -> Result<Vec<(WindowRect, MonitorFrame)>, &'static str> {
        if self.session.is_none() {
            self.session = Some(KwinSession::establish()?);
        }
        self.session
            .as_ref()
            .ok_or(ERR_UNAVAILABLE)?
            .capture_generation()
    }

    fn capture_failure_is_retryable(error: &'static str) -> bool {
        matches!(error, ERR_UNAVAILABLE | ERR_NO_OUTPUTS | ERR_CAPTURE_FAILED)
    }

    fn record_terminal_failure(&mut self, error: &'static str) {
        if error == ERR_NOT_AUTHORIZED {
            self.authorization_available = false;
        }
    }

    pub fn capture_monitors(&mut self) -> Result<Vec<(WindowRect, MonitorFrame)>, &'static str> {
        let first = self.capture_once();
        let Err(error) = first else {
            self.retry_after = None;
            return first;
        };

        self.session = None;
        if !Self::capture_failure_is_retryable(error) {
            self.record_terminal_failure(error);
            return Err(error);
        }
        let retry = self.capture_once();
        if retry.is_err() {
            self.session = None;
            self.retry_after = Some(Instant::now() + CAPTURE_RETRY_INTERVAL);
        } else {
            self.retry_after = None;
        }
        retry
    }
}

fn kwin_retry_ready(retry_after: Option<Instant>, now: Instant) -> bool {
    retry_after.is_none_or(|deadline| now >= deadline)
}

/// Keep authorization failures above generic failures when reporting a failed generation.
/// This lets a refusal survive an incidental failure reported by another monitor afterwards;
/// otherwise the one message that names a fixable cause gets overwritten by a generic one.
fn rank_failure(current: Option<&'static str>, incoming: &'static str) -> &'static str {
    if current == Some(ERR_NOT_AUTHORIZED) || incoming == ERR_NOT_AUTHORIZED {
        return ERR_NOT_AUTHORIZED;
    }
    incoming
}

/// Read the advertised interface version and decide whether it is usable.
fn screenshot2_version(dbus: &zbus::blocking::Connection) -> Result<KwinApiVersion, &'static str> {
    let proxy = zbus::blocking::Proxy::new(
        dbus,
        SCREENSHOT2_SERVICE,
        SCREENSHOT2_PATH,
        SCREENSHOT2_INTERFACE,
    )
    .map_err(|_| ERR_UNAVAILABLE)?;
    let advertised = proxy
        .get_property::<u32>(VERSION_PROPERTY)
        .map_err(|error| {
            if is_timeout_error(&error) {
                ERR_TIMEOUT
            } else {
                ERR_UNAVAILABLE
            }
        })?;
    screenshot2_capability(Some(advertised))
}

/// Capture one named output into a frame.
fn capture_output(
    dbus: &zbus::blocking::Connection,
    version: KwinApiVersion,
    output: &KwinOutput,
) -> Result<MonitorFrame, &'static str> {
    let (read_end, write_end) = rustix::pipe::pipe_with(rustix::pipe::PipeFlags::CLOEXEC)
        .map_err(|_| ERR_CAPTURE_FAILED)?;
    let deadline = Instant::now() + CAPTURE_DEADLINE;

    let proxy = zbus::blocking::Proxy::new(
        dbus,
        SCREENSHOT2_SERVICE,
        SCREENSHOT2_PATH,
        SCREENSHOT2_INTERFACE,
    )
    .map_err(|_| ERR_UNAVAILABLE)?;

    let (metadata, bytes) = capture_then_read(
        move || {
            let options: HashMap<&str, zvariant::Value<'_>> = capture_options(version)
                .into_iter()
                .map(|(key, value)| (key, zvariant::Value::Bool(value)))
                .collect();
            let reply: HashMap<String, zvariant::OwnedValue> = proxy
                .call(
                    CAPTURE_SCREEN_METHOD,
                    &(
                        output.name.as_str(),
                        options,
                        zvariant::Fd::from(write_end.as_fd()),
                    ),
                )
                .map_err(|error| map_dbus_error(&error))?;
            drop(write_end);
            metadata_from_reply(&reply)
        },
        || read_frame_until(read_end, deadline, MAX_FRAME_BYTES),
    )?;

    decode_monitor_frame(&metadata, &bytes, output.rect.x, output.rect.y)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw_meta(format: u32, width: u32, height: u32, stride: u32) -> ScreenshotMetadata {
        ScreenshotMetadata {
            result_type: RAW_RESULT_TYPE.to_owned(),
            format,
            width,
            height,
            stride,
            scale: 1.0,
        }
    }

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
    fn only_session_failures_are_retried_in_the_same_poll() {
        assert!(KwinCapture::capture_failure_is_retryable(ERR_UNAVAILABLE));
        assert!(KwinCapture::capture_failure_is_retryable(ERR_NO_OUTPUTS));
        assert!(KwinCapture::capture_failure_is_retryable(
            ERR_CAPTURE_FAILED
        ));
        assert!(!KwinCapture::capture_failure_is_retryable(
            ERR_NOT_AUTHORIZED
        ));
        assert!(!KwinCapture::capture_failure_is_retryable(ERR_TIMEOUT));
        assert!(!KwinCapture::capture_failure_is_retryable(ERR_TOO_LARGE));
    }

    #[test]
    fn failed_capture_temporarily_yields_to_portal_fallback() {
        let now = std::time::Instant::now();
        let retry_after = Some(now + CAPTURE_RETRY_INTERVAL);

        assert!(!kwin_retry_ready(retry_after, now));
        assert!(!kwin_retry_ready(
            retry_after,
            now + CAPTURE_RETRY_INTERVAL / 2
        ));
        assert!(kwin_retry_ready(retry_after, now + CAPTURE_RETRY_INTERVAL));
    }

    #[test]
    fn authorization_refusal_makes_portal_fallback_eligible_on_the_next_poll() {
        let mut capture = KwinCapture::new();

        capture.record_terminal_failure(ERR_NOT_AUTHORIZED);

        assert!(!capture.authorization_available);
    }

    #[test]
    fn one_failed_output_makes_the_generation_fail_closed() {
        let result = collect_generation([Ok(1_u8), Err(ERR_CAPTURE_FAILED)]);
        assert_eq!(result, Err(ERR_CAPTURE_FAILED));
    }

    #[test]
    fn appimage_runtime_is_not_eligible_for_kwin_capture() {
        assert!(kwin_authorizable(None));
        assert!(!kwin_authorizable(Some(std::ffi::OsStr::new(
            "/tmp/.mount_tenno/AppRun.wrapped"
        ))));
    }

    #[test]
    fn old_service_error_names_missing_image_information() {
        assert_eq!(
            ERR_TOO_OLD,
            "KWin's screenshot service does not report the image information TennoScope needs"
        );
    }

    #[test]
    fn pipe_without_data_reaches_the_timeout_error() {
        let (read_end, _write_end) = rustix::pipe::pipe_with(
            rustix::pipe::PipeFlags::CLOEXEC | rustix::pipe::PipeFlags::NONBLOCK,
        )
        .expect("pipe");
        assert_eq!(
            read_frame_until(read_end, std::time::Instant::now(), MAX_FRAME_BYTES),
            Err(ERR_TIMEOUT)
        );
    }

    /// A compositor that streams without closing its writer must be cut off rather than allowed
    /// to grow the frame buffer until the process is killed.
    #[test]
    fn a_frame_past_the_ceiling_is_refused() {
        let (read_end, write_end) = rustix::pipe::pipe_with(
            rustix::pipe::PipeFlags::CLOEXEC | rustix::pipe::PipeFlags::NONBLOCK,
        )
        .expect("pipe");
        rustix::io::write(&write_end, &[0_u8; 512]).expect("write");
        drop(write_end);

        assert_eq!(
            read_frame_until(
                read_end,
                std::time::Instant::now() + std::time::Duration::from_secs(5),
                256,
            ),
            Err(ERR_TOO_LARGE)
        );
    }

    #[test]
    fn dbus_deadline_reaches_the_timeout_error() {
        let timeout = zbus::Error::from(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "timed out",
        ));
        assert_eq!(map_dbus_error(&timeout), ERR_TIMEOUT);
    }

    #[test]
    fn a_zero_height_is_rejected_at_the_arithmetic_boundary() {
        assert_eq!(buffer_extent(1, 0, 4), Err(ERR_EMPTY));
    }

    #[test]
    fn malformed_reply_fields_are_not_reported_as_result_types() {
        let reply = HashMap::new();
        assert_eq!(metadata_from_reply(&reply), Err(ERR_REPLY_FIELDS));
    }

    /// Catches a gate that accepts any advertised version, or that treats an absent service as
    /// merely outdated.
    #[test]
    fn capability_accepts_supported_versions_and_names_each_refusal() {
        assert_eq!(
            screenshot2_capability(Some(4)).map(KwinApiVersion::get),
            Ok(4)
        );
        assert_eq!(
            screenshot2_capability(Some(5)).map(KwinApiVersion::get),
            Ok(5)
        );
        assert_eq!(screenshot2_capability(None), Err(ERR_UNAVAILABLE));
        for version in 0..MIN_SCREENSHOT2_VERSION {
            assert_eq!(screenshot2_capability(Some(version)), Err(ERR_TOO_OLD));
        }
    }

    /// Catches an `X` byte copied through as alpha, which yields fully transparent pixels when
    /// KWin sends its current default format.
    #[test]
    fn rgbx8888_forces_opaque_alpha() {
        let bytes = [10, 20, 30, 0, 40, 50, 60, 7];
        let meta = raw_meta(QIMAGE_FORMAT_RGBX8888, 2, 1, 8);
        let frame = decode_monitor_frame(&meta, &bytes, 0, 0).expect("decodes");
        assert_eq!(frame.image.get_pixel(0, 0).0, [10, 20, 30, 255]);
        assert_eq!(frame.image.get_pixel(1, 0).0, [40, 50, 60, 255]);
    }

    /// Catches alpha being overwritten with opaque for the format that actually carries it.
    #[test]
    fn rgba8888_preserves_alpha() {
        let bytes = [10, 20, 30, 128, 40, 50, 60, 255];
        let meta = raw_meta(QIMAGE_FORMAT_RGBA8888, 2, 1, 8);
        let frame = decode_monitor_frame(&meta, &bytes, 0, 0).expect("decodes");
        assert_eq!(frame.image.get_pixel(0, 0).0, [10, 20, 30, 128]);
        assert_eq!(frame.image.get_pixel(1, 0).0, [40, 50, 60, 255]);
    }

    /// Catches ARGB32 being read as if its bytes were laid out `[A, R, G, B]` in memory. The
    /// fixture is built from the machine word Qt actually stores, so the test states the same
    /// endianness contract the decoder implements instead of hard-coding little-endian bytes.
    #[test]
    fn argb32_is_decoded_as_a_native_endian_word() {
        let opaque = 0xFF_11_22_33_u32.to_ne_bytes();
        let bytes: Vec<u8> = opaque.to_vec();
        let meta = raw_meta(QIMAGE_FORMAT_ARGB32, 1, 1, 4);
        let frame = decode_monitor_frame(&meta, &bytes, 0, 0).expect("decodes");
        assert_eq!(frame.image.get_pixel(0, 0).0, [0x11, 0x22, 0x33, 0xFF]);
    }

    /// Catches a premultiplied frame being passed through unchanged, which darkens every
    /// translucent pixel, and catches a divide-by-zero on a fully transparent one.
    #[test]
    fn premultiplied_formats_are_restored_to_straight_alpha() {
        let half = 0x80_40_40_40_u32.to_ne_bytes();
        let meta = raw_meta(QIMAGE_FORMAT_ARGB32_PREMULTIPLIED, 1, 1, 4);
        let frame = decode_monitor_frame(&meta, &half, 0, 0).expect("decodes");
        let pixel = frame.image.get_pixel(0, 0).0;
        assert_eq!(pixel[3], 0x80);
        assert!(
            pixel[0] > 0x7E && pixel[0] < 0x82,
            "unpremultiplied {pixel:?}"
        );

        let clear = [0, 0, 0, 0];
        let meta = raw_meta(QIMAGE_FORMAT_RGBA8888_PREMULTIPLIED, 1, 1, 4);
        let frame = decode_monitor_frame(&meta, &clear, 0, 0).expect("decodes");
        assert_eq!(frame.image.get_pixel(0, 0).0, [0, 0, 0, 0]);
    }

    /// Catches a decoder that walks the buffer by row width instead of by stride, which shears
    /// the image on every padded buffer KWin produces.
    #[test]
    fn padded_stride_is_skipped_rather_than_decoded() {
        let bytes = [
            1, 1, 1, 255, 2, 2, 2, 255, 0xDE, 0xAD, 0xBE, 0xEF, // row 0 + padding
            3, 3, 3, 255, 4, 4, 4, 255, 0xDE, 0xAD, 0xBE, 0xEF, // row 1 + padding
        ];
        let meta = raw_meta(QIMAGE_FORMAT_RGBA8888, 2, 2, 12);
        let frame = decode_monitor_frame(&meta, &bytes, 0, 0).expect("decodes");
        assert_eq!(frame.image.dimensions(), (2, 2));
        assert_eq!(frame.image.get_pixel(0, 0).0, [1, 1, 1, 255]);
        assert_eq!(frame.image.get_pixel(1, 0).0, [2, 2, 2, 255]);
        assert_eq!(frame.image.get_pixel(0, 1).0, [3, 3, 3, 255]);
        assert_eq!(frame.image.get_pixel(1, 1).0, [4, 4, 4, 255]);
    }

    /// Catches a decoder that requires trailing padding after the final row and so rejects a
    /// frame that is actually complete.
    #[test]
    fn a_buffer_without_trailing_padding_is_complete() {
        let bytes = [
            1, 1, 1, 255, 0xDE, 0xAD, 0xBE, 0xEF, // row 0 + padding
            2, 2, 2, 255, // row 1, padding omitted
        ];
        let meta = raw_meta(QIMAGE_FORMAT_RGBA8888, 1, 2, 8);
        let frame = decode_monitor_frame(&meta, &bytes, 0, 0).expect("decodes");
        assert_eq!(frame.image.get_pixel(0, 1).0, [2, 2, 2, 255]);
    }

    /// Catches an unsupported Qt image layout being collapsed into a generic capture failure.
    #[test]
    fn an_unsupported_format_keeps_its_actionable_message() {
        let error = decode_monitor_frame(&raw_meta(9999, 1, 1, 4), &[0; 4], 0, 0)
            .err()
            .expect("the decoder must refuse unknown Qt formats");
        assert_eq!(error, "KWin returned an unsupported screenshot format");
    }

    /// Catches every malformed reply being allowed to reach allocation or indexing. Each arm
    /// names a distinct compositor defect, and each must keep its own message.
    #[test]
    fn malformed_replies_are_refused_before_any_image_is_built() {
        let cases: [(ScreenshotMetadata, Vec<u8>, &str); 8] = [
            (
                ScreenshotMetadata {
                    result_type: "dmabuf".to_owned(),
                    ..raw_meta(QIMAGE_FORMAT_RGBA8888, 1, 1, 4)
                },
                vec![0; 4],
                ERR_RESULT_TYPE,
            ),
            (raw_meta(9999, 1, 1, 4), vec![0; 4], ERR_FORMAT),
            (
                raw_meta(QIMAGE_FORMAT_RGBA8888, 0, 4, 4),
                vec![0; 16],
                ERR_EMPTY,
            ),
            (
                raw_meta(QIMAGE_FORMAT_RGBA8888, 4, 0, 16),
                vec![0; 16],
                ERR_EMPTY,
            ),
            (
                raw_meta(QIMAGE_FORMAT_RGBA8888, 4, 1, 12),
                vec![0; 16],
                ERR_STRIDE,
            ),
            (
                raw_meta(QIMAGE_FORMAT_RGBA8888, u32::MAX, 2, u32::MAX),
                Vec::new(),
                ERR_STRIDE,
            ),
            (
                raw_meta(QIMAGE_FORMAT_RGBA8888, 2, 2, 8),
                vec![0; 12],
                ERR_TRUNCATED,
            ),
            (
                ScreenshotMetadata {
                    scale: 0.0,
                    ..raw_meta(QIMAGE_FORMAT_RGBA8888, 2, 2, 8)
                },
                vec![0; 16],
                ERR_SCALE,
            ),
        ];

        for (meta, bytes, expected) in cases {
            assert_eq!(
                decode_monitor_frame(&meta, &bytes, 0, 0).err(),
                Some(expected),
                "metadata {meta:?} must be refused"
            );
        }
    }

    /// Catches size arithmetic that overflows and wraps to a small allocation, which would
    /// then be indexed far past its end. Driven through the helper rather than through
    /// metadata because `u32` fields cannot reach the guard on a 64-bit host, while a 32-bit
    /// build reaches it from an ordinary 4K frame.
    #[test]
    fn size_overflow_is_refused_rather_than_wrapped() {
        assert_eq!(buffer_extent(usize::MAX, 1, usize::MAX), Err(ERR_TOO_LARGE));
        assert_eq!(
            buffer_extent(4, usize::MAX, usize::MAX / 2),
            Err(ERR_TOO_LARGE)
        );
        assert_eq!(buffer_extent(2, 2, 8), Ok((8, 16)));
    }

    /// Catches physical pixel dimensions leaking into logical geometry, and catches an origin
    /// that is clamped to zero instead of travelling with the frame.
    #[test]
    fn fractional_scale_reports_logical_geometry_at_a_negative_origin() {
        let meta = ScreenshotMetadata {
            scale: 1.25,
            ..raw_meta(QIMAGE_FORMAT_RGBA8888, 10, 5, 40)
        };
        let bytes = vec![7; 200];

        let frame = decode_monitor_frame(&meta, &bytes, -10, -5).expect("decodes");
        assert_eq!(frame.image.dimensions(), (10, 5));
        assert_eq!((frame.width, frame.height), (8, 4));
        assert_eq!((frame.origin_x, frame.origin_y), (-10, -5));

        let rect = logical_rect(&meta, -10, -5).expect("rect");
        assert_eq!(
            rect,
            WindowRect {
                x: -10,
                y: -5,
                width: 8,
                height: 4,
            }
        );
    }

    fn pending(
        name: Option<&str>,
        position: Option<(i32, i32)>,
        size: Option<(i32, i32)>,
    ) -> PendingOutput {
        PendingOutput {
            name: name.map(str::to_owned),
            position,
            size,
        }
    }

    /// Catches logical geometry being reordered, renamed or clamped to a non-negative origin,
    /// which would crop the wrong region on a monitor left of the primary display.
    #[test]
    fn outputs_keep_their_names_and_logical_rectangles() {
        let outputs = finish_outputs(&[
            pending(Some("DP-2"), Some((-1920, 120)), Some((1920, 1080))),
            pending(Some("DP-1"), Some((0, 0)), Some((2560, 1440))),
        ])
        .expect("both outputs are complete");

        assert_eq!(
            outputs,
            vec![
                KwinOutput {
                    name: "DP-2".to_owned(),
                    rect: WindowRect {
                        x: -1920,
                        y: 120,
                        width: 1920,
                        height: 1080,
                    },
                },
                KwinOutput {
                    name: "DP-1".to_owned(),
                    rect: WindowRect {
                        x: 0,
                        y: 0,
                        width: 2560,
                        height: 1440,
                    },
                },
            ]
        );
    }

    /// Catches a missing name, position or size being defaulted into a guessed rectangle, and
    /// catches a repeated output being captured twice.
    #[test]
    fn incomplete_and_duplicate_outputs_never_become_geometry() {
        let outputs = finish_outputs(&[
            pending(None, Some((0, 0)), Some((1920, 1080))),
            pending(Some("DP-1"), None, Some((1920, 1080))),
            pending(Some("DP-1"), Some((0, 0)), None),
            pending(Some(""), Some((0, 0)), Some((1920, 1080))),
            pending(Some("DP-3"), Some((0, 0)), Some((0, 1080))),
            pending(Some("DP-1"), Some((0, 0)), Some((1920, 1080))),
            pending(Some("DP-1"), Some((500, 500)), Some((640, 480))),
        ])
        .expect("one output is complete");

        assert_eq!(outputs.len(), 1);
        assert_eq!(outputs[0].name, "DP-1");
        assert_eq!(outputs[0].rect.width, 1920);
        assert_eq!(outputs[0].rect.x, 0);

        assert_eq!(finish_outputs(&[]), Err(ERR_NO_OUTPUTS));
        assert_eq!(
            finish_outputs(&[pending(Some("DP-1"), None, None)]),
            Err(ERR_NO_OUTPUTS)
        );
    }

    /// Catches the cursor being burned into a reward screen, native pixels being requested when
    /// the crop is computed in logical coordinates, and the caller-window option being sent to a
    /// version that predates it.
    #[test]
    fn capture_options_follow_the_interface_version() {
        let four = capture_options(screenshot2_capability(Some(4)).expect("supported"));
        assert_eq!(
            four,
            vec![
                (OPTION_INCLUDE_CURSOR, false),
                (OPTION_NATIVE_RESOLUTION, false)
            ]
        );

        let five = capture_options(screenshot2_capability(Some(5)).expect("supported"));
        assert_eq!(
            five,
            vec![
                (OPTION_INCLUDE_CURSOR, false),
                (OPTION_NATIVE_RESOLUTION, false),
                (OPTION_HIDE_CALLER_WINDOWS, true),
            ]
        );
    }

    /// Catches the reply being awaited before the pipe is drained. The capture half cannot
    /// finish until the reader signals it has started, so an implementation that reads only
    /// after the call returns fails here instead of deadlocking on a real desktop.
    #[test]
    fn the_reader_starts_before_the_reply_is_awaited() {
        let (started_tx, started_rx) = std::sync::mpsc::channel::<()>();

        let captured = capture_then_read(
            || {
                started_rx
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .map_err(|_| "the reader never started before the call")?;
                Ok(raw_meta(QIMAGE_FORMAT_RGBA8888, 1, 1, 4))
            },
            move || {
                started_tx.send(()).map_err(|_| "the caller went away")?;
                Ok(vec![1, 2, 3, 255])
            },
        )
        .expect("the reader and the call must overlap");

        assert_eq!(captured.0.width, 1);
        assert_eq!(captured.1, vec![1, 2, 3, 255]);
    }

    /// Catches a failed call being reported as a read failure, which would hide the reason.
    #[test]
    fn a_failed_call_outranks_the_read_it_cut_short() {
        let outcome = capture_then_read(
            || Err::<(), &'static str>(ERR_NOT_AUTHORIZED),
            || Err::<Vec<u8>, &'static str>(ERR_CAPTURE_FAILED),
        );
        assert_eq!(outcome.err(), Some(ERR_NOT_AUTHORIZED));
    }

    /// Catches an authorization refusal being buried under an incidental error reported by a
    /// later screen, which would tell the player the desktop is broken instead of that the
    /// build must be installed.
    #[test]
    fn an_authorization_refusal_outranks_later_output_failures() {
        let mut failure = None;
        for error in [ERR_CAPTURE_FAILED, ERR_NOT_AUTHORIZED, ERR_CAPTURE_FAILED] {
            failure = Some(rank_failure(failure, error));
        }
        assert_eq!(failure, Some(ERR_NOT_AUTHORIZED));

        // Without a refusal in play the most recent failure is the one reported.
        assert_eq!(rank_failure(None, ERR_CAPTURE_FAILED), ERR_CAPTURE_FAILED);
        assert_eq!(
            rank_failure(Some(ERR_CAPTURE_FAILED), ERR_NO_OUTPUTS),
            ERR_NO_OUTPUTS
        );
    }

    /// Catches a D-Bus refusal losing its identity, which would tell an unauthorized build that
    /// the desktop is broken instead of that it must be installed.
    #[test]
    fn an_authorization_refusal_names_the_installed_build() {
        let refusal = zbus::Error::MethodError(
            NOT_AUTHORIZED_ERROR.try_into().expect("valid error name"),
            Some("The process is not authorized to take a screenshot".to_owned()),
            zbus::message::Message::method_call("/org/kde/KWin/ScreenShot2", "CaptureScreen")
                .expect("builder")
                .destination(SCREENSHOT2_SERVICE)
                .expect("destination")
                .build(&())
                .expect("message"),
        );
        assert_eq!(
            map_dbus_error(&refusal),
            "KWin screen capture was not authorized; run the installed TennoScope build"
        );

        let other = zbus::Error::MethodError(
            "org.kde.KWin.ScreenShot2.Error.InvalidScreen"
                .try_into()
                .expect("valid error name"),
            None,
            zbus::message::Message::method_call("/org/kde/KWin/ScreenShot2", "CaptureScreen")
                .expect("builder")
                .destination(SCREENSHOT2_SERVICE)
                .expect("destination")
                .build(&())
                .expect("message"),
        );
        assert_eq!(map_dbus_error(&other), ERR_CAPTURE_FAILED);
    }
}
