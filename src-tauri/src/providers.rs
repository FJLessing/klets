use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::settings::AuthMode;

/// A launchable ACP agent, described declaratively.
///
/// Every provider is spawned as a subprocess that speaks ACP (JSON-RPC 2.0 over
/// stdio). The only differences between them are the binary, the arguments, the
/// environment variable carrying the API key, and the default model.
#[derive(Debug, Clone, Copy)]
pub struct ProviderSpec {
    /// Stable identifier used in settings and IPC.
    pub id: &'static str,
    /// Human readable name shown in the launcher.
    pub name: &'static str,
    /// Executable to spawn. Resolved against PATH at runtime.
    pub command: &'static str,
    /// Arguments that put the binary into ACP mode.
    pub args: &'static [&'static str],
    /// Keychain entry name. Providers may share one (OpenCode Go and Zen do).
    pub key_id: &'static str,
    /// Label for the shared key, shown in settings.
    pub key_label: &'static str,
    /// Environment variable the agent reads the key from.
    pub env_var: &'static str,
    /// Shown when the binary is missing.
    pub install_hint: &'static str,
    /// Where the user gets an API key.
    pub key_url: &'static str,
    /// Model requested after the session opens, when the agent supports it.
    pub default_model: Option<&'static str>,
    /// How the user signs in when no API key is saved.
    pub login_hint: &'static str,
    /// Credential files written by the agent's own login flow, relative to the
    /// home directory. Their presence means the CLI can authenticate itself.
    pub credential_paths: &'static [&'static str],
    /// Supported auth modes, most preferred first.
    pub auth_modes: &'static [AuthMode],
    /// Env var carrying a long-lived subscription token, where the agent
    /// offers one (Claude Code's `setup-token`, for example).
    pub subscription_env_var: Option<&'static str>,
    /// Keychain entry holding that subscription token.
    pub subscription_key_id: Option<&'static str>,
    /// Set when the provider is really another agent under the hood, so the
    /// settings window can say so.
    pub runs_via: Option<&'static str>,
    /// Flag used to pick the model on the command line, for agents that do not
    /// expose model selection over ACP.
    pub model_flag: Option<&'static str>,
    /// Config that has to be forced on an agent that pins its own auth method.
    pub auth_settings_override: Option<AuthSettingsOverride>,
}

/// Agents whose sign-in method lives in a config file with no CLI override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthSettingsOverride {
    /// gemini-cli stores `security.auth.selectedType` in its settings and has
    /// no flag to change it. Klets writes a private settings file and points
    /// the process at it, leaving the user's own config untouched.
    GeminiCli,
}

