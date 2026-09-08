use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use crate::error::{AppError, AppResult};
use crate::permissions::ToolPolicy;
use crate::providers;

const KEYRING_SERVICE: &str = "klets";
pub const DEFAULT_HOTKEY: &str = "Ctrl+Space";

/// How an agent proves who it is.
///
/// This is chosen explicitly rather than inferred from whether a key happens
/// to be saved: injecting an API key at an agent that is signed in to a
/// subscription silently moves the user onto per-token billing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AuthMode {
    /// Use the agent's own login, or a long-lived subscription token.
    Subscription,
    /// Use an API key supplied by Klets.
    ApiKey,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderSettings {
    pub enabled: bool,
    /// Overrides the provider's default model when supported.
    pub model: Option<String>,
    /// `None` means "whatever the provider defaults to".
    pub auth_mode: Option<AuthMode>,
}

impl Default for ProviderSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            model: None,
            auth_mode: None,
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
    pub tool_policy: ToolPolicy,
    /// Instructions handed to every agent as its project instruction file.
    pub system_prompt: String,
    /// Directory agents run in. Empty means the managed scratch directory.
    pub working_dir: Option<String>,
    /// Clear the conversation once the launcher has been hidden for a while,
    /// so re-summoning it after a long gap never resumes a stale exchange.
    pub reset_when_hidden: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            hotkey: DEFAULT_HOTKEY.to_string(),
            active_provider: providers::PROVIDERS[0].id.to_string(),
            hide_on_blur: true,
            launch_at_login: false,
            providers: HashMap::new(),
            tool_policy: ToolPolicy::default(),
            system_prompt: DEFAULT_SYSTEM_PROMPT.to_string(),
            working_dir: None,
            reset_when_hidden: true,
        }
    }
}

/// Seed persona for the prompt editor.
///
/// Written to the session directory as the agent's project instructions, so it
/// has to read as guidance to an agent rather than configuration.
pub const DEFAULT_SYSTEM_PROMPT: &str = "\
You are answering a quick question from a launcher bar, not working in a repo.

- Be brief. Lead with the answer, then only the context needed to trust it.
- Prefer prose over code. Short fragments are fine when they genuinely help;
  do not write complete files or drop-in implementations.
- Use read-only tools freely to check facts before answering. Accuracy matters
  more than speed.
- Never edit, create or delete files. You do not have permission, and should
  not try to work around that.
- Give full URLs on their own line so they can be clicked.
- If a question is ambiguous, ask instead of guessing.
";

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

    /// Auth mode for a provider, ignoring a stored mode the provider no longer
    /// supports so a registry change can never strand a saved setting.
    pub fn auth_mode_for(&self, spec: &providers::ProviderSpec) -> AuthMode {
        self.provider(spec.id)
            .auth_mode
            .filter(|mode| spec.auth_modes.contains(mode))
            .unwrap_or(spec.default_auth_mode())
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

/// Klets-owned scratch directory, used as the session cwd by default.
///
/// Keeping agents here by default means a quick question starts with no access
/// to real files at all; pointing `working_dir` somewhere else is a deliberate
/// choice made in settings.
pub fn workspace_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Config(format!("no data dir: {e}")))?
        .join("workspace");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// Directory Klets writes its own files into, never the session directory.
