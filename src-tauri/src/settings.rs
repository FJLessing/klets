use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};
use crate::providers;

const KEYRING_SERVICE: &str = "klets";
pub const DEFAULT_HOTKEY: &str = "Ctrl+Space";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderSettings {
    pub enabled: bool,
    /// Overrides the provider's default model when supported.
    pub model: Option<String>,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            model: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub hotkey: String,
    pub active_provider: String,
    pub hide_on_blur: bool,
    pub launch_at_login: bool,
    pub providers: HashMap<String, ProviderSettings>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HOTKEY.to_string(),
            active_provider: providers::PROVIDERS[0].id.to_string(),
            hide_on_blur: true,
            launch_at_login: false,
            providers: HashMap::new(),
        }
    }
}

impl Settings {
    pub fn provider(&self, id: &str) -> ProviderSettings {
        self.providers.get(id).cloned().unwrap_or_default()
    }

    /// Model for a provider: explicit override, else the registry default.
    pub fn model_for(&self, spec: &providers::ProviderSpec) -> Option<String> {
        self.provider(spec.id)
            .model
            .filter(|m| !m.trim().is_empty())
            .or_else(|| spec.default_model.map(str::to_string))
    }
}

fn settings_path(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| AppError::Config(format!("no config dir: {e}")))?;
    std::fs::create_dir_all(&dir)?;
    Ok(dir.join("settings.json"))
}

/// Scratch directory used as the ACP session cwd.
///
/// Agents are given this instead of a real project so a quick question can
/// never touch the user's files.
pub fn workspace_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Config(format!("no data dir: {e}")))?
        .join("workspace");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

pub fn load(app: &AppHandle) -> Settings {
    let Ok(path) = settings_path(app) else {
        return Settings::default();
    };
    let Ok(raw) = std::fs::read_to_string(&path) else {
        return Settings::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn save(app: &AppHandle, settings: &Settings) -> AppResult<()> {
    let path = settings_path(app)?;
    std::fs::write(path, serde_json::to_string_pretty(settings)?)?;
    Ok(())
}

fn entry(key_id: &str) -> AppResult<keyring::Entry> {
    Ok(keyring::Entry::new(KEYRING_SERVICE, key_id)?)
}

pub fn get_api_key(key_id: &str) -> Option<String> {
    entry(key_id)
        .ok()?
        .get_password()
        .ok()
        .filter(|k| !k.trim().is_empty())
}

pub fn set_api_key(key_id: &str, value: &str) -> AppResult<()> {
    let entry = entry(key_id)?;
    if value.trim().is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.into()),
        }
    } else {
        entry.set_password(value)?;
        Ok(())
    }
}

pub fn has_api_key(key_id: &str) -> bool {
    get_api_key(key_id).is_some()
}
