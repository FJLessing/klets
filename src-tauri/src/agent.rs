//! Owns the ACP agent subprocess and the connection to it.
//!
//! The `agent-client-protocol` client connection is `!Send` (it is built on
//! `LocalBoxFuture`), so it cannot live in Tauri's shared state directly.
//! Instead each agent runs on its own thread with a current-thread Tokio
//! runtime and a `LocalSet`; the rest of the app talks to it over an mpsc
//! channel of [`AgentCommand`].

use std::cell::Cell;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::process::Stdio;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use agent_client_protocol::{
    Agent as _, CancelNotification, Client, ClientSideConnection, ContentBlock, Error as AcpError,
    InitializeRequest, ModelId, NewSessionRequest, PromptRequest, ProtocolVersion,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SessionConfigOptionCategory, SessionConfigValueId, SessionNotification, SessionUpdate,
    SetSessionConfigOptionRequest, SetSessionModelRequest, TextContent,
};
use tauri::AppHandle;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::mpsc::{self, UnboundedSender};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::error::{AppError, AppResult};
use crate::events::{AgentEvent, AgentState, EventSink};
use crate::providers::{self, ProviderSpec};
use crate::settings::{self, Settings};

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
    pub cwd: PathBuf,
    pub model: Option<String>,
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

        // The key is optional: every one of these agents can also run on the
        // credentials from its own `login` flow, so an empty keychain entry
        // means "let the CLI authenticate itself".
        let key = settings::get_api_key(spec.key_id);

        let cwd = settings::workspace_dir(app)?;
        let model = settings.model_for(spec);

        let mut env = Vec::new();
        if let Some(key) = key {
            env.push((spec.env_var.to_string(), key));
        }

        Ok(Self {
            provider_id: spec.id.to_string(),
            provider_name: spec.name.to_string(),
            binary,
            args: spec.args.iter().map(|a| a.to_string()).collect(),
            env,
            cwd,
            model,
            login_hint: spec.login_hint.to_string(),
        })
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
}

impl AgentManager {
    pub fn new() -> Self {
        Self::default()
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
        std::thread::Builder::new()
            .name(format!("klets-agent-{}", plan.provider_id))
            .spawn(move || run_agent_thread(app_handle, thread_plan, rx))
            .map_err(|e| AppError::Agent(format!("could not start agent thread: {e}")))?;

        *self.current.lock().expect("agent mutex") = Some(running);
        Ok(())
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
        if let Err(message) = drive_agent(sink, plan, rx).await {
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

/// Spawn the process, run the ACP handshake, then service commands until shutdown.
async fn drive_agent<S: EventSink>(
    sink: S,
    plan: LaunchPlan,
    mut rx: mpsc::UnboundedReceiver<AgentCommand>,
) -> Result<(), String> {
    emit_status(&sink, &plan.provider_id, AgentState::Starting, None);

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
            explain(&plan, &format!("{} could not start.", plan.provider_name), &describe(e), &tail)
        })?;

    let session = connection
        .new_session(NewSessionRequest::new(plan.cwd.clone()))
        .await
        .map_err(|e| {
            let tail = stderr_snapshot(&stderr_tail);
            explain(
                &plan,
                &format!("{} rejected the session.", plan.provider_name),
                &describe(e),
                &tail,
            )
        })?;

    let mut session_id = session.session_id.clone();

    if let Some(model) = plan.model.clone() {
        select_model(&connection, &session, &session_id, &model).await;
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
/// Agents advertise model choice in one of two ways: the unstable
/// `session/set_model` route, or a `model` entry in the session's config
/// options (what OpenCode does — which is how Go and Zen, one binary, are told
/// apart). Both are tried, and an agent that supports neither keeps its
/// default rather than failing the turn.
async fn select_model(
    connection: &Rc<ClientSideConnection>,
    session: &agent_client_protocol::NewSessionResponse,
    session_id: &agent_client_protocol::SessionId,
    model: &str,
) {
    let known_model = session.models.as_ref().is_some_and(|state| {
        state
            .available_models
            .iter()
            .any(|m| m.model_id.0.as_ref() == model)
    });

    if known_model {
        let _ = connection
            .set_session_model(SetSessionModelRequest::new(
                session_id.clone(),
                ModelId::new(model.to_string()),
            ))
            .await;
        return;
    }

    let Some(options) = session.config_options.as_ref() else {
        return;
    };

    let model_option = options.iter().find(|option| {
        matches!(option.category, Some(SessionConfigOptionCategory::Model))
            || option.id.0.as_ref() == "model"
    });

    if let Some(option) = model_option {
        let _ = connection
            .set_session_config_option(SetSessionConfigOptionRequest::new(
                session_id.clone(),
                option.id.clone(),
                SessionConfigValueId::new(model.to_string()),
            ))
            .await;
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
}

#[async_trait::async_trait(?Send)]
impl<S: EventSink> Client for KletsClient<S> {
    async fn request_permission(
        &self,
        args: RequestPermissionRequest,
    ) -> Result<RequestPermissionResponse, AcpError> {
        let title = args
            .tool_call
            .fields
            .title
            .unwrap_or_else(|| "a tool".to_string());

        emit(
            &self.sink,
            AgentEvent::PermissionDenied {
                turn: self.turn.get(),
                title,
            },
        );

        Ok(RequestPermissionResponse::new(
            RequestPermissionOutcome::Cancelled,
        ))
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
