//! The X11 capture path: xcap over `_NET_CLIENT_LIST_STACKING`, with an `xwininfo` tree walk
//! behind it for Wine's virtual-desktop mode.
//!
//! Unchanged in behaviour from when this lived in `reward_ocr` -- it is the path that works today
//! for an X11 session and for an XWayland game on a Wayland session.

#[cfg(target_os = "linux")]
use std::process::Command;
#[cfg(target_os = "linux")]
use std::time::{Duration, Instant};
#[cfg(target_os = "linux")]
use xcb::{
    Connection, XidNew,
    x::{Drawable, GetImage, GetWindowAttributes, ImageFormat, ImageOrder, Window},
};

use super::{GameFrameSource, GameRectSource, MonitorFrame};
use crate::overlay_window::WindowRect;

/// The game's window title. Warframe titles its window the same on every platform and under every
/// launcher, which its window *class* does not do -- that is `steam_app_230410` under Steam and
/// `warframe.x64.exe` under bare Wine.
pub const WINDOW_TITLE: &str = "Warframe";

/// Failed `xwininfo` tree walks are not useful often enough to justify spawning a process on every
/// 400 ms capture poll. A short cooldown keeps recovery responsive without creating a hot loop.
#[cfg(target_os = "linux")]
const XWININFO_RETRY_INTERVAL: Duration = Duration::from_secs(5);

#[cfg(target_os = "linux")]
static XWININFO_RETRY_AFTER: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);

/// The X11-backed rect and frame source.
pub struct X11Capture {
    selected_window: Option<u32>,
}

impl X11Capture {
    pub fn new() -> Self {
        Self {
            selected_window: None,
        }
    }
}

impl Default for X11Capture {
    fn default() -> Self {
        Self::new()
    }
}

impl X11Capture {
    /// Capture the X11 game drawable itself, including an XWayland child window nested inside a
    /// Wine virtual desktop. This is raw X11 `GetImage`; no monitor API and no portal are involved.
    #[cfg(target_os = "linux")]
    pub fn capture_window(&mut self, rect: WindowRect) -> Result<MonitorFrame, &'static str> {
        let window = self.selected_window.ok_or("no Warframe window found")?;
        let image = capture_x11_window(window, rect.width, rect.height)?;
        Ok(MonitorFrame {
            image,
            origin_x: rect.x,
            origin_y: rect.y,
            width: rect.width,
            height: rect.height,
        })
    }
}

impl GameRectSource for X11Capture {
    fn game_rect(&mut self) -> Result<WindowRect, &'static str> {
        let windows = xcap::Window::all().map_err(|_| "could not enumerate windows")?;
        let found = largest_warframe_window_with_id(windows.iter().filter_map(|window| {
            Some((
                window.title().ok()?,
                window.id().ok()?,
                WindowRect {
                    x: window.x().ok()?,
                    y: window.y().ok()?,
                    width: window.width().ok()?,
                    height: window.height().ok()?,
                },
            ))
        }));
        if let Some((id, rect)) = found {
            self.selected_window = Some(id);
            return Ok(rect);
        }
        // Wine virtual-desktop mode nests the game below a top-level desktop window, so xcap's
        // EWMH list cannot see it. The tree walk supplies both its absolute rect and child XID.
        #[cfg(target_os = "linux")]
        {
            let now = Instant::now();
            if shared_xwininfo_retry_ready(&XWININFO_RETRY_AFTER, now) {
                match xwininfo_tree() {
                    Ok(tree) => {
                        if let Some((id, rect)) = warframe_window_from_xwininfo_tree(&tree) {
                            self.selected_window = parse_xid(&id);
                            if self.selected_window.is_some() {
                                *XWININFO_RETRY_AFTER
                                    .lock()
                                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
                                return Ok(rect);
                            }
                        }
                    }
                    Err(error) => {
                        self.selected_window = None;
                        return Err(error);
                    }
                }
            }
        }
        self.selected_window = None;
        Err("no Warframe window found")
    }
}

