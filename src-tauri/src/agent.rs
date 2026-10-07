//! Owns the ACP agent subprocess and the connection to it.
//!
//! The `agent-client-protocol` client connection is `!Send` (it is built on
//! `LocalBoxFuture`), so it cannot live in Tauri's shared state directly.
//! Instead each agent runs on its own thread with a current-thread Tokio
//! runtime and a `LocalSet`; the rest of the app talks to it over an mpsc
//! channel of [`AgentCommand`].

use std::cell::Cell;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use agent_client_protocol::{
    Agent as _, CancelNotification, Client, ClientSideConnection, ContentBlock, Error as AcpError,
    InitializeRequest, ModelId, NewSessionRequest, PromptRequest, ProtocolVersion,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome,
    SessionConfigKind, SessionConfigOptionCategory, SessionConfigValueId, SessionNotification,
    SessionUpdate, SetSessionConfigOptionRequest, SetSessionModelRequest, TextContent,
};
use tauri::AppHandle;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::error::{AppError, AppResult};
use crate::events::{AgentCommandInfo, AgentEvent, AgentState, EventSink};
use crate::permissions::{self, PermissionBroker, ToolPolicy};
use crate::providers::{self, ProviderSpec};
use crate::settings::{self, AuthMode, Settings};

/// Messages sent from Tauri commands to the agent thread.
#[derive(Debug)]
pub enum AgentCommand {
    Prompt { turn: u64, text: String },
    Cancel,
    NewSession { turn: u64 },
    Shutdown,
}

/// How the agent process should be launched.
#[derive(Debug, Clone)]
pub struct LaunchPlan {
    pub provider_id: String,
    /// Display name used in error messages.
    pub provider_name: String,
    pub binary: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Inherited variables to clear before spawning.
    pub unset_env: Vec<String>,
    pub cwd: PathBuf,
    pub model: Option<String>,
    /// What the agent is allowed to do during the session.
    pub tool_policy: ToolPolicy,
    /// Instructions written into the session directory before the handshake.
    pub system_prompt: String,
    /// False when the model was already fixed on the command line.
    pub select_model_over_acp: bool,
    /// Command that signs the user in, quoted back when auth fails.
    pub login_hint: String,
}

impl LaunchPlan {
    /// Resolve everything needed to launch a provider, or explain what's missing.
    pub fn build(app: &AppHandle, spec: &ProviderSpec, settings: &Settings) -> AppResult<Self> {
        let binary = providers::resolve_binary(spec.command).ok_or_else(|| {
            AppError::MissingBinary {
                command: spec.command.to_string(),
                hint: format!("Install it with: {}", spec.install_hint),
            }
        })?;

        let cwd = settings.session_dir(app)?;
        let model = settings.model_for(spec);
        let mode = settings.auth_mode_for(spec);

        let mut env = Vec::new();
        let mut unset_env = Vec::new();

        match mode {
            AuthMode::ApiKey => {
                if let Some(key) = settings::get_api_key(spec.key_id) {
                    env.push((spec.env_var.to_string(), key));
                }
            }
            AuthMode::Subscription => {
                // An inherited API key would quietly switch the agent from the
                // user's subscription to per-token billing, so clear it.
                unset_env.push(spec.env_var.to_string());

                if let (Some(var), Some(key_id)) =
                    (spec.subscription_env_var, spec.subscription_key_id)
                {
                    if let Some(token) = settings::get_api_key(key_id) {
                        env.push((var.to_string(), token));
                    }
                }
            }
        }

        match spec.auth_settings_override {
            // gemini-cli pins its sign-in method in a settings file with no
            // CLI override, so Klets points it at a private one for this
            // process only.
            Some(providers::AuthSettingsOverride::GeminiCli) if mode == AuthMode::ApiKey => {
                let path = write_gemini_settings(app)?;
                env.push((
                    "GEMINI_CLI_SYSTEM_SETTINGS_PATH".to_string(),
                    path.to_string_lossy().to_string(),
                ));
            }
            // agy_acp_server ties its settings file and its OAuth cache to the
            // same $GEMINI_HOME root — there is no settings-only override
            // like gemini-cli's — so the two auth modes need opposite
            // strategies. See `AuthSettingsOverride::AntigravityAcp`.
            Some(providers::AuthSettingsOverride::AntigravityAcp) => match mode {
                AuthMode::ApiKey => {
                    let home = write_antigravity_api_key_settings(app)?;
                    env.push(("GEMINI_HOME".to_string(), home.to_string_lossy().to_string()));
                }
                AuthMode::Subscription => {
                    // Leave $GEMINI_HOME untouched so the real, shared
                    // ~/.gemini/oauth_creds.json — written by the user's own
                    // Antigravity IDE/CLI sign-in, not by Klets — stays
                    // visible. Confirmed empirically: a present credential
                    // file with no settings file still reads as fully
                    // unauthenticated, so one has to exist; never overwrite
                    // a real one.
                    ensure_antigravity_subscription_settings()?;
                }
            },
            _ => {}
        }

        let mut args: Vec<String> = spec.args.iter().map(|a| a.to_string()).collect();

        // Agents without ACP model selection take the model as an argument.
        let mut select_model_over_acp = true;
        if let (Some(flag), Some(model)) = (spec.model_flag, model.as_ref()) {
            args.push(flag.to_string());
            args.push(model.clone());
            select_model_over_acp = false;
        }

        Ok(Self {
            provider_id: spec.id.to_string(),
            provider_name: spec.name.to_string(),
            binary,
            args,
            env,
            unset_env,
            cwd,
            model,
            tool_policy: settings.tool_policy,
            system_prompt: settings.system_prompt.clone(),
            select_model_over_acp,
            login_hint: spec.login_hint.to_string(),
        })
    }
}

