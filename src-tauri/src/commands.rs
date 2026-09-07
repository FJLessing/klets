use tauri::{AppHandle, Manager, State};

use crate::agent::AgentManager;
use crate::error::{AppError, AppResult};
use crate::events::AGENT_EVENT;
use crate::providers::{self, ProviderStatus};
use crate::settings::{self, Settings};
use crate::shortcuts;
use crate::windows;

/// Everything the launcher needs on open: providers, readiness, and the hotkey.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSnapshot {
    pub settings: Settings,
    pub providers: Vec<ProviderStatus>,
    pub agent_event: &'static str,
    pub active_provider: Option<String>,
}

fn provider_statuses(settings: &Settings) -> Vec<ProviderStatus> {
    providers::PROVIDERS
        .iter()
        .map(|spec| {
            let has_api_key = settings::has_api_key(spec.key_id);
            ProviderStatus {
                id: spec.id.to_string(),
                name: spec.name.to_string(),
                command: spec.command.to_string(),
                key_id: spec.key_id.to_string(),
                key_label: spec.key_label.to_string(),
                install_hint: spec.install_hint.to_string(),
                key_url: spec.key_url.to_string(),
                login_hint: spec.login_hint.to_string(),
                default_model: spec.default_model.map(str::to_string),
                binary_path: providers::resolve_binary(spec.command)
                    .map(|p| p.to_string_lossy().to_string()),
                has_api_key,
                authenticated: spec.is_authenticated(has_api_key),
                model: settings.model_for(spec),
            }
        })
        .collect()
}

/// A provider Klets can actually use right now: installed and able to sign in.
fn is_usable(status: &ProviderStatus) -> bool {
    status.binary_path.is_some() && status.authenticated
}

/// Keep the active provider pointed at something that works.
///
/// The registry order is only a starting preference — on a given machine the
/// first few entries are often not installed, so the first usable provider
/// wins and the choice is persisted.
fn ensure_active_provider(
    app: &AppHandle,
    settings: &mut Settings,
    statuses: &[ProviderStatus],
) {
    let active_is_usable = statuses
        .iter()
        .find(|s| s.id == settings.active_provider)
        .is_some_and(is_usable);

    if active_is_usable {
        return;
    }

    let fallback = statuses
        .iter()
        .find(|s| is_usable(s))
        .or_else(|| statuses.iter().find(|s| s.binary_path.is_some()));

    if let Some(next) = fallback {
        if next.id != settings.active_provider {
            settings.active_provider = next.id.clone();
            let _ = settings::save(app, settings);
        }
    }
}

#[tauri::command]
pub fn get_snapshot(app: AppHandle, agent: State<'_, AgentManager>) -> AppSnapshot {
    let mut settings = settings::load(&app);
    let providers = provider_statuses(&settings);
    ensure_active_provider(&app, &mut settings, &providers);

    AppSnapshot {
        providers,
        settings,
        agent_event: AGENT_EVENT,
        active_provider: agent.active_provider(),
    }
}

#[tauri::command]
pub fn save_settings(
    app: AppHandle,
    agent: State<'_, AgentManager>,
    settings: Settings,
) -> AppResult<AppSnapshot> {
    let previous = settings::load(&app);
    settings::save(&app, &settings)?;

    if previous.hotkey != settings.hotkey {
        shortcuts::rebind(&app, &previous.hotkey, &settings.hotkey)?;
    }

    if previous.launch_at_login != settings.launch_at_login {
        shortcuts::set_autostart(&app, settings.launch_at_login)?;
    }

    // A model or provider change means the running agent no longer matches.
    agent.stop();

    Ok(AppSnapshot {
        providers: provider_statuses(&settings),
        settings,
        agent_event: AGENT_EVENT,
        active_provider: None,
    })
}

#[tauri::command]
pub fn set_api_key(app: AppHandle, key_id: String, value: String) -> AppResult<AppSnapshot> {
    settings::set_api_key(&key_id, &value)?;
    let settings = settings::load(&app);
    Ok(AppSnapshot {
        providers: provider_statuses(&settings),
        settings,
        agent_event: AGENT_EVENT,
        active_provider: None,
    })
}

#[tauri::command]
pub fn set_active_provider(app: AppHandle, provider_id: String) -> AppResult<()> {
    if providers::find(&provider_id).is_none() {
        return Err(AppError::UnknownProvider(provider_id));
    }
    let mut settings = settings::load(&app);
    settings.active_provider = provider_id;
    settings::save(&app, &settings)
}

#[tauri::command]
pub fn send_prompt(
    app: AppHandle,
    agent: State<'_, AgentManager>,
    text: String,
) -> AppResult<u64> {
    let text = text.trim().to_string();
    if text.is_empty() {
        return Err(AppError::Agent("nothing to ask".into()));
    }

    let settings = settings::load(&app);
    let provider_id = settings.active_provider.clone();
    agent.prompt(&app, &settings, &provider_id, text)
}

#[tauri::command]
pub fn cancel_turn(agent: State<'_, AgentManager>) {
    agent.cancel();
}

#[tauri::command]
pub fn new_chat(agent: State<'_, AgentManager>) -> u64 {
    agent.new_session()
}

#[tauri::command]
pub fn hide_launcher(app: AppHandle) {
    windows::hide_launcher(&app);
}

#[tauri::command]
pub fn open_settings(app: AppHandle) {
    windows::open_settings(&app);
}

#[tauri::command]
pub fn close_settings(app: AppHandle) {
    if let Some(window) = app.get_webview_window(windows::SETTINGS) {
        let _ = window.hide();
    }
}

/// Resize the launcher to fit its content, within the configured bounds.
#[tauri::command]
pub fn resize_launcher(app: AppHandle, height: f64) {
    let Some(window) = windows::launcher(&app) else {
        return;
    };
    let Ok(size) = window.inner_size() else {
        return;
    };
    let Ok(scale) = window.scale_factor() else {
        return;
    };

    let width = size.width as f64 / scale;
    let height = height.clamp(72.0, 620.0);
    let _ = window.set_size(tauri::LogicalSize::new(width, height));
}

#[tauri::command]
pub fn quit(app: AppHandle, agent: State<'_, AgentManager>) {
    agent.stop();
    app.exit(0);
}
