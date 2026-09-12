use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverlayGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WindowRect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

/// Overlay height as a fraction of the screen: room for a wrapped reward name, the value row and a
/// badge row, without covering more of the game than it has to.
const OVERLAY_HEIGHT: f32 = 156.0 / 1080.0;

/// Place the overlay directly under the game's reward cards.
///
/// The rectangle comes from `reward_ocr`'s calibrated card block rather than from proportions
/// invented here, so the overlay is exactly as wide as the cards and starts just below the
/// player-name row. It used to be 75% of the screen wide at 56% of the height, which on a 1080p
/// screen made it 1440px against the cards' 966 and put it ~75px too low, with columns that lined
/// up with nothing.
///
/// `cards` is how many the screen is showing, because the game centres the block on that count --
/// a three-player squad's cards sit half a pitch right of a four-player squad's. Same reason the
/// reader takes it, and the same source, so the strip and the crops can never disagree.
///
/// The block is measured against window *height* and centred horizontally, which is how Warframe
/// scales its HUD. At 16:9 that is indistinguishable from fractions of width, which is why this was
/// width-based for so long; at 16:10 -- a Steam Deck's 1280x800 -- the two disagree by up to 36px
/// on a 178px card, enough to cut a title in half.
///
/// No clamp on the width: the point is to track the cards, and a clamp is what would break that.
pub fn reward_overlay_geometry(
    screen_width: u32,
    screen_height: u32,
    screen_x: i32,
    screen_y: i32,
    cards: usize,
) -> OverlayGeometry {
    let fraction = |value: f32, of: u32| f64::from(value) * f64::from(of);
    let width = f64::from(crate::reward_ocr::card_block_width(cards, screen_height)).round() as u32;
    let height = (f64::from(OVERLAY_HEIGHT) * f64::from(screen_height)).round() as u32;
    let x = screen_x
        + i32::try_from(
            f64::from(crate::reward_ocr::card_block_left(
                cards,
                screen_width,
                screen_height,
            ))
            .round() as i64,
        )
        .unwrap_or_default();
    let y = screen_y
        + i32::try_from(
            fraction(crate::reward_ocr::CARD_BLOCK_BOTTOM, screen_height).round() as i64,
        )
        .unwrap_or_default();
    OverlayGeometry {
        x,
        y,
        width,
        height,
    }
}

fn overlay_geometry(
    window: &WebviewWindow,
    cards: usize,
    game_rect: Option<WindowRect>,
) -> tauri::Result<Option<OverlayGeometry>> {
    let monitor = if game_rect.is_none() {
        window
            .primary_monitor()?
            .or(window.current_monitor()?)
            .or_else(|| window.available_monitors().ok()?.into_iter().next())
    } else {
        None
    };
    Ok(game_rect
        .map(|rect| reward_overlay_geometry(rect.width, rect.height, rect.x, rect.y, cards))
        .or_else(|| {
            monitor.map(|monitor| {
                let size = monitor.size();
                let position = monitor.position();
                reward_overlay_geometry(size.width, size.height, position.x, position.y, cards)
            })
        }))
}

/// The kiosk overlay's rectangle: the game window, all of it.
///
/// The reward strip spans only the card block because its columns line up with cards that never
/// move. The kiosk chips ride a scrolling grid, so they have to move *inside* the overlay window
/// rather than have the window moved under them -- which only works if the window owns the whole
/// game rect to scroll within.
pub fn kiosk_overlay_geometry(
    screen_width: u32,
    screen_height: u32,
    screen_x: i32,
    screen_y: i32,
) -> OverlayGeometry {
    OverlayGeometry {
        x: screen_x,
        y: screen_y,
        width: screen_width,
        height: screen_height,
    }
}

fn kiosk_geometry_from_sources(
    game_rect: Option<WindowRect>,
    monitor_rect: Option<WindowRect>,
    require_game_rect: bool,
) -> Option<OverlayGeometry> {
    game_rect
        .map(|rect| kiosk_overlay_geometry(rect.width, rect.height, rect.x, rect.y))
        .or_else(|| {
            (!require_game_rect).then(|| {
                monitor_rect
                    .map(|rect| kiosk_overlay_geometry(rect.width, rect.height, rect.x, rect.y))
            })?
        })
}

