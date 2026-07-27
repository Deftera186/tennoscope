use std::process::Command;

#[cfg(target_os = "linux")]
use gtk::prelude::WidgetExt;
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

/// Place the overlay directly under the game's four reward cards.
///
/// The rectangle comes from `reward_ocr`'s calibrated card block rather than from proportions
/// invented here, so the overlay is exactly as wide as the cards and starts just below the
/// player-name row. It used to be 75% of the screen wide at 56% of the height, which on a 1080p
/// screen made it 1440px against the cards' 966 and put it ~75px too low, with columns that lined
/// up with nothing.
///
/// No clamp on the width: the point is to track the cards, and a clamp is what would break that.
pub fn reward_overlay_geometry(
    screen_width: u32,
    screen_height: u32,
    screen_x: i32,
    screen_y: i32,
) -> OverlayGeometry {
    let fraction = |value: f32, of: u32| f64::from(value) * f64::from(of);
    let width = fraction(crate::reward_ocr::CARD_BLOCK_WIDTH, screen_width).round() as u32;
    let height = (f64::from(OVERLAY_HEIGHT) * f64::from(screen_height)).round() as u32;
    let x = screen_x
        + i32::try_from(fraction(crate::reward_ocr::CARD_BLOCK_LEFT, screen_width).round() as i64)
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

fn overlay_geometry(window: &WebviewWindow) -> tauri::Result<Option<OverlayGeometry>> {
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
        .map(|rect| reward_overlay_geometry(rect.width, rect.height, rect.x, rect.y))
        .or_else(|| {
            monitor.map(|monitor| {
                let size = monitor.size();
                let position = monitor.position();
                reward_overlay_geometry(size.width, size.height, position.x, position.y)
            })
        }))
}

pub fn configure_reward_overlay(window: &WebviewWindow) -> tauri::Result<()> {
    let geometry = overlay_geometry(window)?;
    if let Some(geometry) = geometry {
        window.set_size(PhysicalSize::new(geometry.width, geometry.height))?;
        window.set_position(PhysicalPosition::new(geometry.x, geometry.y))?;
    }
    window.set_focusable(false)?;
    window.set_ignore_cursor_events(true)?;
    window.set_always_on_top(true)?;
    Ok(())
}

pub(crate) fn warframe_window_rect() -> Option<WindowRect> {
    let output = Command::new("swaymsg")
        .args(["-t", "get_tree", "-r"])
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| warframe_window_rect_from_sway_tree(&output.stdout))
        .flatten()
}

pub fn warframe_window_rect_from_sway_tree(bytes: &[u8]) -> Option<WindowRect> {
    fn visit(value: &serde_json::Value) -> Option<WindowRect> {
        if let Some(object) = value.as_object() {
            let title = object.get("name").and_then(serde_json::Value::as_str);
            let class = object
                .get("window_properties")
                .and_then(|properties| properties.get("class"))
                .and_then(serde_json::Value::as_str);
            let visible = object
                .get("visible")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            let is_warframe = title.is_some_and(|title| title.eq_ignore_ascii_case("warframe"))
                || class.is_some_and(|class| class.eq_ignore_ascii_case("steam_app_warframe"));
            if visible && is_warframe {
                let rect = object.get("rect")?;
                return Some(WindowRect {
                    x: i32::try_from(rect.get("x")?.as_i64()?).ok()?,
                    y: i32::try_from(rect.get("y")?.as_i64()?).ok()?,
                    width: u32::try_from(rect.get("width")?.as_u64()?).ok()?,
                    height: u32::try_from(rect.get("height")?.as_u64()?).ok()?,
                });
            }
            for child in object.values() {
                if let Some(rect) = visit(child) {
                    return Some(rect);
                }
            }
        } else if let Some(array) = value.as_array() {
            for child in array {
                if let Some(rect) = visit(child) {
                    return Some(rect);
                }
            }
        }
        None
    }

    serde_json::from_slice(bytes)
        .ok()
        .and_then(|tree| visit(&tree))
}

