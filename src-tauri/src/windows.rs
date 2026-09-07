use tauri::{AppHandle, Emitter, Manager, WebviewWindow};

pub const LAUNCHER: &str = "main";
pub const SETTINGS: &str = "settings";

pub fn launcher(app: &AppHandle) -> Option<WebviewWindow> {
    app.get_webview_window(LAUNCHER)
}

/// Show the launcher centred on the active monitor, a third from the top.
///
/// Spotlight sits above the middle of the screen, which reads as "in front of"
/// your work rather than "on top of" it.
pub fn show_launcher(app: &AppHandle) {
    let Some(window) = launcher(app) else {
        return;
    };

    position_launcher(&window);
    let _ = window.show();
    let _ = window.set_always_on_top(true);
    let _ = window.set_focus();
    let _ = app.emit_to(LAUNCHER, "klets://focus", ());
}

pub fn hide_launcher(app: &AppHandle) {
    if let Some(window) = launcher(app) {
        let _ = window.hide();
    }
}

pub fn toggle_launcher(app: &AppHandle) {
    let Some(window) = launcher(app) else {
        return;
    };

    match window.is_visible() {
        Ok(true) => hide_launcher(app),
        _ => show_launcher(app),
    }
}

fn position_launcher(window: &WebviewWindow) {
    let Ok(Some(monitor)) = window.current_monitor() else {
        let _ = window.center();
        return;
    };

    let screen = monitor.size();
    let position = monitor.position();
    let Ok(size) = window.outer_size() else {
        return;
    };

    let x = position.x + ((screen.width as i32 - size.width as i32) / 2);
    let y = position.y + (screen.height as i32 / 5);

    let _ = window.set_position(tauri::PhysicalPosition::new(x, y));
}

pub fn open_settings(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(SETTINGS) {
        let _ = window.show();
        let _ = window.set_focus();
    }
}