pub const PROVIDERS: &[ProviderSpec] = &[
    ProviderSpec {
        id: "claude",
        name: "Claude",
        command: "claude-agent-acp",
        args: &[],
        key_id: "anthropic",
        key_label: "Anthropic API key",
        env_var: "ANTHROPIC_API_KEY",
        install_hint: "npm install -g @agentclientprotocol/claude-agent-acp",
        key_url: "https://console.anthropic.com/settings/keys",
        default_model: None,
        login_hint: "claude auth login",
        credential_paths: &[".claude/.credentials.json"],
        auth_modes: &[AuthMode::Subscription, AuthMode::ApiKey],
        // `claude setup-token` mints a long-lived token for a subscription.
        subscription_env_var: Some("CLAUDE_CODE_OAUTH_TOKEN"),
        subscription_key_id: Some("claude-oauth"),
        runs_via: None,
        model_flag: None,
        auth_settings_override: None,
    },
    ProviderSpec {
        id: "codex",
        name: "Codex",
        command: "codex-acp",
        args: &[],
        key_id: "openai",
        key_label: "OpenAI API key",
        env_var: "OPENAI_API_KEY",
        install_hint: "npm install -g @agentclientprotocol/codex-acp",
        key_url: "https://platform.openai.com/api-keys",
        default_model: None,
        login_hint: "codex login",
        credential_paths: &[".codex/auth.json"],
        auth_modes: &[AuthMode::Subscription, AuthMode::ApiKey],
        subscription_env_var: None,
        subscription_key_id: None,
        runs_via: None,
        model_flag: None,
        auth_settings_override: None,
    },
    // gemini-cli talks to Google directly, so it can reach current models.
    // Its default `oauth-personal` mode authenticates against Gemini Code
    // Assist for individuals, which Google discontinued, and there is no flag
    // to switch to an API key — hence the settings override below.
    ProviderSpec {
        id: "gemini",
        name: "Gemini",
        command: "gemini",
        args: &["--acp"],
        key_id: "gemini",
        key_label: "Gemini API key",
        env_var: "GEMINI_API_KEY",
        install_hint: "npm install -g @google/gemini-cli",
        key_url: "https://aistudio.google.com/apikey",
        default_model: Some("gemini-3.7-flash"),
        login_hint: "paste a Gemini API key below",
        credential_paths: &[],
        auth_modes: &[AuthMode::ApiKey],
        subscription_env_var: None,
        subscription_key_id: None,
        runs_via: None,
        model_flag: Some("-m"),
        auth_settings_override: Some(AuthSettingsOverride::GeminiCli),
    },
    ProviderSpec {
        id: "opencode-go",
        name: "OpenCode Go",
        command: "opencode",
        args: &["acp"],
        key_id: "opencode",
        key_label: "OpenCode API key (Go and Zen share one key)",
        env_var: "OPENCODE_API_KEY",
        install_hint: "curl -fsSL https://opencode.ai/install | bash  (or: npm install -g opencode-ai)",
        key_url: "https://opencode.ai/go",
        default_model: Some("opencode-go/kimi-k3"),
        login_hint: "opencode auth login",
        credential_paths: &[".local/share/opencode/auth.json", ".config/opencode/auth.json"],
        auth_modes: &[AuthMode::Subscription, AuthMode::ApiKey],
        subscription_env_var: None,
        subscription_key_id: None,
        runs_via: None,
        model_flag: None,
        auth_settings_override: None,
    },
    ProviderSpec {
        id: "opencode-zen",
        name: "OpenCode Zen",
        command: "opencode",
        args: &["acp"],
        key_id: "opencode",
        key_label: "OpenCode API key (Go and Zen share one key)",
        env_var: "OPENCODE_API_KEY",
        install_hint: "curl -fsSL https://opencode.ai/install | bash  (or: npm install -g opencode-ai)",
        key_url: "https://opencode.ai/zen",
        default_model: Some("opencode/big-pickle"),
        login_hint: "opencode auth login",
        credential_paths: &[".local/share/opencode/auth.json", ".config/opencode/auth.json"],
        auth_modes: &[AuthMode::Subscription, AuthMode::ApiKey],
        subscription_env_var: None,
        subscription_key_id: None,
        runs_via: None,
        model_flag: None,
        auth_settings_override: None,
    },
];

pub fn find(id: &str) -> Option<&'static ProviderSpec> {
    PROVIDERS.iter().find(|p| p.id == id)
}

/// What the settings window shows for each provider row.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderStatus {
    pub id: String,
    pub name: String,
    pub command: String,
    pub key_id: String,
    pub key_label: String,
    pub install_hint: String,
    pub key_url: String,
    pub login_hint: String,
    pub default_model: Option<String>,
    /// Absolute path of the resolved binary, when found.
    pub binary_path: Option<String>,
    pub has_api_key: bool,
    pub has_subscription_token: bool,
    /// Whether the provider can authenticate in its selected mode.
    pub authenticated: bool,
    pub auth_mode: AuthMode,
    pub auth_modes: Vec<AuthMode>,
    pub subscription_key_id: Option<String>,
    /// The agent this provider actually runs on, when it isn't its own.
    pub runs_via: Option<String>,
    pub model: Option<String>,
}

impl ProviderSpec {
    pub fn default_auth_mode(&self) -> AuthMode {
        self.auth_modes.first().copied().unwrap_or(AuthMode::ApiKey)
    }