///
/// Once the user points sessions at a real folder, writing instruction files
/// into it would collide with their own `AGENTS.md` or `CLAUDE.md`.
pub fn managed_dir(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = app
        .path()
        .app_data_dir()
        .map_err(|e| AppError::Config(format!("no data dir: {e}")))?
        .join("managed");
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

impl Settings {
    /// Where agents should run, falling back to the scratch directory when the
    /// configured folder is unset or has gone missing.
    pub fn session_dir(&self, app: &AppHandle) -> AppResult<PathBuf> {
        match resolve_working_dir(&self.working_dir) {
            Some(path) => Ok(path),
            None => workspace_dir(app),
        }
    }
}

/// The configured directory, if it is set and still exists.
///
/// Split out from [`Settings::session_dir`] so the fallback rule is testable
/// without an `AppHandle` — the only reason that method needs one is to build
/// the scratch-directory path when this returns `None`.
fn resolve_working_dir(working_dir: &Option<String>) -> Option<PathBuf> {
    let configured = working_dir.as_ref().filter(|d| !d.trim().is_empty())?;
    let path = PathBuf::from(configured);
    path.is_dir().then_some(path)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A settings.json from before tool policy, the system prompt and the
    /// working directory existed. Loading it must fill in safe defaults for
    /// the new fields without disturbing anything the user actually set —
    /// this is the exact file an existing install has on disk today.
    const PRE_TOOLS_SETTINGS: &str = r#"{
        "hotkey": "Ctrl+Space",
        "activeProvider": "claude",
        "hideOnBlur": true,
        "launchAtLogin": true,
        "providers": {}
    }"#;

    #[test]
    fn old_settings_file_gets_safe_new_defaults() {
        let settings: Settings =
            serde_json::from_str(PRE_TOOLS_SETTINGS).expect("old settings.json must still parse");

        // What the user actually configured is untouched.
        assert_eq!(settings.hotkey, "Ctrl+Space");
        assert_eq!(settings.active_provider, "claude");
        assert!(settings.hide_on_blur);
        assert!(settings.launch_at_login);

        // Fields the file predates land on the same defaults a fresh install
        // gets — never on tool access being silently more permissive.
        assert_eq!(settings.tool_policy, ToolPolicy::AskToRun);
        assert_eq!(settings.system_prompt, DEFAULT_SYSTEM_PROMPT);
        assert_eq!(settings.working_dir, None);
        assert!(settings.reset_when_hidden);
    }

    #[test]
    fn missing_provider_entry_falls_back_to_provider_default_auth_mode() {
        let settings = Settings::default();
        for spec in providers::PROVIDERS {
            assert_eq!(settings.auth_mode_for(spec), spec.default_auth_mode());
        }
    }

    #[test]
    fn stored_auth_mode_unsupported_by_the_registry_is_ignored() {
        // Simulates a provider losing a mode in an update: a saved
        // `AuthMode::Subscription` must not be trusted for a provider whose
        // registry entry no longer lists it.
        let mut settings = Settings::default();
        settings.providers.insert(
            "gemini".to_string(),
            ProviderSettings {
                enabled: true,
                model: None,
                auth_mode: Some(AuthMode::Subscription),
            },
        );

        let gemini = providers::find("gemini").expect("gemini is registered");
        assert_eq!(gemini.auth_modes, &[AuthMode::ApiKey]);
        assert_eq!(settings.auth_mode_for(gemini), AuthMode::ApiKey);
    }

    #[test]
    fn working_dir_resolution_falls_back_on_a_missing_folder() {
        let missing = Some(r"A:\this\path\does\not\exist\klets-test".to_string());
        assert_eq!(resolve_working_dir(&missing), None);
    }

    #[test]
    fn working_dir_resolution_falls_back_on_blank_or_unset() {
        assert_eq!(resolve_working_dir(&None), None);
        assert_eq!(resolve_working_dir(&Some("   ".to_string())), None);
    }

    #[test]
    fn working_dir_resolution_accepts_a_real_folder() {
        let cwd = std::env::current_dir().expect("cwd");
        let resolved = resolve_working_dir(&Some(cwd.to_string_lossy().to_string()));
        assert_eq!(resolved, Some(cwd));
    }

    #[test]
    fn model_for_prefers_an_explicit_override_over_the_registry_default() {
        let mut settings = Settings::default();
        settings.providers.insert(
            "opencode-go".to_string(),
            ProviderSettings {
                enabled: true,
                model: Some("opencode-go/glm-5.3-flash".to_string()),
                auth_mode: None,
            },
        );

        let spec = providers::find("opencode-go").expect("registered");
        assert_eq!(settings.model_for(spec), Some("opencode-go/glm-5.3-flash".to_string()));
    }

    #[test]
    fn model_for_falls_back_to_the_registry_default_when_override_is_blank() {
        let mut settings = Settings::default();
        settings.providers.insert(
            "opencode-go".to_string(),
            ProviderSettings {
                enabled: true,
                model: Some("   ".to_string()),
                auth_mode: None,
            },
        );

        let spec = providers::find("opencode-go").expect("registered");
        assert_eq!(settings.model_for(spec), spec.default_model.map(str::to_string));
    }

    #[test]
    fn model_for_is_none_when_neither_an_override_nor_a_default_exists() {
        // Claude and Codex expose model selection through the settings text
        // field but have no registry default of their own.
        let settings = Settings::default();
        let spec = providers::find("claude").expect("registered");
        assert_eq!(settings.model_for(spec), None);
    }

    /// These strings are a contract with the frontend (`src/helpers/types.ts`),
    /// not an implementation detail — a rename here without a matching
    /// `ProviderSettings.authMode` / `Settings.toolPolicy` change on the
    /// TypeScript side would desync silently, since both ends just serialize
    /// through untyped JSON at the IPC boundary.
    #[test]
    fn auth_mode_serializes_to_the_exact_strings_the_frontend_expects() {
        assert_eq!(serde_json::to_string(&AuthMode::Subscription).unwrap(), r#""subscription""#);
        assert_eq!(serde_json::to_string(&AuthMode::ApiKey).unwrap(), r#""apiKey""#);
    }

    #[test]
    fn tool_policy_serializes_to_the_exact_strings_the_frontend_expects() {
        assert_eq!(serde_json::to_string(&ToolPolicy::Off).unwrap(), r#""off""#);
        assert_eq!(serde_json::to_string(&ToolPolicy::ReadOnly).unwrap(), r#""readOnly""#);
        assert_eq!(serde_json::to_string(&ToolPolicy::AskToRun).unwrap(), r#""askToRun""#);
    }
}