impl GameFrameSource for X11Capture {
    fn capture_monitor(&mut self, rect: WindowRect) -> Result<MonitorFrame, &'static str> {
        let monitor = xcap::Monitor::from_point(rect.x, rect.y)
            .map_err(|_| "the game window is not on any monitor")?;
        let origin_x = monitor.x().map_err(|_| "could not read the monitor")?;
        let origin_y = monitor.y().map_err(|_| "could not read the monitor")?;
        let width = monitor.width().map_err(|_| "could not read the monitor")?;
        let height = monitor.height().map_err(|_| "could not read the monitor")?;
        // The whole monitor, then cropped by the caller: xcap's `capture_region` ignores which
        // monitor it was asked for (measured on a two-output desktop -- both outputs returned
        // the first one's pixels), and `Window::capture_image` returns a stale frame for game
        // windows on Windows (xcap#131).
        let image = monitor
            .capture_image()
            .map_err(|_| "could not capture the game window")?;
        Ok(MonitorFrame {
            image,
            origin_x,
            origin_y,
            width,
            height,
        })
    }
}

/// Pick the game's window out of a list of candidates.
///
/// Wine spawns several 1x1 helper windows that share the game's title, and on Windows the launcher
/// briefly holds a window of its own, so the first match is routinely not the game. The largest
/// exact-title match is. The 100px floor drops the helpers before size even matters.
pub fn largest_warframe_window(
    candidates: impl Iterator<Item = (String, WindowRect)>,
) -> Option<WindowRect> {
    candidates
        .filter(|(title, _)| title == WINDOW_TITLE)
        .map(|(_, rect)| rect)
        .filter(|rect| rect.width >= 100 && rect.height >= 100)
        .max_by_key(|rect| u64::from(rect.width) * u64::from(rect.height))
}

fn largest_warframe_window_with_id(
    candidates: impl Iterator<Item = (String, u32, WindowRect)>,
) -> Option<(u32, WindowRect)> {
    candidates
        .filter(|(title, _, _)| title == WINDOW_TITLE)
        .map(|(_, id, rect)| (id, rect))
        .filter(|(_, rect)| rect.width >= 100 && rect.height >= 100)
        .max_by_key(|(_, rect)| u64::from(rect.width) * u64::from(rect.height))
}

#[cfg(target_os = "linux")]
fn parse_xid(value: &str) -> Option<u32> {
    u32::from_str_radix(value.strip_prefix("0x")?, 16).ok()
}

#[cfg(target_os = "linux")]
fn capture_x11_window(id: u32, width: u32, height: u32) -> Result<image::RgbaImage, &'static str> {
    let wire_width = u16::try_from(width).map_err(|_| "the game window is too wide to capture")?;
    let wire_height =
        u16::try_from(height).map_err(|_| "the game window is too tall to capture")?;
    let (connection, _) = Connection::connect(None).map_err(|_| "could not connect to X11")?;
    let window = Window::new(id);
    let attributes_cookie = connection.send_request(&GetWindowAttributes { window });
    let image_cookie = connection.send_request(&GetImage {
        format: ImageFormat::ZPixmap,
        drawable: Drawable::Window(window),
        x: 0,
        y: 0,
        width: wire_width,
        height: wire_height,
        plane_mask: u32::MAX,
    });
    let attributes = connection
        .wait_for_reply(attributes_cookie)
        .map_err(|_| "could not read the X11 window attributes")?;
    let reply = connection
        .wait_for_reply(image_cookie)
        .map_err(|_| "could not capture the game window")?;
    let setup = connection.get_setup();
    let format = setup
        .pixmap_formats()
        .iter()
        .find(|format| format.depth() == reply.depth())
        .ok_or("could not read the X11 pixel format")?;
    let visual = setup
        .roots()
        .flat_map(|screen| screen.allowed_depths())
        .flat_map(|depth| depth.visuals())
        .find(|visual| visual.visual_id() == attributes.visual())
        .ok_or("could not read the X11 window visual")?;
    x11_image_to_rgba(
        reply.data(),
        width,
        height,
        u32::from(format.bits_per_pixel()),
        u32::from(format.scanline_pad()),
        setup.image_byte_order(),
        (visual.red_mask(), visual.green_mask(), visual.blue_mask()),
    )
}

