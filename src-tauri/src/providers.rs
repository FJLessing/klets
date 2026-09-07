use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

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
        login_hint: "claude login",
        credential_paths: &[".claude/.credentials.json", ".claude.json"],
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
    },
    ProviderSpec {
        id: "gemini",
        name: "Gemini",
        command: "gemini",
        args: &["--experimental-acp"],
        key_id: "gemini",
        key_label: "Gemini API key",
        env_var: "GEMINI_API_KEY",
        install_hint: "npm install -g @google/gemini-cli",
        key_url: "https://aistudio.google.com/apikey",
        default_model: None,
        login_hint: "gemini (then choose Login with Google)",
        credential_paths: &[".gemini/oauth_creds.json", ".gemini/google_accounts.json"],
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
    /// A saved key, an inherited env var, or the agent's own login.
    pub authenticated: bool,
    pub model: Option<String>,
}

impl ProviderSpec {
    /// Whether the agent can authenticate without asking the user for anything.
    ///
    /// Klets checks three sources, cheapest first, and never spawns the agent
    /// to find out: a key in the OS keychain, an API key already exported in
    /// the environment (which the child process inherits), or the credential
    /// file the agent's own `login` flow writes.
    pub fn is_authenticated(&self, has_saved_key: bool) -> bool {
        if has_saved_key {
            return true;
        }

        if std::env::var(self.env_var).is_ok_and(|v| !v.trim().is_empty()) {
            return true;
        }

        let Some(home) = home_dir() else {
            return false;
        };

        self.credential_paths
            .iter()
            .any(|relative| home.join(relative).exists())
    }
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
    let extensions = executable_extensions();

    for dir in std::env::split_paths(&path_var) {
        for ext in &extensions {
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