/// Instruction files the agents read from their working directory.
///
/// Each agent looks for a different name, and they all read plain markdown, so
/// the same prompt is written under every name rather than maintaining one per
/// provider.
const INSTRUCTION_FILES: &[&str] = &["AGENTS.md", "CLAUDE.md", "GEMINI.md"];

/// Put the user's prompt where the agent will find it.
///
/// Returns the files written so they can be cleaned up: when the session
/// directory is a real project, leaving a `CLAUDE.md` behind would trample the
/// user's own instructions.
fn write_instructions(cwd: &Path, prompt: &str) -> Vec<PathBuf> {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        return Vec::new();
    }

    let body = format!("{prompt}\n");
    let mut written = Vec::new();

    for name in INSTRUCTION_FILES {
        let path = cwd.join(name);
        // Never clobber instructions that already belong to the folder.
        if path.exists() {
            continue;
        }
        if std::fs::write(&path, &body).is_ok() {
            written.push(path);
        }
    }

    written
}

fn remove_instructions(paths: &[PathBuf]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

/// Write the settings file that forces gemini-cli onto API-key auth.
///
/// It is passed as gemini-cli's *system* settings, which outrank both user and
/// workspace settings and are not gated behind folder trust. Auto-update is
/// disabled too, since shadowing the real system file would otherwise re-enable
/// it mid-session.
fn write_gemini_settings(app: &AppHandle) -> AppResult<PathBuf> {
    let dir = settings::managed_dir(app)?.join("gemini");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("settings.json");

    let config = serde_json::json!({
        "security": { "auth": { "selectedType": "gemini-api-key" } },
        "general": { "enableAutoUpdate": false }
    });

    std::fs::write(&path, serde_json::to_string_pretty(&config)?)?;
    Ok(path)
}

/// Write a private settings file that forces agy_acp_server onto API-key
/// auth, inside an isolated `$GEMINI_HOME` Klets owns for this purpose.
///
/// Unlike gemini-cli, agy_acp_server resolves its settings file and its
/// OAuth credential cache from the same `$GEMINI_HOME` root, so there is no
/// way to override just the settings file the way [`write_gemini_settings`]
/// does. Pointing `$GEMINI_HOME` at a directory Klets owns keeps this mode
/// from ever reading or writing the user's real `~/.gemini`.
///
/// Returns the `$GEMINI_HOME` value to set, not the settings file path
/// itself.
fn write_antigravity_api_key_settings(app: &AppHandle) -> AppResult<PathBuf> {
    let home = settings::managed_dir(app)?.join("antigravity-home");
    let dir = home.join("antigravity-acp");
    std::fs::create_dir_all(&dir)?;

    let config = serde_json::json!({ "auth": { "type": "gemini-api-key" } });
    std::fs::write(dir.join("settings.json"), serde_json::to_string_pretty(&config)?)?;

    Ok(home)
}

/// Make sure agy_acp_server can resolve to oauth-personal on its own,
/// without redirecting `$GEMINI_HOME` away from the user's real
/// `~/.gemini` — that would hide the shared `oauth_creds.json` an existing
/// Antigravity IDE or CLI sign-in already wrote there.
///
fn ensure_antigravity_subscription_settings() -> AppResult<()> {
    let Some(home) = providers::home_dir() else {
        return Ok(());
    };
    ensure_antigravity_subscription_settings_at(&home)
}

/// The testable half of [`ensure_antigravity_subscription_settings`], taking
/// the home directory as a parameter instead of reading it from the
/// environment — mirrors the split in `providers::resolve_binary` /
/// `find_on_path`, for the same reason: real `HOME`/`USERPROFILE` isn't safe
/// to rewrite from a test.
///
/// Never overwrites an existing file: a real Antigravity install may already
/// own this path with its own `gcp.project`/`gcp.location` configuration.
fn ensure_antigravity_subscription_settings_at(home: &Path) -> AppResult<()> {
    let dir = home.join(".gemini").join("antigravity-acp");
    let path = dir.join("settings.json");
    if path.exists() {
        return Ok(());
    }

    std::fs::create_dir_all(&dir)?;
    let config = serde_json::json!({ "auth": { "type": "oauth-personal" } });
    std::fs::write(&path, serde_json::to_string_pretty(&config)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A temp directory unique to the test process, cleaned up on drop —
    /// same pattern `providers::tests` uses for the same reason: these tests
    /// touch the real filesystem and must not collide under parallel runs.
    struct TempDir(PathBuf);

    impl TempDir {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("klets-agent-test-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).expect("create temp dir");
            Self(dir)
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn writes_oauth_personal_when_no_settings_file_exists_yet() {
        let home = TempDir::new("missing");

        ensure_antigravity_subscription_settings_at(&home.0).expect("writes settings");

        let path = home.0.join(".gemini").join("antigravity-acp").join("settings.json");
        let written: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&path).expect("read settings")).expect("valid json");
        assert_eq!(written["auth"]["type"], "oauth-personal");
    }

    #[test]
    fn never_overwrites_a_real_settings_file() {
        // A real Antigravity IDE/CLI install may already own this path with
        // its own auth type and gcp project/location — Klets must never
        // clobber it, even if that means occasionally leaving a mismatched
        // config in place rather than "fixing" it silently.
        let home = TempDir::new("existing");
        let dir = home.0.join(".gemini").join("antigravity-acp");
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("settings.json");
        let real_config = r#"{"auth":{"type":"oauth-business"},"gcp":{"project":"real-project"}}"#;
        std::fs::write(&path, real_config).expect("seed real settings");

        ensure_antigravity_subscription_settings_at(&home.0).expect("no-op on existing file");

        let untouched = std::fs::read_to_string(&path).expect("read settings");
        assert_eq!(untouched, real_config);
    }
}

/// Handle to the running agent thread.
struct RunningAgent {
    provider_id: String,
    model: Option<String>,
    tx: UnboundedSender<AgentCommand>,
}

/// Shared state registered with Tauri.
#[derive(Default)]
pub struct AgentManager {
    current: Mutex<Option<RunningAgent>>,
    turn: Mutex<u64>,
    /// Shared with the agent thread so the UI can answer its questions.
    broker: PermissionBroker,
}

impl AgentManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn broker(&self) -> PermissionBroker {
        self.broker.clone()
    }

    fn next_turn(&self) -> u64 {
        let mut turn = self.turn.lock().expect("turn mutex");
        *turn += 1;
        *turn
    }

    /// Ensure an agent matching the requested provider and model is running.
    fn ensure(&self, app: &AppHandle, settings: &Settings, provider_id: &str) -> AppResult<()> {
        let spec = providers::find(provider_id)
            .ok_or_else(|| AppError::UnknownProvider(provider_id.to_string()))?;
        let plan = LaunchPlan::build(app, spec, settings)?;

        {
            let current = self.current.lock().expect("agent mutex");
            if let Some(running) = current.as_ref() {
                let same = running.provider_id == plan.provider_id
                    && running.model == plan.model
                    && !running.tx.is_closed();
                if same {
                    return Ok(());
                }
            }
        }

        self.stop();

        let (tx, rx) = mpsc::unbounded_channel();
        let running = RunningAgent {
            provider_id: plan.provider_id.clone(),
            model: plan.model.clone(),
            tx,
        };

        let app_handle = app.clone();
        let thread_plan = plan.clone();
        let thread_broker = self.broker.clone();
        std::thread::Builder::new()
            .name(format!("klets-agent-{}", plan.provider_id))
            .spawn(move || run_agent_thread(app_handle, thread_plan, thread_broker, rx))
            .map_err(|e| AppError::Agent(format!("could not start agent thread: {e}")))?;

        *self.current.lock().expect("agent mutex") = Some(running);
        Ok(())
    }

    /// Start the agent for `provider_id` ahead of the first prompt, so the
    /// process spawn and ACP handshake are already done by the time the user
    /// asks something.
    ///
    /// A thin public name over `ensure` so callers that only want to warm up
    /// — not send anything — don't have to reach for `prompt`. Free thanks to
    /// `ensure`'s own idempotence: calling this repeatedly (every time the
    /// launcher is shown, say) is a no-op once a matching agent is already
    /// running or already warming up.
    pub fn warm(&self, app: &AppHandle, settings: &Settings, provider_id: &str) -> AppResult<()> {
        self.ensure(app, settings, provider_id)
    }

    /// Send a prompt, starting the agent if needed. Returns the turn id.
    pub fn prompt(
        &self,
        app: &AppHandle,
        settings: &Settings,
        provider_id: &str,
        text: String,
    ) -> AppResult<u64> {
        self.ensure(app, settings, provider_id)?;
        let turn = self.next_turn();

        let current = self.current.lock().expect("agent mutex");
        let running = current.as_ref().ok_or(AppError::NoProvider)?;
        running
            .tx
            .send(AgentCommand::Prompt { turn, text })
            .map_err(|_| AppError::Agent("the agent stopped unexpectedly".into()))?;
        Ok(turn)
    }

    pub fn cancel(&self) {
        // Nothing should stay blocked on a prompt for a turn being abandoned.
        self.broker.clear();
        let current = self.current.lock().expect("agent mutex");
        if let Some(running) = current.as_ref() {
            let _ = running.tx.send(AgentCommand::Cancel);
        }
    }

    /// Start a fresh conversation without restarting the process.
    pub fn new_session(&self) -> u64 {
        let turn = self.next_turn();
        let current = self.current.lock().expect("agent mutex");
        if let Some(running) = current.as_ref() {
            let _ = running.tx.send(AgentCommand::NewSession { turn });
        }
        turn
    }

    pub fn stop(&self) {
        self.broker.clear();
        let mut current = self.current.lock().expect("agent mutex");
        if let Some(running) = current.take() {
            let _ = running.tx.send(AgentCommand::Shutdown);
        }
    }

    pub fn active_provider(&self) -> Option<String> {
        self.current
            .lock()
            .expect("agent mutex")
            .as_ref()
            .map(|r| r.provider_id.clone())
    }
}

fn emit<S: EventSink>(sink: &S, event: AgentEvent) {
    sink.emit(event);
}

fn emit_status<S: EventSink>(sink: &S, provider: &str, state: AgentState, detail: Option<String>) {
    emit(
        sink,
        AgentEvent::Status {
            provider: provider.to_string(),
            state,
            detail,
        },
    );
}

/// Entry point of the dedicated agent thread.
///
/// Blocks until the agent shuts down, so callers run it on their own thread.
pub fn run_agent_thread<S: EventSink>(
    sink: S,
    plan: LaunchPlan,
    broker: PermissionBroker,
    rx: mpsc::UnboundedReceiver<AgentCommand>,
) {
    let runtime = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            emit(
                &sink,
                AgentEvent::Error {
                    turn: 0,
                    message: format!("could not start agent runtime: {e}"),
                },
            );
            return;
        }
    };

    let local = tokio::task::LocalSet::new();
    let provider_id = plan.provider_id.clone();
    let sink_for_error = sink.clone();

    local.block_on(&runtime, async move {
        if let Err(message) = drive_agent(sink, plan, broker, rx).await {
            emit(
                &sink_for_error,
                AgentEvent::Error {
                    turn: 0,
                    message: message.clone(),
                },
            );
            emit_status(
                &sink_for_error,
                &provider_id,
                AgentState::Stopped,
                Some(message),
            );
        } else {
            emit_status(&sink_for_error, &provider_id, AgentState::Stopped, None);
        }
    });
}

