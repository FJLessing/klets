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
    /// The seed prompt, so "reset to default" never has to duplicate it.
    pub default_system_prompt: &'static str,
}

fn provider_statuses(settings: &Settings) -> Vec<ProviderStatus> {
    providers::PROVIDERS
        .iter()
        .map(|spec| {
            let keys = providers::SavedKeys {
                api_key: settings::has_api_key(spec.key_id),
                subscription_token: spec
                    .subscription_key_id
                    .is_some_and(settings::has_api_key),
            };
            let auth_mode = settings.auth_mode_for(spec);
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
                has_api_key: keys.api_key,
                has_subscription_token: keys.subscription_token,
                authenticated: spec.is_authenticated(auth_mode, &keys),
                auth_mode,
                auth_modes: spec.auth_modes.to_vec(),
                subscription_key_id: spec.subscription_key_id.map(str::to_string),
                runs_via: spec.runs_via.map(str::to_string),
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
        default_system_prompt: settings::DEFAULT_SYSTEM_PROMPT,
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
        default_system_prompt: settings::DEFAULT_SYSTEM_PROMPT,
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
        default_system_prompt: settings::DEFAULT_SYSTEM_PROMPT,
    })
}

/// Switch a provider between its own login and a Klets-supplied API key.
#[tauri::command]
pub fn set_auth_mode(
    app: AppHandle,
    agent: State<'_, AgentManager>,
    provider_id: String,
    mode: settings::AuthMode,
) -> AppResult<AppSnapshot> {
    let spec = providers::find(&provider_id)
        .ok_or_else(|| AppError::UnknownProvider(provider_id.clone()))?;

    if !spec.auth_modes.contains(&mode) {
        return Err(AppError::Config(format!(
            "{} does not support that sign-in method",
            spec.name
        )));
    }

    let mut settings = settings::load(&app);
    let mut provider = settings.provider(&provider_id);
    provider.auth_mode = Some(mode);
    settings.providers.insert(provider_id, provider);
    settings::save(&app, &settings)?;

    // The running agent was launched with the old credentials.
    agent.stop();

    let providers = provider_statuses(&settings);
    Ok(AppSnapshot {
        providers,
        settings,
        agent_event: AGENT_EVENT,
        active_provider: None,
        default_system_prompt: settings::DEFAULT_SYSTEM_PROMPT,
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

/// Answer a pending tool request.
#[tauri::command]
pub fn respond_permission(agent: State<'_, AgentManager>, id: u64, allow: bool) {
    agent.broker().resolve(id, allow);
}

/// Choose the folder agents run in. An empty value restores the scratch
/// directory, which is the safe default.
#[tauri::command]
pub fn set_working_dir(
    app: AppHandle,
    agent: State<'_, AgentManager>,
    path: Option<String>,
) -> AppResult<AppSnapshot> {
    let cleaned = path.map(|p| p.trim().to_string()).filter(|p| !p.is_empty());

    if let Some(dir) = cleaned.as_ref() {
        if !std::path::Path::new(dir).is_dir() {
            return Err(AppError::Config(format!("'{dir}' is not a folder")));
        }
    }

    let mut settings = settings::load(&app);
    settings.working_dir = cleaned;
    settings::save(&app, &settings)?;

    // The running agent is rooted in the old directory.
    agent.stop();

    let providers = provider_statuses(&settings);
    Ok(AppSnapshot {
        providers,
        settings,
        agent_event: AGENT_EVENT,
        active_provider: None,
        default_system_prompt: settings::DEFAULT_SYSTEM_PROMPT,
    })
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

/// Smallest and largest launcher heights, in logical pixels. Both include the
/// transparent gutter the frontend leaves around the card for its shadow.
const MIN_LAUNCHER_HEIGHT: f64 = 96.0;
const MAX_LAUNCHER_HEIGHT: f64 = 700.0;

/// Resize the launcher to fit its content, within the configured bounds.
///
/// Only the height ever changes, and it is applied in physical pixels while the
/// existing width is passed through untouched. Converting the width to logical
/// units and back does not round-trip cleanly under fractional DPI scaling, so
/// doing that on every streamed token made the window twitch horizontally.
#[tauri::command]
pub fn resize_launcher(app: AppHandle, height: f64) {
    let Some(window) = windows::launcher(&app) else {
        return;
    };
    let (Ok(size), Ok(scale)) = (window.outer_size(), window.scale_factor()) else {
        return;
    };

    let clamped = height.clamp(MIN_LAUNCHER_HEIGHT, MAX_LAUNCHER_HEIGHT);
    let target = (clamped * scale).round() as u32;

    // Skip no-op resizes; the frontend measures far more often than the size
    // actually changes.
    if target == size.height {
        return;
    }

    let _ = window.set_size(tauri::PhysicalSize::new(size.width, target));
}

#[tauri::command]
pub fn quit(app: AppHandle, agent: State<'_, AgentManager>) {
    agent.stop();
    app.exit(0);
}