#[cfg(target_os = "linux")]
fn x11_image_to_rgba(
    bytes: &[u8],
    width: u32,
    height: u32,
    bits_per_pixel: u32,
    scanline_pad: u32,
    order: ImageOrder,
    masks: (u32, u32, u32),
) -> Result<image::RgbaImage, &'static str> {
    if !matches!(bits_per_pixel, 16 | 24 | 32)
        || scanline_pad == 0
        || masks.0 == 0
        || masks.1 == 0
        || masks.2 == 0
    {
        return Err("unsupported X11 pixel format");
    }
    let row_bits = width
        .checked_mul(bits_per_pixel)
        .ok_or("the game window is too large to capture")?;
    let row_stride = row_bits
        .checked_add(scanline_pad - 1)
        .map(|bits| bits / scanline_pad * scanline_pad / 8)
        .ok_or("the game window is too large to capture")?;
    let required = row_stride
        .checked_mul(height)
        .and_then(|size| usize::try_from(size).ok())
        .ok_or("the game window is too large to capture")?;
    if bytes.len() < required {
        return Err("the X11 frame was incomplete");
    }
    let output_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .and_then(|size| usize::try_from(size).ok())
        .ok_or("the game window is too large to capture")?;
    let mut rgba = vec![0_u8; output_len];
    let pixel_bytes =
        usize::try_from(bits_per_pixel / 8).map_err(|_| "unsupported X11 pixel format")?;
    let row_stride =
        usize::try_from(row_stride).map_err(|_| "the game window is too large to capture")?;
    let width_usize =
        usize::try_from(width).map_err(|_| "the game window is too large to capture")?;
    let height_usize =
        usize::try_from(height).map_err(|_| "the game window is too large to capture")?;
    for y in 0..height_usize {
        for x in 0..width_usize {
            let source = y * row_stride + x * pixel_bytes;
            let target = (y * width_usize + x) * 4;
            let pixel = match (bits_per_pixel, order) {
                (16, ImageOrder::LsbFirst) => {
                    u32::from(u16::from_le_bytes([bytes[source], bytes[source + 1]]))
                }
                (16, ImageOrder::MsbFirst) => {
                    u32::from(u16::from_be_bytes([bytes[source], bytes[source + 1]]))
                }
                (24, ImageOrder::LsbFirst) => {
                    u32::from_le_bytes([bytes[source], bytes[source + 1], bytes[source + 2], 0])
                }
                (24, ImageOrder::MsbFirst) => {
                    u32::from_be_bytes([0, bytes[source], bytes[source + 1], bytes[source + 2]])
                }
                (32, ImageOrder::LsbFirst) => u32::from_le_bytes([
                    bytes[source],
                    bytes[source + 1],
                    bytes[source + 2],
                    bytes[source + 3],
                ]),
                (32, ImageOrder::MsbFirst) => u32::from_be_bytes([
                    bytes[source],
                    bytes[source + 1],
                    bytes[source + 2],
                    bytes[source + 3],
                ]),
                _ => return Err("unsupported X11 pixel format"),
            };
            let red = masked_channel(pixel, masks.0);
            let green = masked_channel(pixel, masks.1);
            let blue = masked_channel(pixel, masks.2);
            rgba[target..target + 4].copy_from_slice(&[red, green, blue, 255]);
        }
    }
    image::RgbaImage::from_raw(width, height, rgba).ok_or("could not build the captured frame")
}

#[cfg(target_os = "linux")]
fn masked_channel(pixel: u32, mask: u32) -> u8 {
    let shift = mask.trailing_zeros();
    let maximum = mask >> shift;
    let value = (pixel & mask) >> shift;
    ((u64::from(value) * 255) / u64::from(maximum)) as u8
}

#[cfg(target_os = "linux")]
/// `xwininfo -root -tree`, or why it could not be run.
///
/// This used to collapse every launch error into the public capture error, which made a missing
/// `x11-utils` installation indistinguishable from the game not appearing in the tree.
fn xwininfo_tree() -> Result<String, &'static str> {
    match Command::new("xwininfo").args(["-root", "-tree"]).output() {
        Ok(tree) => Ok(String::from_utf8_lossy(&tree.stdout).into_owned()),
        Err(error) => {
            let reason = xwininfo_error_reason(error.kind());
            log::debug!("[DEBUG-capture] {reason}: {error}");
            Err(reason)
        }
    }
}