    /// Whether the agent can authenticate in the given mode without asking the
    /// user for anything.
    ///
    /// Checked cheaply and without ever spawning the agent. Note this cannot
    /// prove a credential is still *valid* — a stale login still looks signed
    /// in — so a session that fails on auth explains how to recover.
    pub fn is_authenticated(&self, mode: AuthMode, keys: &SavedKeys) -> bool {
        match mode {
            AuthMode::ApiKey => keys.api_key || env_var_set(self.env_var),
            AuthMode::Subscription => {
                if keys.subscription_token {
                    return true;
                }
                if self.subscription_env_var.is_some_and(env_var_set) {
                    return true;
                }
                home_dir().is_some_and(|home| {
                    self.credential_paths
                        .iter()
                        .any(|relative| home.join(relative).exists())
                })
            }
        }
    }
}

/// Which secrets Klets holds for a provider, so lookups happen once.
#[derive(Debug, Clone, Copy, Default)]
pub struct SavedKeys {
    pub api_key: bool,
    pub subscription_token: bool,
}

fn env_var_set(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| !value.trim().is_empty())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
}

/// Resolve a command against PATH, honouring Windows executable extensions.
///
/// npm installs global binaries as `.cmd` shims on Windows, so a bare
/// `claude-agent-acp` never resolves without walking PATHEXT.
pub fn resolve_binary(command: &str) -> Option<PathBuf> {
    let candidate = Path::new(command);
    if candidate.is_absolute() {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }

    let path_var = std::env::var_os("PATH")?;
    find_on_path(command, std::env::split_paths(&path_var), &executable_extensions())
}

/// The directory-walking part of [`resolve_binary`], taking PATH as an
/// argument instead of reading the environment.
///
/// Split out so the PATHEXT/`.cmd`-shim search order is unit-testable against
/// a real temp directory rather than the process's actual PATH, which tests
/// cannot safely rewrite (it is shared, mutable, global state).
fn find_on_path(
    command: &str,
    dirs: impl Iterator<Item = PathBuf>,
    extensions: &[String],
) -> Option<PathBuf> {
    for dir in dirs {
        for ext in extensions {
            let mut file = dir.join(command);
            if !ext.is_empty() {
                file.set_file_name(format!("{command}{ext}"));
            }
            if file.is_file() {
                return Some(file);
            }
        }
    }

    None
}

#[cfg(windows)]
fn executable_extensions() -> Vec<String> {
    let raw = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string());
    parse_pathext(&raw)
}

/// Turn a `PATHEXT` value into the ordered list of extensions to try,
/// including the bare (extension-less) name last.
///
/// A free function taking the raw string so the parsing rules are testable
/// without reading or mutating the process's actual `PATHEXT`.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_pathext(raw: &str) -> Vec<String> {
    let mut extensions: Vec<String> = raw
        .split(';')
        .filter(|e| !e.is_empty())
        .map(|e| e.to_lowercase())
        .collect();
    // Also try the bare name, for extension-less binaries on PATH.
    extensions.push(String::new());
    extensions
}

