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

pub fn set_autostart(app: &AppHandle, enabled: bool) -> AppResult<()> {
    let manager = app.autolaunch();
    let result = if enabled {
        manager.enable()
    } else {
        manager.disable()
    };
    result.map_err(|e| AppError::Config(format!("could not update launch at login: {e}")))
}