#[cfg(target_os = "linux")]
fn xwininfo_error_reason(kind: std::io::ErrorKind) -> &'static str {
    match kind {
        std::io::ErrorKind::NotFound => "xwininfo is not installed",
        _ => "could not run xwininfo",
    }
}
#[cfg(target_os = "linux")]
fn xwininfo_retry_ready(retry_after: Option<Instant>, now: Instant) -> bool {
    retry_after.is_none_or(|deadline| now >= deadline)
}
#[cfg(target_os = "linux")]
fn shared_xwininfo_retry_ready(
    retry_after: &std::sync::Mutex<Option<Instant>>,
    now: Instant,
) -> bool {
    let mut retry_after = retry_after
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if !xwininfo_retry_ready(*retry_after, now) {
        return false;
    }
    *retry_after = Some(now + XWININFO_RETRY_INTERVAL);
    true
}

/// Pick the game's window out of `xwininfo -root -tree` output.
///
/// Each line ends with the window's size-and-offset and then its absolute position:
/// `0x1400003 "Warframe": ("Warframe" "steam_app_230410")  1920x1080+1920+0  +1920+0`
///
/// The absolute position is in X root coordinates, which for an XWayland client is the
/// compositor's own output layout -- a window on a second monitor reports that monitor's offset --
/// so the rectangle can be handed straight to the overlay.
///
/// Wine spawns several 1x1 helper windows that share the game's title, and in virtual-desktop mode
/// the real window is nested rather than top-level, so the largest match wins rather than the
/// first one seen.
pub fn warframe_window_from_xwininfo_tree(tree: &str) -> Option<(String, WindowRect)> {
    tree.lines()
        .filter(|line| line.contains("\"Warframe\":"))
        .filter_map(parse_window_line)
        .filter(|(_, rect)| rect.width >= 100 && rect.height >= 100)
        .max_by_key(|(_, rect)| u64::from(rect.width) * u64::from(rect.height))
}

fn parse_window_line(line: &str) -> Option<(String, WindowRect)> {
    let id = line.split_whitespace().next()?;
    let mut tail = line.split_whitespace().rev();
    let absolute = tail.next()?;
    let size = tail.next()?;
    let (width, rest) = size.split_once('x')?;
    let height: String = rest.chars().take_while(char::is_ascii_digit).collect();
    // A negative offset prints as `+-100`, so the leading `+` is a separator and not a sign.
    let (x, y) = absolute.strip_prefix('+')?.split_once('+')?;
    Some((
        id.to_owned(),
        WindowRect {
            x: x.parse().ok()?,
            y: y.parse().ok()?,
            width: width.parse().ok()?,
            height: height.parse().ok()?,
        },
    ))
}

#[cfg(test)]
mod tests {
    #[cfg(target_os = "linux")]
    use std::io::ErrorKind;

    #[cfg(target_os = "linux")]
    use super::{
        XWININFO_RETRY_INTERVAL, shared_xwininfo_retry_ready, x11_image_to_rgba,
        xwininfo_error_reason, xwininfo_retry_ready,
    };
    use super::{largest_warframe_window_with_id, warframe_window_from_xwininfo_tree};
    use crate::overlay_window::WindowRect;
    #[cfg(target_os = "linux")]
    use xcb::x::ImageOrder;