#[cfg(target_os = "linux")]
fn configure_linux_layer(window: &WebviewWindow, geometry: OverlayGeometry) -> bool {
    use gtk::gdk::prelude::MonitorExt;
    use gtk::prelude::{GtkWindowExt, WidgetExt};
    use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return false;
    }
    let Ok(gtk_window) = window.gtk_window() else {
        return false;
    };
    if gtk_window.is_visible() {
        gtk_window.hide();
    }
    if !gtk_window.is_layer_window() {
        gtk_window.init_layer_shell();
        gtk_window.set_namespace("tennoscope-reward-overlay");
    }
    let display = gtk_window.display();
    let monitor = display.monitor_at_point(geometry.x, geometry.y);
    if let Some(monitor) = monitor.as_ref() {
        gtk_window.set_monitor(monitor);
    }
    gtk_window.set_layer(Layer::Overlay);
    gtk_window.set_keyboard_mode(KeyboardMode::None);
    gtk_window.set_exclusive_zone(0);
    gtk_window.set_anchor(Edge::Top, true);
    gtk_window.set_anchor(Edge::Left, true);
    gtk_window.set_anchor(Edge::Right, false);
    gtk_window.set_anchor(Edge::Bottom, false);
    let monitor_geometry = monitor.map(|monitor| monitor.geometry());
    let monitor_x = monitor_geometry.as_ref().map_or(0, |rect| rect.x());
    let monitor_y = monitor_geometry.as_ref().map_or(0, |rect| rect.y());
    gtk_window.set_layer_shell_margin(Edge::Top, (geometry.y - monitor_y).max(0));
    gtk_window.set_layer_shell_margin(Edge::Left, (geometry.x - monitor_x).max(0));
    gtk_window.set_accept_focus(false);
    let width = i32::try_from(geometry.width).unwrap_or(966);
    let height = i32::try_from(geometry.height).unwrap_or(156);
    // `set_default_size` is only an initial hint, and a layer surface anchored on two edges is free
    // to come out wider than it. The overlay is a four-column grid sized to the game's four cards,
    // so any extra width is shared out and every card renders wider than the reward it sits under.
    // `set_size_request` is the part that actually pins it.
    gtk_window.set_size_request(width, height);
    gtk_window.set_default_size(width, height);
    gtk_window.show_all();
    // The rest of the overlay's window properties live in `configure_reward_overlay`, which this
    // path returns before ever reaching. Click-through is the one that is felt: without it the
    // strip is an input-grabbing surface sitting over the game, so the pointer catches on it for as
    // long as the overlay is up.
    let _ = window.set_ignore_cursor_events(true);
    let _ = window.set_focusable(false);
    true
}

/// Both ends of the overlay's life are traced, because "the overlay lingered" has several possible
/// owners -- the poller not noticing the screen went, the monitor not acting on it, or the hide
/// call itself not taking effect -- and they are indistinguishable from outside.
#[cfg(debug_assertions)]
fn trace_overlay(action: &str) {
    warframe_acquisition::append_debug_line(&format!("[DEBUG-overlay] {action}"));
}

pub fn show_reward_overlay(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("reward-overlay") {
        let _ = app.run_on_main_thread(move || {
            #[cfg(debug_assertions)]
            trace_overlay("show");
            #[cfg(target_os = "linux")]
            if let Ok(Some(geometry)) = overlay_geometry(&window) {
                if configure_linux_layer(&window, geometry) {
                    #[cfg(debug_assertions)]
                    trace_overlay(&format!(
                        "shown via layer-shell {}x{} at {},{}",
                        geometry.width, geometry.height, geometry.x, geometry.y
                    ));
                    return;
                }
            }
            let _ = configure_reward_overlay(&window);
            let _ = window.show();
            #[cfg(debug_assertions)]
            trace_overlay("shown via plain window");
        });
    }
}

pub fn hide_reward_overlay(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("reward-overlay") {
        let _ = app.run_on_main_thread(move || {
            #[cfg(debug_assertions)]
            trace_overlay("hide");
            #[cfg(target_os = "linux")]
            if std::env::var_os("WAYLAND_DISPLAY").is_some() {
                if let Ok(gtk_window) = window.gtk_window() {
                    gtk_window.hide();
                    #[cfg(debug_assertions)]
                    trace_overlay("hidden via gtk");
                    return;
                }
            }
            let _ = window.hide();
            #[cfg(debug_assertions)]
            trace_overlay("hidden via plain window");
        });
    }
}
