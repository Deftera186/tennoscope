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
/// No clamp on the width: the point is to track the cards, and a clamp is what would break that.
pub fn reward_overlay_geometry(
    screen_width: u32,
    screen_height: u32,
    screen_x: i32,
    screen_y: i32,
    cards: usize,
) -> OverlayGeometry {
    let fraction = |value: f32, of: u32| f64::from(value) * f64::from(of);
    let width = fraction(crate::reward_ocr::card_block_width(cards), screen_width).round() as u32;
    let height = (f64::from(OVERLAY_HEIGHT) * f64::from(screen_height)).round() as u32;
    let x = screen_x
        + i32::try_from(
            fraction(crate::reward_ocr::card_block_left(cards), screen_width).round() as i64,
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
) -> tauri::Result<Option<OverlayGeometry>> {
    let game_rect = warframe_window_rect();
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

/// What to tell the player when the game window could not be located.
///
/// On Windows an exclusive-fullscreen game owns the display outright: it is absent from the window
/// enumeration the overlay measures against, and no window style draws above it. Borderless is the
/// fix, so the panel names it. Linux has no such gap -- the override-redirect strip sits above a
/// Wine fullscreen game -- so there is nothing to ask for there.
pub const fn borderless_notice(found: bool) -> Option<&'static str> {
    if found || !cfg!(windows) {
        return None;
    }
    Some(
        "Warframe window not found. Set Display Mode to Borderless in the game's options; \
         the overlay cannot draw over exclusive fullscreen.",
    )
}

/// The notice for the game as it is right now, or `None` when there is nothing to say.
pub fn overlay_placement_notice() -> Option<&'static str> {
    borderless_notice(warframe_window_rect().is_some())
}

pub fn configure_reward_overlay(window: &WebviewWindow, cards: usize) -> tauri::Result<()> {
    let geometry = overlay_geometry(window, cards)?;
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

pub(crate) fn warframe_window_rect() -> Option<WindowRect> {
    crate::reward_ocr::warframe_window_rect().ok()
}

/// Put the overlay above the game on any window manager or compositor.
///
/// The window is made *override-redirect*, which takes it out of the window manager's hands
/// altogether: it is never reparented, restacked, focused or tiled, and its position is the one we
/// give it. That is what makes the behaviour identical everywhere. The alternatives each cover only
/// part of the field -- `wlr-layer-shell` is absent on GNOME, `_NET_WM_STATE_ABOVE` is ignored by
/// sway, and neither can be relied on to beat a fullscreen game.
///
/// It only works because the whole app runs on X11 (see `run`), in the same display server and the
/// same coordinate space as the Wine/Proton game window it has to line up with.
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
pub fn show_reward_overlay(app: &tauri::AppHandle, cards: usize) {
    if let Some(window) = app.get_webview_window("reward-overlay") {
        let _ = app.run_on_main_thread(move || {
            trace_overlay(&format!("show cards={cards}"));
            #[cfg(target_os = "linux")]
            if let Ok(Some(geometry)) = overlay_geometry(&window, cards) {
                if show_over_game(&window, geometry) {
                    trace_overlay(&format!(
                        "shown override-redirect {}x{} at {},{}",
                        geometry.width, geometry.height, geometry.x, geometry.y
                    ));
                    return;
                }
            }
            let _ = configure_reward_overlay(&window, cards);
            let _ = window.show();
            // Showing a window puts it at the top of its own band, which on Windows is enough to
            // drop it out of the topmost band it was placed in. Re-asserting after the show is what
            // keeps the strip above a borderless game rather than behind it.
            let _ = window.set_always_on_top(true);
            trace_overlay("shown via plain window");
        });
    }
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