/// Write the instruction files, run the agent, and remove them again however
/// it ends, including a failed spawn or handshake.
async fn drive_agent<S: EventSink>(
    sink: S,
    plan: LaunchPlan,
    broker: PermissionBroker,
    rx: mpsc::UnboundedReceiver<AgentCommand>,
) -> Result<(), String> {
    emit_status(&sink, &plan.provider_id, AgentState::Starting, None);

    // Written before the agent starts, since instruction files are read when
    // the session opens.
    let instructions = write_instructions(&plan.cwd, &plan.system_prompt);
    let result = run_session(sink, &plan, broker, rx).await;
    remove_instructions(&instructions);
    result
}

/// Spawn the process, run the ACP handshake, then service commands until shutdown.
async fn run_session<S: EventSink>(
    sink: S,
    plan: &LaunchPlan,
    broker: PermissionBroker,
    mut rx: mpsc::UnboundedReceiver<AgentCommand>,
) -> Result<(), String> {
    let mut command = tokio::process::Command::new(&plan.binary);
    command
        .args(&plan.args)
        .current_dir(&plan.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);

    for (key, value) in &plan.env {
        command.env(key, value);
    }
    for key in &plan.unset_env {
        command.env_remove(key);
    }

    // Keep npm/.cmd shims from flashing a console window on Windows.
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command
        .spawn()
        .map_err(|e| format!("could not start {}: {e}", plan.binary.display()))?;

    let stdin = child.stdin.take().ok_or("agent stdin unavailable")?;
    let stdout = child.stdout.take().ok_or("agent stdout unavailable")?;
    let stderr = child.stderr.take().ok_or("agent stderr unavailable")?;

    // Keep the tail of stderr so a crash can be explained to the user.
    let stderr_tail: Arc<Mutex<VecDeque<String>>> = Arc::new(Mutex::new(VecDeque::new()));
    {
        let tail = stderr_tail.clone();
        tokio::task::spawn_local(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(target: "klets::agent", "stderr: {line}");
                let mut tail = tail.lock().expect("stderr mutex");
                if tail.len() == 20 {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        });
    }

    emit_status(&sink, &plan.provider_id, AgentState::Connecting, None);

    let turn_cell = Rc::new(Cell::new(0u64));
    let handler = KletsClient {
        sink: sink.clone(),
        turn: turn_cell.clone(),
        provider_id: plan.provider_id.clone(),
        policy: plan.tool_policy,
        broker: broker.clone(),
    };

    let (connection, io_task) = ClientSideConnection::new(
        handler,
        stdin.compat_write(),
        stdout.compat(),
        |fut| {
            tokio::task::spawn_local(fut);
        },
    );

    tokio::task::spawn_local(async move {
        if let Err(e) = io_task.await {
            tracing::warn!(target: "klets::agent", "connection closed: {e}");
        }
    });

    let connection = Rc::new(connection);
    let describe = |e: AcpError| -> String { format!("{}: {}", e.code, e.message) };

    connection
        .initialize(InitializeRequest::new(ProtocolVersion::LATEST))
        .await
        .map_err(|e| {
            let tail = stderr_snapshot(&stderr_tail);
            explain(plan, &format!("{} could not start.", plan.provider_name), &describe(e), &tail)
        })?;

    let session = connection
        .new_session(NewSessionRequest::new(plan.cwd.clone()))
        .await
        .map_err(|e| {
            let tail = stderr_snapshot(&stderr_tail);
            explain(
                plan,
                &format!("{} rejected the session.", plan.provider_name),
                &describe(e),
                &tail,
            )
        })?;

    let mut session_id = session.session_id.clone();

    if plan.select_model_over_acp {
        if let Some(model) = plan.model.clone() {
            select_model(&sink, 0, &connection, &session, &session_id, &model).await;
        }
    }

    emit_status(&sink, &plan.provider_id, AgentState::Ready, None);

    while let Some(command) = rx.recv().await {
        match command {
            AgentCommand::Prompt { turn, text } => {
                turn_cell.set(turn);
                let connection = connection.clone();
                let sink = sink.clone();
                let session_id = session_id.clone();
                let stderr_tail = stderr_tail.clone();

                // Run the turn concurrently so cancels are still processed.
                tokio::task::spawn_local(async move {
                    let request = PromptRequest::new(
                        session_id,
                        vec![ContentBlock::Text(TextContent::new(text))],
                    );
                    match connection.prompt(request).await {
                        Ok(response) => emit(
                            &sink,
                            AgentEvent::Done {
                                turn,
                                stop_reason: serde_json::to_value(response.stop_reason)
                                    .ok()
                                    .and_then(|v| v.as_str().map(str::to_string)),
                            },
                        ),
                        Err(e) => {
                            let tail = stderr_snapshot(&stderr_tail);
                            emit(
                                &sink,
                                AgentEvent::Error {
                                    turn,
                                    message: format!("{}{tail}", e.message),
                                },
                            );
                        }
                    }
                });
            }
            AgentCommand::Cancel => {
                let _ = connection
                    .cancel(CancelNotification::new(session_id.clone()))
                    .await;
            }
            AgentCommand::NewSession { turn } => {
                match connection
                    .new_session(NewSessionRequest::new(plan.cwd.clone()))
                    .await
                {
                    Ok(response) => {
                        session_id = response.session_id.clone();
                        // Each session starts on the agent's own default model.
                        if plan.select_model_over_acp {
                            if let Some(model) = plan.model.as_deref() {
                                select_model(&sink, turn, &connection, &response, &session_id, model)
                                    .await;
                            }
                        }
                        emit(
                            &sink,
                            AgentEvent::Done {
                                turn,
                                stop_reason: Some("new_session".into()),
                            },
                        );
                    }
                    Err(e) => emit(
                        &sink,
                        AgentEvent::Error {
                            turn,
                            message: describe(e),
                        },
                    ),
                }
            }
            AgentCommand::Shutdown => break,
        }
    }

    let _ = child.kill().await;
    Ok(())
}