fn kiosk_geometry(window: &WebviewWindow) -> tauri::Result<Option<OverlayGeometry>> {
    let game_rect = warframe_window_rect_with_origin().map(|(rect, _)| rect);
    let monitor = if game_rect.is_none() {
        window
            .primary_monitor()?
            .or(window.current_monitor()?)
            .or_else(|| window.available_monitors().ok()?.into_iter().next())
            .map(|monitor| {
                let size = monitor.size();
                let position = monitor.position();
                WindowRect {
                    x: position.x,
                    y: position.y,
                    width: size.width,
                    height: size.height,
                }
            })
    } else {
        None
    };
    Ok(kiosk_geometry_from_sources(
        game_rect,
        monitor,
        cfg!(target_os = "linux"),
    ))
}

/// What to tell the player when the game window could not be located.
///
/// On Windows an exclusive-fullscreen game owns the display outright: it is absent from the
/// window enumeration and no window style draws above it, so borderless is the fix.
///
/// On Linux the advice is the same but the reason is different. A native Wayland game is
/// invisible to X11 window enumeration no matter what mode it is in, so the capture path falls
/// back to casting the monitor -- which only lines up with the cards when the game fills that
/// monitor. This used to return `None` on every non-Windows platform, so the one user who hit
/// it was told nothing at all.
pub const fn placement_notice(
    exact_window_found: bool,
    session: crate::reward_capture::SessionKind,
) -> Option<&'static str> {
    if exact_window_found {
        return None;
    }
    if cfg!(windows) {
        return Some(
            "Warframe window not found. Set Display Mode to Borderless in the game's options; \
             the overlay cannot draw over exclusive fullscreen.",
        );
    }
    match session {
        crate::reward_capture::SessionKind::Wayland => Some(
            "Warframe window not found. On Wayland the overlay reads the whole monitor, so set \
             Display Mode to Borderless (or Fullscreen) and keep the game on one screen.",
        ),
        crate::reward_capture::SessionKind::X11 => {
            Some("Warframe window not found. Set Display Mode to Borderless in the game's options.")
        }
    }
}