    /// Mutation caught: collapsing `NotFound` into the generic tree-walk failure would make a
    /// missing system dependency indistinguishable from every other launch failure.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_missing_xwininfo_executable_has_a_specific_static_reason() {
        assert_eq!(
            xwininfo_error_reason(ErrorKind::NotFound),
            "xwininfo is not installed"
        );
    }

    /// Mutation caught: mapping every launch failure to the missing-package diagnostic would send
    /// users to install software that is already present when execution failed for another reason.
    #[cfg(target_os = "linux")]
    #[test]
    fn other_xwininfo_launch_failures_keep_the_generic_static_reason() {
        assert_eq!(
            xwininfo_error_reason(ErrorKind::PermissionDenied),
            "could not run xwininfo"
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn failed_tree_walks_are_rate_limited() {
        let now = std::time::Instant::now();
        let retry_after = Some(now + XWININFO_RETRY_INTERVAL);

        assert!(!xwininfo_retry_ready(retry_after, now));
        assert!(!xwininfo_retry_ready(
            retry_after,
            now + XWININFO_RETRY_INTERVAL / 2
        ));
        assert!(xwininfo_retry_ready(
            retry_after,
            now + XWININFO_RETRY_INTERVAL
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn independent_callers_share_the_failed_tree_walk_cooldown() {
        let start = std::time::Instant::now();
        let retry_after = std::sync::Mutex::new(None);

        assert!(shared_xwininfo_retry_ready(&retry_after, start));
        assert!(!shared_xwininfo_retry_ready(
            &retry_after,
            start + XWININFO_RETRY_INTERVAL / 2,
        ));
        assert!(shared_xwininfo_retry_ready(
            &retry_after,
            start + XWININFO_RETRY_INTERVAL,
        ));
    }

    #[test]
    fn window_selection_keeps_the_xid_needed_for_direct_capture() {
        let selected = largest_warframe_window_with_id(
            [
                (
                    "Warframe".to_owned(),
                    0x10,
                    WindowRect {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                    },
                ),
                (
                    "Warframe".to_owned(),
                    0x2a00001,
                    WindowRect {
                        x: 1920,
                        y: 0,
                        width: 1920,
                        height: 1080,
                    },
                ),
            ]
            .into_iter(),
        )
        .expect("the real game window is selected");

        assert_eq!(selected.0, 0x2a00001);
        assert_eq!(selected.1.x, 1920);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn x11_pixels_honor_byte_order_and_scanline_padding() {
        let rgb888 = (0x00ff_0000, 0x0000_ff00, 0x0000_00ff);
        let little = x11_image_to_rgba(
            &[3, 2, 1, 99, 6, 5, 4, 88],
            1,
            2,
            24,
            32,
            ImageOrder::LsbFirst,
            rgb888,
        )
        .expect("the padded little-endian frame converts");
        assert_eq!(little.get_pixel(0, 0).0, [1, 2, 3, 255]);
        assert_eq!(little.get_pixel(0, 1).0, [4, 5, 6, 255]);

        let big = x11_image_to_rgba(&[7, 8, 9, 0], 1, 1, 24, 32, ImageOrder::MsbFirst, rgb888)
            .expect("the padded big-endian frame converts");
        assert_eq!(big.get_pixel(0, 0).0, [7, 8, 9, 255]);

        let rgb565 = x11_image_to_rgba(
            &[0xe0, 0x07],
            1,
            1,
            16,
            16,
            ImageOrder::LsbFirst,
            (0xf800, 0x07e0, 0x001f),
        )
        .expect("the visual masks decode RGB565");
        assert_eq!(rgb565.get_pixel(0, 0).0, [0, 255, 0, 255]);
    }

    /// Real `xwininfo -root -tree` lines. Warframe's IME helpers carry the same class name as the
    /// game window and one of them carries its title too, so picking the first match by name alone
    /// grabs a 1x1 window and captures nothing.
    #[test]
    fn only_the_real_game_window_is_picked_up() {
        let helpers = [
            r#"0x2a00002 "Warframe": ("steam_app_warframe" "steam_app_warframe")  1x1+0+0  +0+0"#,
            r#"0x1e00003 "Warframe": ("steam_app_warframe" "steam_app_warframe")  5x5+0+0  +0+0"#,
            r#"0x1600001 "Warframe": ("steam_app_warframe" "steam_app_warframe")  111x1+8+34  +8+34"#,
        ];
        for helper in helpers {
            assert!(
                warframe_window_from_xwininfo_tree(helper).is_none(),
                "accepted {helper}"
            );
        }

        let game = r#"0x2a00001 "Warframe": ("steam_app_warframe" "steam_app_warframe")  1920x1080+1920+0  +1920+0"#;
        let tree = format!("{}\n{game}\n", helpers.join("\n"));
        let (id, rect) = warframe_window_from_xwininfo_tree(&tree).unwrap();
        assert_eq!(id, "0x2a00001");
        assert_eq!(
            (rect.x, rect.y, rect.width, rect.height),
            (1920, 0, 1920, 1080)
        );
    }
}
