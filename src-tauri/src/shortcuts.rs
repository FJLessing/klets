use tauri::AppHandle;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};
use tauri_plugin_autostart::ManagerExt;

use crate::error::{AppError, AppResult};
use crate::windows;

fn parse(accelerator: &str) -> AppResult<Shortcut> {
    accelerator
        .parse::<Shortcut>()
        .map_err(|e| AppError::Config(format!("'{accelerator}' is not a valid shortcut: {e}")))
}

/// Register the launcher hotkey, falling back to the default if the user's
/// choice is already taken by another app.
pub fn register(app: &AppHandle, accelerator: &str) -> AppResult<()> {
    let shortcut = parse(accelerator)?;
    let handle = app.clone();

    app.global_shortcut()
        .on_shortcut(shortcut, move |_app, _shortcut, event| {
            if event.state() == ShortcutState::Pressed {
                windows::toggle_launcher(&handle);
            }
        })
        .map_err(|e| {
            AppError::Config(format!(
                "could not register '{accelerator}': {e}. Another app may already use it."
            ))
        })
}

pub fn unregister(app: &AppHandle, accelerator: &str) {
    if let Ok(shortcut) = parse(accelerator) {
        let _ = app.global_shortcut().unregister(shortcut);
    }
}

pub fn rebind(app: &AppHandle, old: &str, new: &str) -> AppResult<()> {
    unregister(app, old);
    match register(app, new) {
        Ok(()) => Ok(()),
        Err(e) => {
            // Never leave the user without a way to open the launcher.
            let _ = register(app, old);
            Err(e)
        }
    }
}

/// Register or remove this executable as a login item.
///
/// Dev builds never touch it. The plugin registers whichever binary is
/// running, and a dev binary loads its UI from the Vite server (`devUrl`),
/// which isn't up at login, so every boot would open a launcher reading
/// "localhost refused to connect". The setting still saves either way.
pub fn set_autostart(app: &AppHandle, enabled: bool) -> AppResult<()> {
    if tauri::is_dev() {
        tracing::info!(target: "klets", "dev build: leaving launch at login untouched");
        return Ok(());
    }
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|e| AppError::Config(format!("could not update launch at login: {e}")))
}

/// Point an existing login item at this executable, or remove it if the
/// setting is off.
///
/// `set_autostart` only runs when the setting changes, so an entry left
/// pointing at another binary (a dev build, an old install location) would
/// otherwise never be corrected. This only touches an entry that is already
/// live: on Windows `is_enabled` is also false when the user switched Klets
/// off in Task Manager's Startup tab, and `enable` would override that.
pub fn sync_autostart(app: &AppHandle, enabled: bool) {
    if tauri::is_dev() {
        return;
    }
    let manager = app.autolaunch();
    if !manager.is_enabled().unwrap_or(false) {
        return;
    }
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    if let Err(e) = result {
        tracing::warn!(target: "klets", "could not refresh launch at login: {e}");
    }
}