/// Point the session at a specific model.
///
/// Agents advertise model choice in one of two ways: the `session/set_model`
/// route, or a `model` entry in the session's config options (what OpenCode
/// does — which is how Go, Zen and Gemini, all one binary, are told apart).
///
/// Failures are reported rather than swallowed. Silently falling back would
/// leave the launcher claiming one model while another answered, which also
/// hides the difference between the OpenCode-backed providers.
async fn select_model<S: EventSink>(
    sink: &S,
    turn: u64,
    connection: &Rc<ClientSideConnection>,
    session: &agent_client_protocol::NewSessionResponse,
    session_id: &agent_client_protocol::SessionId,
    model: &str,
) {
    let notify = |message: String| emit(sink, AgentEvent::Notice { turn, message });

    if let Some(state) = session.models.as_ref() {
        if state
            .available_models
            .iter()
            .any(|m| m.model_id.0.as_ref() == model)
        {
            if let Err(e) = connection
                .set_session_model(SetSessionModelRequest::new(
                    session_id.clone(),
                    ModelId::new(model.to_string()),
                ))
                .await
            {
                notify(format!("Could not switch to {model}: {}", e.message));
            }
            return;
        }
    }

    let model_option = session.config_options.as_ref().and_then(|options| {
        options.iter().find(|option| {
            matches!(option.category, Some(SessionConfigOptionCategory::Model))
                || option.id.0.as_ref() == "model"
        })
    });

    let Some(option) = model_option else {
        notify(format!(
            "This agent does not support choosing a model, so {model} was ignored."
        ));
        return;
    };

    // The change is attempted even when the model is missing from the
    // advertised list, because agents do not always advertise everything they
    // accept. What matters is the value that comes back.
    let response = connection
        .set_session_config_option(SetSessionConfigOptionRequest::new(
            session_id.clone(),
            option.id.clone(),
            SessionConfigValueId::new(model.to_string()),
        ))
        .await;

    match response {
        Ok(response) => {
            let applied = response
                .config_options
                .iter()
                .find(|updated| updated.id == option.id)
                .and_then(|updated| match &updated.kind {
                    SessionConfigKind::Select(select) => {
                        Some(select.current_value.0.as_ref().to_string())
                    }
                    _ => None,
                });

            // Reading the value back is the only way to catch an agent that
            // accepts the request but quietly keeps its own model.
            if let Some(applied) = applied {
                if applied != model {
                    notify(format!(
                        "{model} is unavailable, so {applied} answered instead. \
                         Change the model in settings."
                    ));
                }
            }
        }
        Err(e) => notify(format!("Could not switch to {model}: {}", e.message)),
    }
}

