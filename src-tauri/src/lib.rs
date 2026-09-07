pub mod agent;
mod commands;
pub mod error;
pub mod events;
pub mod providers;
pub mod settings;
mod shortcuts;
mod windows;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{Manager, WindowEvent};

use agent::AgentManager;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "klets=info".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::new().build())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            windows::show_launcher(app);
        }))
        .manage(AgentManager::new())
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::save_settings,
            commands::set_api_key,
            commands::set_active_provider,
            commands::send_prompt,
            commands::cancel_turn,
            commands::new_chat,
            commands::hide_launcher,
            commands::open_settings,
            commands::close_settings,
            commands::resize_launcher,
            commands::quit,
        ])
        .setup(|app| {
            let handle = app.handle().clone();
            let stored = settings::load(&handle);

            // Fall back to the default hotkey if the saved one is unavailable,
            // so the launcher is always reachable.
            if let Err(e) = shortcuts::register(&handle, &stored.hotkey) {
                tracing::warn!(target: "klets", "{e}");
                if stored.hotkey != settings::DEFAULT_HOTKEY {
                    let _ = shortcuts::register(&handle, settings::DEFAULT_HOTKEY);
                }
            }

            build_tray(&handle)?;

            // Only interrupt with settings when nothing can answer a question:
            // an agent counts as usable if it is installed and can sign in,
            // whether that is a saved key or its own CLI login.
            let usable = providers::PROVIDERS.iter().any(|spec| {
                providers::resolve_binary(spec.command).is_some()
                    && spec.is_authenticated(settings::has_api_key(spec.key_id))
            });
            if usable {
                windows::show_launcher(&handle);
            } else {
                windows::open_settings(&handle);
            }

            Ok(())
        })
        .on_window_event(|window, event| {
            let app = window.app_handle();

            match event {
                // The launcher is a transient surface: dismiss it instead of
                // closing, and let the settings window hide too.
                WindowEvent::CloseRequested { api, .. } => {
                    api.prevent_close();
                    let _ = window.hide();
                }
                WindowEvent::Focused(false)
                    if window.label() == windows::LAUNCHER
                        && settings::load(app).hide_on_blur =>
                {
                    let _ = window.hide();
                }
                _ => {}
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running klets");
}

fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Klets", true, None::<&str>)?;
    let settings_item = MenuItem::with_id(app, "settings", "Settings", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &settings_item, &quit])?;

    TrayIconBuilder::with_id("klets-tray")
        .icon(app.default_window_icon().cloned().unwrap())
        .tooltip("Klets")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => windows::show_launcher(app),
            "settings" => windows::open_settings(app),
            "quit" => {
                app.state::<AgentManager>().stop();
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                windows::toggle_launcher(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}
