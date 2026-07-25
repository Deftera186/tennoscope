use tauri::{Manager, PhysicalPosition, PhysicalSize, WebviewWindow};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OverlayGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

pub fn reward_overlay_geometry(
    screen_width: u32,
    screen_height: u32,
    screen_x: i32,
    screen_y: i32,
) -> OverlayGeometry {
    let width = ((screen_width as f64 * 0.75).round() as u32).clamp(720, 1600);
    let height = 148;
    let x = screen_x + i32::try_from((screen_width - width) / 2).unwrap_or_default();
    let y =
        screen_y + i32::try_from((screen_height as f64 * 0.56).round() as i64).unwrap_or_default();
    OverlayGeometry {
        x,
        y,
        width,
        height,
    }
}

pub fn configure_reward_overlay(window: &WebviewWindow) -> tauri::Result<()> {
    let monitor = window
        .primary_monitor()?
        .or(window.current_monitor()?)
        .or_else(|| window.available_monitors().ok()?.into_iter().next());
    if let Some(monitor) = monitor {
        let size = monitor.size();
        let position = monitor.position();
        let geometry = reward_overlay_geometry(size.width, size.height, position.x, position.y);
        window.set_size(PhysicalSize::new(geometry.width, geometry.height))?;
        window.set_position(PhysicalPosition::new(geometry.x, geometry.y))?;
    }
    window.set_focusable(false)?;
    window.set_ignore_cursor_events(true)?;
    window.set_always_on_top(true)?;

    #[cfg(target_os = "linux")]
    configure_linux_layer(window);
    Ok(())
}

#[cfg(target_os = "linux")]
fn configure_linux_layer(window: &WebviewWindow) {
    use gtk::prelude::{GtkWindowExt, WidgetExt};
    use gtk_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};

    if std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return;
    }
    let Ok(gtk_window) = window.gtk_window() else {
        return;
    };
    if gtk_window.is_visible() {
        gtk_window.hide();
    }
    if !gtk_window.is_layer_window() {
        gtk_window.init_layer_shell();
        gtk_window.set_namespace("tennoscope-reward-overlay");
    }
    gtk_window.set_layer(Layer::Overlay);
    gtk_window.set_keyboard_mode(KeyboardMode::None);
    gtk_window.set_exclusive_zone(0);
    gtk_window.set_anchor(Edge::Top, true);
    gtk_window.set_anchor(Edge::Left, true);
    gtk_window.set_anchor(Edge::Right, false);
    gtk_window.set_anchor(Edge::Bottom, false);
    if let Ok(position) = window.outer_position() {
        gtk_window.set_layer_shell_margin(Edge::Top, position.y.max(0));
        gtk_window.set_layer_shell_margin(Edge::Left, position.x.max(0));
    }
    gtk_window.set_accept_focus(false);
}

pub fn show_reward_overlay(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("reward-overlay") {
        let _ = configure_reward_overlay(&window);
        let _ = window.show();
    }
}

pub fn hide_reward_overlay(app: &tauri::AppHandle) {
    if let Some(window) = app.get_webview_window("reward-overlay") {
        let _ = window.hide();
    }
}
