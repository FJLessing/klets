use serde::Serialize;

/// Event name the launcher listens on.
pub const AGENT_EVENT: &str = "klets://agent";

/// Destination for streamed agent events.
///
/// The agent pipeline is written against this rather than `AppHandle` so it
/// can be driven without a running Tauri app — see `examples/acp_probe.rs`,
/// which exercises the same code path against a real agent binary.
pub trait EventSink: Clone + Send + 'static {
    fn emit(&self, event: AgentEvent);

    /// Bring the launcher forward because something needs the user.
    ///
    /// A tool request raised while the window is hidden would otherwise sit
    /// unanswered until it times out.
    fn attention(&self) {}
}

impl EventSink for tauri::AppHandle {
    fn emit(&self, event: AgentEvent) {
        use tauri::Emitter;
        let _ = Emitter::emit(self, AGENT_EVENT, event);
    }

    fn attention(&self) {
        crate::windows::show_launcher(self);
    }
}

/// Everything the Rust core streams to the launcher during a turn.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AgentEvent {
    /// Connection lifecycle: spawning the binary, handshaking, ready.
    Status {
        provider: String,
        state: AgentState,
        #[serde(skip_serializing_if = "Option::is_none")]
        detail: Option<String>,
    },
    /// A chunk of the answer.
    Chunk { turn: u64, text: String },
    /// A chunk of the agent's reasoning.
    Thought { turn: u64, text: String },
    /// Tool activity, rendered as a compact chip.
    Tool {
        turn: u64,
        id: String,
        title: String,
        kind: String,
        status: String,
    },
    /// A tool request that policy refused without asking.
    PermissionDenied {
        turn: u64,
        title: String,
        kind: String,
    },
    /// A tool request waiting on the user.
    PermissionRequest {
        turn: u64,
        request: crate::permissions::PendingPermission,
    },
    /// A pending request that is no longer waiting, with how it ended.
    PermissionResolved {
        turn: u64,
        id: u64,
        allowed: bool,
        /// True when nobody answered in time.
        timed_out: bool,
    },
    /// Something the user should know that isn't an error, such as the
    /// requested model being unavailable.
    Notice { turn: u64, message: String },
    /// What the connected agent can actually do, as it reports it.
    ///
    /// Agents load their own MCP servers and skills from their own config, so
    /// this is the only way to know what is live in a session.
    Capabilities {
        provider: String,
        commands: Vec<AgentCommandInfo>,
    },
    /// The turn finished.
    Done {
        turn: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        stop_reason: Option<String>,
    },
    /// The turn (or the connection) failed.
    Error { turn: u64, message: String },
}

/// A command or skill the agent advertises.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentCommandInfo {
    pub name: String,
    pub description: String,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentState {
    Starting,
    Connecting,
    Ready,
    Stopped,
}
