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
}

impl EventSink for tauri::AppHandle {
    fn emit(&self, event: AgentEvent) {
        use tauri::Emitter;
        let _ = Emitter::emit(self, AGENT_EVENT, event);
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
    /// A tool request that safe mode refused.
    PermissionDenied { turn: u64, title: String },
    /// The turn finished.
    Done {
        turn: u64,
        #[serde(skip_serializing_if = "Option::is_none")]
        stop_reason: Option<String>,
    },
    /// The turn (or the connection) failed.
    Error { turn: u64, message: String },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum AgentState {
    Starting,
    Connecting,
    Ready,
    Stopped,
}
