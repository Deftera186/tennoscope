//! Turning a whole-monitor capture into a window-sized frame.
//!
//! Every backend in this module captures a whole monitor and hands the window rect alongside it,
//! so the transform from "these are the monitor's pixels" to "this is the game window" is the
//! same arithmetic whatever produced the frame. It lives next to the backends because it is the
//! second half of a capture, not the first half of a read: OCR downstream only ever sees a
//! window-sized frame.
//!
//! The whole monitor is captured and cropped here rather than through `capture_region`, because
//! `capture_region` ignored which monitor it was asked. Measured on a two-output XWayland desktop
//! with xcap 0.9.8: `HDMI-A-1` at origin (0,0) and `HDMI-A-2` at (1920,0), and
//! `capture_region(0, 0, 1920, 1080)` returned byte-identical frames for both -- both of them
//! `HDMI-A-1`'s pixels. So a game on any monitor but the first read the first monitor's pixels,
//! every card missed the relic pool, and the overlay never appeared while the log showed a poller
//! running normally -- precisely the 2026-08-20 report, where the four cards were sitting on the
//! second monitor and read as `WOH DIGeil` and similar.
//!
//! Capturing the monitor rather than the window is also what avoids a stale frame: xcap's
//! `Window::capture_image` returns one for game windows on Windows (xcap#131), and a reward screen
//! read from a stale frame is a reward screen read from whatever was on screen a moment ago.

use crate::overlay_window::WindowRect;

/// The part of the game window that is actually on this monitor.
///
/// `capture_region` rejects a region that reaches past the monitor rather than clipping it, so a
/// game in windowed mode sitting even one pixel off the edge -- which is ordinary, the title bar
/// gets dragged -- makes every capture fail with "could not capture the game window". Clamping the
/// offset alone is worse than failing: the region would then be captured from the wrong place and
/// the reward crops would silently read the pixels next to the cards.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct VisibleRegion {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
    /// Where the captured piece belongs within the window-sized frame.
    paste_x: u32,
    paste_y: u32,
}

pub(crate) fn visible_region(
    rect: WindowRect,
    origin_x: i32,
    origin_y: i32,
    monitor_width: u32,
    monitor_height: u32,
) -> Option<VisibleRegion> {
    let (x, width, paste_x) = visible_axis(rect.x, origin_x, rect.width, monitor_width)?;
    let (y, height, paste_y) = visible_axis(rect.y, origin_y, rect.height, monitor_height)?;
    (width > 0 && height > 0).then_some(VisibleRegion {
        x,
        y,
        width,
        height,
        paste_x,
        paste_y,
    })
}

/// One axis of the clip: where to capture from, how much of it is on the monitor, and how far into
/// the window-sized frame the piece belongs.
fn visible_axis(start: i32, origin: i32, span: u32, monitor: u32) -> Option<(u32, u32, u32)> {
    let offset = i64::from(start) - i64::from(origin);
    let clipped = offset.max(0);
    let visible = (offset + i64::from(span)).min(i64::from(monitor)) - clipped;
    Some((
        u32::try_from(clipped).ok()?,
        u32::try_from(visible.max(0)).ok()?,
        u32::try_from(clipped - offset).ok()?,
    ))
}