fn configure_reward_overlay(
    window: &WebviewWindow,
    cards: usize,
    game_rect: Option<WindowRect>,
) -> tauri::Result<()> {
    let geometry = overlay_geometry(window, cards, game_rect)?;
    if let Some(geometry) = geometry {
        window.set_size(PhysicalSize::new(geometry.width, geometry.height))?;
        window.set_position(PhysicalPosition::new(geometry.x, geometry.y))?;
    }
    // The three that make this a strip over a game rather than a window: no activation (so clicking
    // nothing steals the game's focus), no hit testing (so the pointer passes through), topmost.
    // On Windows these are exactly `WS_EX_NOACTIVATE`, `WS_EX_TRANSPARENT | WS_EX_LAYERED` and
    // `WS_EX_TOPMOST`; re-asserting topmost on every show is what recovers the z-order after the
    // game has been alt-tabbed back to the front.
    window.set_focusable(false)?;
    window.set_ignore_cursor_events(true)?;
    window.set_always_on_top(true)?;
    // Escape hatch for the one failure this cannot be tested for from here: a WebView2 child HWND
    // under `WS_EX_LAYERED` with no layer attributes is the likeliest way `transparent: true` comes
    // out invisible or black on a real Windows machine. Setting a colour makes the strip opaque --
    // uglier, but readable -- and costs nothing when unset.
    if std::env::var_os("TENNOSCOPE_OPAQUE_OVERLAY").is_some() {
        window.set_background_color(Some(tauri::window::Color(14, 16, 22, 255)))?;
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum GameRectOrigin {
    X11,
    MatchedCapture,
}

fn warframe_window_rect_with_origin() -> Option<(WindowRect, GameRectOrigin)> {
    use crate::reward_capture::{GameRectSource, x11::X11Capture};
    preferred_game_rect(
        X11Capture::new().game_rect().ok(),
        crate::reward_ocr::latest_matched_rect(),
    )
}

fn preferred_game_rect(
    x11_rect: Option<WindowRect>,
    matched_rect: Option<WindowRect>,
) -> Option<(WindowRect, GameRectOrigin)> {
    x11_rect
        .map(|rect| (rect, GameRectOrigin::X11))
        .or_else(|| matched_rect.map(|rect| (rect, GameRectOrigin::MatchedCapture)))
}

/// Same trio of window styles as the reward strip -- click-through, no activation, topmost --
/// sized to the whole game window instead of a card block. See `configure_reward_overlay` for why
/// each of the three is load-bearing.
pub fn configure_kiosk_overlay(window: &WebviewWindow) -> tauri::Result<()> {
    if let Some(geometry) = kiosk_geometry(window)? {
        window.set_size(PhysicalSize::new(geometry.width, geometry.height))?;
        window.set_position(PhysicalPosition::new(geometry.x, geometry.y))?;
    }
    window.set_focusable(false)?;
    window.set_ignore_cursor_events(true)?;
    window.set_always_on_top(true)?;
    if std::env::var_os("TENNOSCOPE_OPAQUE_OVERLAY").is_some() {
        window.set_background_color(Some(tauri::window::Color(14, 16, 22, 255)))?;
    }
    Ok(())
}

/// Put the overlay above the game on any window manager or compositor.
///
/// The window is made *override-redirect*, which takes it out of the window manager's hands
/// altogether: it is never reparented, restacked, or tiled, and its position is the one we give
/// it. That is what makes the behaviour identical everywhere. The alternatives each cover only
/// part of the field -- `wlr-layer-shell` is absent on GNOME, `_NET_WM_STATE_ABOVE` is ignored by
/// sway, and neither can be relied on to beat a fullscreen game.
///
/// It only works because the whole app runs on X11 (see `run`), in the same display server and the
/// same coordinate space as the Wine/Proton game window it has to line up with.
///
/// Focus is the one thing override-redirect does not settle by itself on a wlroots compositor
/// (sway, and anything else built on wlroots). `set_accept_focus(false)` only clears the ICCCM
/// `WM_HINTS` input field, which pure X11 window managers honour but which sway's XWayland
/// override-redirect path (`unmanaged_handle_map` in `sway/desktop/xwayland.c`) never looks at:
/// it decides purely from `_NET_WM_WINDOW_TYPE`, and a plain GTK window (type `NORMAL`, or no type
/// at all) is treated as *wanting* focus, so sway hands the overlay's surface keyboard focus and
/// activation the instant it maps. For a native-Wayland game client (`PROTON_ENABLE_WAYLAND=1`,
/// e.g. the `warframe-wayland` launcher) that focus steal deactivates its `xdg_toplevel`, and
/// winewayland.drv's fullscreen-focus-loss handling can leave the game's own render loop stalled
/// -- confirmed live: the game froze on-screen the instant the overlay mapped, and refocusing it
/// with `swaymsg '[app_id="warframe.x64.exe"] focus'` was what unstuck it. XWayland Warframe never
/// showed this because the game's own window shares the same X11/XWM focus semantics as the
/// overlay there, and winex11.drv's borderless mode does not tie itself to activation the same way.
///
/// The fix is to give the override-redirect window a `_NET_WM_WINDOW_TYPE` that is on wlroots'
/// exclusion list for `wlr_xwayland_or_surface_wants_focus` (utility, tooltip, notification, menu,
/// splash, combo, dnd, or a popup/dropdown menu) instead of the default `NORMAL`. `Utility` is the
/// closest fit for a strip that only ever displays, never accepts input.
#[cfg(target_os = "linux")]
fn show_over_game(window: &WebviewWindow, geometry: OverlayGeometry) -> bool {
    use gtk::prelude::{GtkWindowExt, WidgetExt};

    let Ok(gtk_window) = window.gtk_window() else {
        return false;
    };
    gtk_window.set_accept_focus(false);
    // Override-redirect has to be set while the window is unmapped, or the window manager has
    // already taken it. Realizing first is what creates the underlying window to set it on.
    gtk_window.realize();
    let Some(gdk_window) = gtk_window.window() else {
        return false;
    };
    gdk_window.set_override_redirect(true);
    // Stops wlroots compositors (sway) from handing this window keyboard focus/activation on map
    // -- see the doc comment above for why that matters beyond just stealing input.
    gdk_window.set_type_hint(gtk::gdk::WindowTypeHint::Utility);
    let width = i32::try_from(geometry.width).unwrap_or(966);
    let height = i32::try_from(geometry.height).unwrap_or(156);
    // The overlay is one column per card, sized to the game's own card block, so extra width is
    // shared out and every column renders wider than the reward it sits under. `set_default_size`
    // is only a hint; `set_size_request` is the part that pins it.
    gtk_window.set_size_request(width, height);
    gtk_window.resize(width, height);
    gtk_window.move_(geometry.x, geometry.y);
    gtk_window.show_all();
    // Nothing else will restack us, so raising is ours to do -- and the move is reissued because a
    // position set before the window is on screen is not always the one it keeps.
    gdk_window.raise();
    gtk_window.move_(geometry.x, geometry.y);
    // Click-through is the property that is felt: without it the strip is an input-grabbing surface
    // over the game and the pointer catches on it for as long as the overlay is up.
    let _ = window.set_ignore_cursor_events(true);
    true
}

/// Both ends of the overlay's life are traced, because "the overlay lingered" has several possible
/// owners -- the poller not noticing the screen went, the monitor not acting on it, or the hide
/// call itself not taking effect -- and they are indistinguishable from outside.
fn trace_overlay(action: &str) {
    log::debug!("[DEBUG-overlay] {action}");
}

/// `cards` is how many rewards are on screen, so the strip lands on the block the game actually
/// drew rather than on a four-card block it may not have.
pub fn show_reward_overlay(app: &tauri::AppHandle, cards: usize) -> Option<&'static str> {
    let located = warframe_window_rect_with_origin();
    let notice = placement_notice(
        matches!(located, Some((_, GameRectOrigin::X11))),
        crate::reward_capture::session_kind(),
    );
    let game_rect = located.map(|(rect, _)| rect);
    if let Some(window) = app.get_webview_window("reward-overlay") {
        let _ = app.run_on_main_thread(move || {
            trace_overlay(&format!("show cards={cards}"));
            #[cfg(target_os = "linux")]
            if let Ok(Some(geometry)) = overlay_geometry(&window, cards, game_rect) {
                if show_over_game(&window, geometry) {
                    trace_overlay(&format!(
                        "shown override-redirect {}x{} at {},{}",
                        geometry.width, geometry.height, geometry.x, geometry.y
                    ));
                    return;
                }
            }
            let _ = configure_reward_overlay(&window, cards, game_rect);
            let _ = window.show();
            // Showing a window puts it at the top of its own band, which on Windows is enough to
            // drop it out of the topmost band it was placed in. Re-asserting after the show is what
            // keeps the strip above a borderless game rather than behind it.
            let _ = window.set_always_on_top(true);
            trace_overlay("shown via plain window");
        });
    }
    notice
}

pub fn hide_reward_overlay(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("reward-overlay") {
        let _ = app.run_on_main_thread(move || {
            trace_overlay("hide");
            let _ = window.hide();
            trace_overlay("hidden");
        });
    }
}

pub fn show_kiosk_overlay(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("kiosk-overlay") {
        let _ = app.run_on_main_thread(move || {
            trace_overlay("show kiosk");
            #[cfg(target_os = "linux")]
            if let Ok(Some(geometry)) = kiosk_geometry(&window) {
                if show_over_game(&window, geometry) {
                    trace_overlay(&format!(
                        "kiosk shown override-redirect {}x{} at {},{}",
                        geometry.width, geometry.height, geometry.x, geometry.y
                    ));
                    return;
                }
            }
            #[cfg(target_os = "linux")]
            if kiosk_geometry(&window).ok().flatten().is_none() {
                trace_overlay("kiosk show deferred until capture locates the game");
                return;
            }
            let _ = configure_kiosk_overlay(&window);
            let _ = window.show();
        });
    }
}

fn dispatch_on_main_thread_and_wait<E>(
    dispatch: impl FnOnce(Box<dyn FnOnce() + Send>) -> Result<(), E>,
    action: impl FnOnce() + Send + 'static,
) {
    let (done, wait) = std::sync::mpsc::sync_channel(0);
    if dispatch(Box::new(move || {
        action();
        let _ = done.send(());
    }))
    .is_ok()
    {
        let _ = wait.recv();
    }
}

pub fn hide_kiosk_overlay(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("kiosk-overlay") {
        let main_thread = app.clone();
        dispatch_on_main_thread_and_wait(
            move |action| main_thread.run_on_main_thread(action),
            move || {
                trace_overlay("hide kiosk");
                let _ = window.hide();
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::{
        GameRectOrigin, WindowRect, kiosk_geometry_from_sources, placement_notice,
        preferred_game_rect,
    };
    use crate::overlay_window::kiosk_overlay_geometry;
    use crate::reward_capture::SessionKind;

    /// Current X11 geometry is fresher than the last OCR-matched rectangle after the game moves.
    #[test]
    fn overlay_prefers_current_x11_geometry_then_uses_the_ocr_match() {
        let x11 = WindowRect {
            x: 50,
            y: 60,
            width: 1600,
            height: 900,
        };
        let matched = WindowRect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };

        assert_eq!(
            preferred_game_rect(Some(x11), Some(matched)),
            Some((x11, GameRectOrigin::X11))
        );
        assert_eq!(
            preferred_game_rect(None, Some(matched)),
            Some((matched, GameRectOrigin::MatchedCapture))
        );
        assert_eq!(preferred_game_rect(None, None), None);
    }

    #[test]
    fn linux_kiosk_geometry_waits_for_a_located_game_capture() {
        let monitor = WindowRect {
            x: 1920,
            y: 0,
            width: 1920,
            height: 1080,
        };

        assert_eq!(kiosk_geometry_from_sources(None, Some(monitor), true), None);
        assert_eq!(
            kiosk_geometry_from_sources(Some(monitor), None, true),
            Some(kiosk_overlay_geometry(1920, 1080, 1920, 0))
        );
    }

    /// Exact X11 geometry needs no advice, including for XWayland under a Wayland session.
    #[test]
    fn an_exact_window_match_says_nothing() {
        assert!(placement_notice(true, SessionKind::X11).is_none());
        assert!(placement_notice(true, SessionKind::Wayland).is_none());
    }

    /// An OCR match can supply monitor geometry without proving that the game fills that monitor.
    /// Wayland players still need the display-mode guidance in that case.
    #[test]
    fn a_missing_exact_window_on_wayland_names_borderless_and_fullscreen() {
        let notice = placement_notice(false, SessionKind::Wayland)
            .expect("a Wayland session without exact window geometry has something to say");
        let lower = notice.to_lowercase();
        assert!(lower.contains("borderless"), "notice was: {notice}");
        assert!(lower.contains("fullscreen"), "notice was: {notice}");
    }

    #[test]
    fn a_missing_window_on_an_x11_session_also_explains_itself() {
        assert!(placement_notice(false, SessionKind::X11).is_some());
    }

    #[test]
    fn main_thread_dispatch_waits_until_the_ui_action_finishes() {
        let (queued_tx, queued_rx) = std::sync::mpsc::channel();
        let (acted_tx, acted_rx) = std::sync::mpsc::channel();

        let waiter = std::thread::spawn(move || {
            super::dispatch_on_main_thread_and_wait(
                |action| queued_tx.send(action).map_err(|_| ()),
                move || acted_tx.send(()).expect("record action"),
            );
        });

        let action = queued_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("UI action queued");
        assert!(
            !waiter.is_finished(),
            "dispatch returned before the UI action ran"
        );
        action();
        acted_rx
            .recv_timeout(std::time::Duration::from_secs(1))
            .expect("UI action finished");
        waiter.join().expect("dispatch waiter");
    }
}