#[cfg(not(windows))]
fn executable_extensions() -> Vec<String> {
    vec![String::new()]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A provider fixture entirely separate from the real registry, so tests
    /// are deterministic regardless of what's actually installed or exported
    /// in the environment on the machine running them.
    const TEST_SPEC: ProviderSpec = ProviderSpec {
        id: "test",
        name: "Test Provider",
        command: "test-agent",
        args: &[],
        key_id: "test",
        key_label: "Test key",
        env_var: "KLETS_TEST_UNSET_VAR_9f3a1",
        install_hint: "",
        key_url: "",
        default_model: None,
        login_hint: "",
        credential_paths: &[],
        auth_modes: &[AuthMode::Subscription, AuthMode::ApiKey],
        subscription_env_var: None,
        subscription_key_id: None,
        runs_via: None,
        model_flag: None,
        auth_settings_override: None,
    };

    // --- find_on_path / PATHEXT resolution -----------------------------
    //
    // This is the exact mechanism behind a real bug: npm installs global
    // binaries as `.cmd` shims on Windows, and some agents (gemini-cli) ship
    // a same-named `.ps1` beside it that must NOT be picked, since `CreateProcess`
    // cannot execute a PowerShell script directly.

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("klets-test-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }

        fn touch(&self, name: &str) -> PathBuf {
            let path = self.0.join(name);
            std::fs::write(&path, b"").expect("write fixture file");
            path
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn find_on_path_picks_the_cmd_shim_over_a_same_named_ps1() {
        let dir = TempDir::new("cmd-shim");
        dir.touch("gemini.cmd");
        dir.touch("gemini.ps1");

        // PATHEXT never includes .ps1, matching the real default on Windows.
        let extensions = parse_pathext(".COM;.EXE;.BAT;.CMD");
        let found = find_on_path("gemini", std::iter::once(dir.0.clone()), &extensions)
            .expect("gemini.cmd should resolve");

        assert_eq!(found.extension().and_then(|e| e.to_str()), Some("cmd"));
    }

    #[test]
    fn find_on_path_returns_none_when_nothing_matches() {
        let dir = TempDir::new("empty");
        let extensions = parse_pathext(".EXE;.CMD");
        assert_eq!(find_on_path("nope", std::iter::once(dir.0.clone()), &extensions), None);
    }

    #[test]
    fn find_on_path_checks_later_path_entries() {
        let empty = TempDir::new("path-empty");
        let has_it = TempDir::new("path-has-it");
        has_it.touch("tool.exe");

        let extensions = parse_pathext(".EXE");
        let dirs = vec![empty.0.clone(), has_it.0.clone()].into_iter();
        let found = find_on_path("tool", dirs, &extensions).expect("found in second dir");

        assert_eq!(found, has_it.0.join("tool.exe"));
    }

    #[test]
    fn parse_pathext_lowercases_and_appends_the_bare_name() {
        assert_eq!(
            parse_pathext(".COM;.EXE;.BAT;.CMD"),
            vec![".com", ".exe", ".bat", ".cmd", ""],
        );
    }

    #[test]
    fn parse_pathext_of_an_empty_string_still_tries_the_bare_name() {
        assert_eq!(parse_pathext(""), vec![""]);
    }

    #[test]
    fn resolve_binary_absolute_path_checks_existence_directly() {
        let dir = TempDir::new("absolute");
        let file = dir.touch("exists.exe");

        assert_eq!(resolve_binary(file.to_str().unwrap()), Some(file));
        assert_eq!(
            resolve_binary(dir.0.join("missing.exe").to_str().unwrap()),
            None,
        );
    }

    // --- authentication ---------------------------------------------------

    #[test]
    fn is_authenticated_true_with_a_saved_api_key() {
        let keys = SavedKeys { api_key: true, subscription_token: false };
        assert!(TEST_SPEC.is_authenticated(AuthMode::ApiKey, &keys));
    }

    #[test]
    fn is_authenticated_true_with_a_saved_subscription_token() {
        let keys = SavedKeys { api_key: false, subscription_token: true };
        assert!(TEST_SPEC.is_authenticated(AuthMode::Subscription, &keys));
    }

    #[test]
    fn is_authenticated_false_with_nothing_saved_and_no_credential_files() {
        let keys = SavedKeys::default();
        assert!(!TEST_SPEC.is_authenticated(AuthMode::ApiKey, &keys));
        assert!(!TEST_SPEC.is_authenticated(AuthMode::Subscription, &keys));
    }

    #[test]
    fn default_auth_mode_matches_the_intended_registry_design() {
        // Claude, Codex and OpenCode default to the user's own login; only
        // Gemini is API-key only. A registry edit that silently reorders
        // `auth_modes` would change what's offered by default, so this pins
        // the intended shape down.
        let expected: &[(&str, AuthMode)] = &[
            ("claude", AuthMode::Subscription),
            ("codex", AuthMode::Subscription),
            ("gemini", AuthMode::ApiKey),
            ("opencode-go", AuthMode::Subscription),
            ("opencode-zen", AuthMode::Subscription),
        ];

        for (id, mode) in expected {
            let spec = find(id).unwrap_or_else(|| panic!("provider '{id}' is registered"));
            assert_eq!(spec.default_auth_mode(), *mode, "provider '{id}'");
        }
    }

    #[test]
    fn gemini_only_supports_api_key_auth() {
        // Gemini has no subscription route — offering "Subscription" in the
        // settings toggle for it would be a dead end.
        let gemini = find("gemini").expect("gemini is registered");
        assert_eq!(gemini.auth_modes, &[AuthMode::ApiKey]);
    }
}