/// Cut the game window out of a whole-monitor capture and lay it into a window-sized frame.
///
/// Split from the backends because everything there talks to the compositor and everything here is
/// arithmetic on pixels. The monitor-mixup bug lived in this half but could only be reached through
/// the other, so nothing could test it; this is the seam that makes the multi-monitor case
/// assertable without a second physical screen.
pub(crate) fn window_frame_from_monitor(
    whole: &image::RgbaImage,
    monitor_width: u32,
    monitor_height: u32,
    rect: WindowRect,
    visible: VisibleRegion,
) -> image::DynamicImage {
    // A scaled display hands back the framebuffer's pixels, not the logical ones asked for: X and
    // the window manager speak in logical units, the compositor captures physical ones. The crop
    // below is in logical units, so an unscaled frame would have it read the wrong place entirely
    // -- and the paste after it clips rather than scales, so an oversized piece becomes a magnified
    // corner and every card reads blank, which is exactly how a working reader looks from outside.
    // Resampling the whole monitor to its logical size first is what makes the fractions mean the
    // same thing on a scaled desktop as on an unscaled one.
    let resampled;
    let whole = if whole.dimensions() == (monitor_width, monitor_height) {
        whole
    } else {
        log::debug!(
            "[DEBUG-capture] scaled capture {:?} for monitor {monitor_width}x{monitor_height}",
            whole.dimensions(),
        );
        resampled = image::imageops::resize(
            whole,
            monitor_width,
            monitor_height,
            image::imageops::FilterType::Lanczos3,
        );
        &resampled
    };
    // Crop here rather than asking `capture_region` for the piece, because it hands back the first
    // monitor's pixels whatever monitor it belongs to -- see the module note above.
    let captured =
        image::imageops::crop_imm(whole, visible.x, visible.y, visible.width, visible.height)
            .to_image();
    // Paste back at the window's own origin: every crop downstream is a fraction of the *window*,
    // so the frame handed on has to be window sized even when part of it was off screen.
    let mut frame = image::RgbaImage::new(rect.width, rect.height);
    image::imageops::replace(
        &mut frame,
        &captured,
        i64::from(visible.paste_x),
        i64::from(visible.paste_y),
    );
    image::DynamicImage::ImageRgba8(frame)
}

#[cfg(test)]
mod tests {
    use image::GenericImageView;

    use super::{VisibleRegion, WindowRect, visible_region, window_frame_from_monitor};

    /// A flat monitor capture, tagged so a frame can be traced back to the screen it came from.
    fn monitor(width: u32, height: u32, tag: u8) -> image::RgbaImage {
        image::RgbaImage::from_pixel(width, height, image::Rgba([tag, tag, tag, 255]))
    }

    /// The reward screen must be cut from the capture of the monitor the game is actually on.
    ///
    /// This is the 2026-08-20 bug, and it was not a geometry mistake: `capture_region` hands back
    /// the *first* monitor's pixels whatever monitor it is asked, measured byte-identical on a
    /// two-output desktop with xcap 0.9.8. So a game on the second screen was read against the
    /// first screen's pixels, every card missed the relic pool, `poll failed: reward card text did
    /// not match the relic pool` repeated for the whole three minutes of the fissure, and no
    /// overlay ever appeared. Reading the report's own screenshot confirmed it: the left half read
    /// as `WOH DIGeil`, the right half -- where the game was -- read all four cards exactly.
    ///
    /// Asserting on the tag is what pins it. A frame built from the handed-in capture carries that
    /// capture's tag; one that quietly sampled another screen carries the other's.
    #[test]
    fn the_frame_is_cut_from_the_monitor_the_game_is_on() {
        // Warframe fullscreen on the second monitor of a 3840x1080 desktop, which is the reported
        // layout: `HDMI-A-1` at (0,0) and `HDMI-A-2` at (1920,0).
        let rect = WindowRect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let visible = visible_region(rect, 1920, 0, 1920, 1080).expect("the window is on screen");
        assert_eq!(
            (visible.x, visible.y, visible.paste_x, visible.paste_y),
            (0, 0, 0, 0),
            "a fullscreen window is captured monitor-relative, so the offsets are the origin"
        );

        let second = monitor(1920, 1080, 200);
        let frame = window_frame_from_monitor(&second, 1920, 1080, rect, visible);
        assert_eq!(frame.dimensions(), (1920, 1080));
        assert_eq!(
            frame.to_rgba8().get_pixel(960, 540)[0],
            200,
            "the frame must carry the pixels of the monitor it was handed, not another screen's"
        );

        // The other screen's capture is the same shape and the same call, and must never be what a
        // game on the second monitor reads -- that is the whole of the bug.
        let first = monitor(1920, 1080, 40);
        let wrong = window_frame_from_monitor(&first, 1920, 1080, rect, visible);
        assert_ne!(
            wrong.to_rgba8().get_pixel(960, 540)[0],
            frame.to_rgba8().get_pixel(960, 540)[0],
            "two monitors with different contents must not produce the same frame"
        );
    }