/// Build a failure message, adding recovery steps when the agent refused to
/// authenticate.
///
/// Klets infers sign-in state from credential files, which cannot tell that a
/// token has expired or been revoked, so an auth failure needs to say what to
/// do about it rather than just repeating the agent's error.
fn explain(plan: &LaunchPlan, headline: &str, detail: &str, stderr_tail: &str) -> String {
    let looks_like_auth = detail.to_lowercase().contains("auth")
        || detail.to_lowercase().contains("credential")
        || detail.to_lowercase().contains("unauthorized");

    if looks_like_auth {
        format!(
            "{headline} {detail}\n\nSign in with `{}`, or add an API key in Klets settings.{stderr_tail}",
            plan.login_hint
        )
    } else {
        format!("{headline} {detail}{stderr_tail}")
    }
}

fn stderr_snapshot(tail: &Arc<Mutex<VecDeque<String>>>) -> String {
    let tail = tail.lock().expect("stderr mutex");
    if tail.is_empty() {
        return String::new();
    }
    let lines: Vec<&str> = tail.iter().rev().take(4).map(String::as_str).rev().collect();
    format!("\n\n{}", lines.join("\n"))
}

/// The client half of ACP: everything the agent may ask *us* to do.
///
/// Klets is a quick-answer window, not a coding session, so it declares no
/// filesystem or terminal capabilities and refuses tool permission requests.
/// Every other client method inherits the trait's `method_not_found` default.
struct KletsClient<S: EventSink> {
    sink: S,
    turn: Rc<Cell<u64>>,
    provider_id: String,
    policy: ToolPolicy,
    broker: PermissionBroker,
}