    /// A window hanging off the monitor keeps its place in the window-sized frame: the captured
    /// piece is pasted at the offset it belongs to, and the rest stays empty rather than sliding
    /// the cards over by the width of the missing strip.
    #[test]
    fn a_clipped_window_is_pasted_where_it_belongs() {
        let rect = WindowRect {
            x: -100,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let visible = visible_region(rect, 0, 0, 1920, 1080).expect("partly on screen");
        assert_eq!((visible.x, visible.width, visible.paste_x), (0, 1820, 100));

        let frame = window_frame_from_monitor(&monitor(1920, 1080, 200), 1920, 1080, rect, visible)
            .to_rgba8();
        assert_eq!(frame.dimensions(), (1920, 1080));
        assert_eq!(
            frame.get_pixel(0, 540),
            &image::Rgba([0, 0, 0, 0]),
            "the strip that was off screen stays empty"
        );
        assert_eq!(
            frame.get_pixel(1000, 540)[0],
            200,
            "the piece that was on screen lands at its own offset"
        );
    }

    /// A scaled desktop hands back framebuffer pixels, and the crop that follows is in logical
    /// units, so the whole monitor is resampled to its logical size before anything is cut out of
    /// it. Without that the crop reads the wrong place entirely.
    #[test]
    fn an_oversized_capture_is_resampled_to_the_logical_monitor_first() {
        let rect = WindowRect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        let visible = visible_region(rect, 1920, 0, 1920, 1080).expect("the window is on screen");
        // 1.5x scaling: the compositor hands back 2880x1620 for a 1920x1080 logical screen.
        let frame = window_frame_from_monitor(&monitor(2880, 1620, 200), 1920, 1080, rect, visible);
        assert_eq!(
            frame.dimensions(),
            (1920, 1080),
            "the frame is window sized whatever the framebuffer's scale"
        );
        assert_eq!(frame.to_rgba8().get_pixel(1900, 1070)[0], 200);
    }

    /// `capture_region` rejects an out-of-bounds region rather than clipping it, so a game window
    /// hanging off the edge of the monitor -- ordinary in windowed mode, and the shape a second
    /// monitor produces at every capture -- failed the whole read. The clip is what keeps that a
    /// partial frame instead of no frame.
    #[test]
    fn a_window_hanging_off_the_monitor_is_clipped_rather_than_refused() {
        let full = WindowRect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            visible_region(full, 1920, 0, 1920, 1080),
            Some(VisibleRegion {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
                paste_x: 0,
                paste_y: 0
            }),
            "a window filling its monitor must capture whole and unshifted"
        );

        // Dragged 100px past the right edge and 50 above the top: the capture shrinks, and the
        // piece that survives belongs 50px down in the window-sized frame, not at its origin.
        let overhanging = WindowRect {
            x: 1820,
            y: -50,
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            visible_region(overhanging, 0, 0, 1920, 1080),
            Some(VisibleRegion {
                x: 1820,
                y: 0,
                width: 100,
                height: 1030,
                paste_x: 0,
                paste_y: 50
            })
        );

        // Entirely off the monitor is the one case with nothing to read; an empty region would be
        // rejected by `capture_region` anyway, so it has to be an absence here.
        let elsewhere = WindowRect {
            x: 4000,
            y: 0,
            width: 800,
            height: 600,
        };
        assert_eq!(visible_region(elsewhere, 0, 0, 1920, 1080), None);
    }
}