#[async_trait::async_trait(?Send)]
impl<S: EventSink> Client for KletsClient<S> {
    async fn request_permission(
        &self,
        args: RequestPermissionRequest,
    ) -> Result<RequestPermissionResponse, AcpError> {
        let turn = self.turn.get();
        let fields = args.tool_call.fields;
        let kind = fields.kind.unwrap_or_default();
        let title = fields.title.unwrap_or_else(|| "a tool".to_string());

        let respond = |allow: bool| {
            let outcome = permissions::option_for(&args.options, allow)
                .map(|id| RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id)));

            // An agent that offers no option of the right kind gets a cancel,
            // which every adapter treats as "did not happen".
            Ok(RequestPermissionResponse::new(
                outcome.unwrap_or(RequestPermissionOutcome::Cancelled),
            ))
        };

        match self.policy.decide(kind) {
            permissions::Decision::Allow => respond(true),
            permissions::Decision::Deny => {
                emit(
                    &self.sink,
                    AgentEvent::PermissionDenied {
                        turn,
                        title,
                        kind: as_str(&kind),
                    },
                );
                respond(false)
            }
            permissions::Decision::Ask => {
                let (id, receiver) = self.broker.register();

                self.sink.attention();
                emit(
                    &self.sink,
                    AgentEvent::PermissionRequest {
                        turn,
                        request: permissions::PendingPermission {
                            id,
                            title,
                            kind: as_str(&kind),
                            detail: permissions::describe_input(fields.raw_input.as_ref()),
                        },
                    },
                );

                // Refuse on the user's behalf if nobody answers, so a request
                // raised while the launcher is hidden cannot stall the turn.
                let allowed =
                    match tokio::time::timeout(permissions::DECISION_TIMEOUT, receiver).await {
                        Ok(Ok(decision)) => decision,
                        _ => {
                            self.broker.forget(id);
                            emit(
                                &self.sink,
                                AgentEvent::PermissionResolved {
                                    turn,
                                    id,
                                    allowed: false,
                                    timed_out: true,
                                },
                            );
                            return respond(false);
                        }
                    };

                emit(
                    &self.sink,
                    AgentEvent::PermissionResolved {
                        turn,
                        id,
                        allowed,
                        timed_out: false,
                    },
                );
                respond(allowed)
            }
        }
    }

    async fn session_notification(&self, args: SessionNotification) -> Result<(), AcpError> {
        let turn = self.turn.get();

        match args.update {
            SessionUpdate::AgentMessageChunk(chunk) => {
                if let Some(text) = text_of(&chunk.content) {
                    emit(&self.sink, AgentEvent::Chunk { turn, text });
                }
            }
            SessionUpdate::AgentThoughtChunk(chunk) => {
                if let Some(text) = text_of(&chunk.content) {
                    emit(&self.sink, AgentEvent::Thought { turn, text });
                }
            }
            SessionUpdate::ToolCall(call) => emit(
                &self.sink,
                AgentEvent::Tool {
                    turn,
                    id: call.tool_call_id.0.to_string(),
                    title: call.title,
                    kind: as_str(&call.kind),
                    status: as_str(&call.status),
                },
            ),
            // How Klets learns which MCP servers and skills are live: agents
            // load those from their own config and announce the result here.
            SessionUpdate::AvailableCommandsUpdate(update) => emit(
                &self.sink,
                AgentEvent::Capabilities {
                    provider: self.provider_id.clone(),
                    commands: update
                        .available_commands
                        .into_iter()
                        .map(|command| AgentCommandInfo {
                            name: command.name,
                            description: command.description,
                        })
                        .collect(),
                },
            ),
            SessionUpdate::ToolCallUpdate(update) => emit(
                &self.sink,
                AgentEvent::Tool {
                    turn,
                    id: update.tool_call_id.0.to_string(),
                    title: update.fields.title.unwrap_or_default(),
                    kind: update.fields.kind.as_ref().map(as_str).unwrap_or_default(),
                    status: update.fields.status.as_ref().map(as_str).unwrap_or_default(),
                },
            ),
            _ => {}
        }

        Ok(())
    }
}

fn text_of(block: &ContentBlock) -> Option<String> {
    match block {
        ContentBlock::Text(text) => Some(text.text.clone()),
        _ => None,
    }
}

/// Serialize a schema enum to its wire string (`in_progress`, `read`, ...).
fn as_str<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}
