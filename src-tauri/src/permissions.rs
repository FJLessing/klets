//! Deciding what an agent is allowed to do.
//!
//! Agents run their own tools, so Klets never sees the call itself — it only
//! gets a `session/request_permission` asking for a yes or no. That makes this
//! the single enforcement point, and it behaves identically no matter which
//! agent is connected or how that agent's own permission config is set up.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_client_protocol::{
    PermissionOption, PermissionOptionId, PermissionOptionKind, ToolKind,
};
use serde::{Deserialize, Serialize};
use tokio::sync::oneshot;

/// How much an agent may do on the user's behalf.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum ToolPolicy {
    /// Refuse everything. Answers come from the model's own knowledge.
    Off,
    /// Allow inspection, refuse everything else without asking.
    ReadOnly,
    /// Allow inspection, and ask before anything runs a command.
    #[default]
    AskToRun,
}

/// What should happen to a single request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Allow,
    Deny,
    Ask,
}

impl ToolPolicy {
    /// Reads, searches, fetches and thinking are inspection: they cannot
    /// change anything, so they never interrupt. Anything that writes is
    /// refused outright — Klets is a question box, and an agent that wants to
    /// edit should be run properly instead. Running a command sits in between,
    /// because "read-only command" is not something the protocol can express.
    pub fn decide(self, kind: ToolKind) -> Decision {
        if self == ToolPolicy::Off {
            return Decision::Deny;
        }

        match kind {
            ToolKind::Read | ToolKind::Search | ToolKind::Fetch | ToolKind::Think => {
                Decision::Allow
            }
            ToolKind::Edit | ToolKind::Delete | ToolKind::Move | ToolKind::SwitchMode => {
                Decision::Deny
            }
            // Execute and anything unrecognised.
            _ => {
                if self == ToolPolicy::AskToRun {
                    Decision::Ask
                } else {
                    Decision::Deny
                }
            }
        }
    }
}

/// Pick the option matching an outcome.
///
/// Option ids are agent-specific, so they are matched on `kind` instead.
/// `AllowOnce` is preferred over `AllowAlways` so a decision never silently
/// widens beyond the request the user actually saw.
pub fn option_for(options: &[PermissionOption], allow: bool) -> Option<PermissionOptionId> {
    let wanted: &[PermissionOptionKind] = if allow {
        &[PermissionOptionKind::AllowOnce, PermissionOptionKind::AllowAlways]
    } else {
        &[PermissionOptionKind::RejectOnce, PermissionOptionKind::RejectAlways]
    };

    wanted.iter().find_map(|kind| {
        options
            .iter()
            .find(|option| option.kind == *kind)
            .map(|option| option.option_id.clone())
    })
}

/// A request waiting on the user.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPermission {
    pub id: u64,
    pub title: String,
    pub kind: String,
    /// The command or arguments, when the agent supplies them.
    pub detail: Option<String>,
}

/// Carries a decision from the UI thread back to the agent thread.
///
/// The ACP connection is `!Send` and lives on its own thread, so the answer
/// cannot simply be returned from a command; it is handed over a oneshot
/// keyed by request id.
#[derive(Clone, Default)]
pub struct PermissionBroker {
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<bool>>>>,
    next_id: Arc<AtomicU64>,
}

impl PermissionBroker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a request and hand back its id plus the receiver to await.
    pub fn register(&self) -> (u64, oneshot::Receiver<bool>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed) + 1;
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("permission mutex").insert(id, tx);
        (id, rx)
    }

    /// Answer a pending request. Unknown ids are ignored: the request may have
    /// already timed out or been cancelled with the turn.
    pub fn resolve(&self, id: u64, allow: bool) {
        let sender = self.pending.lock().expect("permission mutex").remove(&id);
        if let Some(sender) = sender {
            let _ = sender.send(allow);
        }
    }

    pub fn forget(&self, id: u64) {
        self.pending.lock().expect("permission mutex").remove(&id);
    }

    /// Deny everything outstanding, used when a turn is cancelled.
    pub fn clear(&self) {
        let drained: Vec<_> = self
            .pending
            .lock()
            .expect("permission mutex")
            .drain()
            .map(|(_, sender)| sender)
            .collect();
        for sender in drained {
            let _ = sender.send(false);
        }
    }
}

/// How long a request waits before it is refused on the user's behalf.
///
/// Without this a request raised while the launcher is hidden would stall the
/// turn indefinitely.
pub const DECISION_TIMEOUT: Duration = Duration::from_secs(60);

/// Summarise a tool call for the approval prompt.
///
/// Agents put the interesting part in different places, so the command is
/// pulled from the usual `raw_input` keys and falls back to the raw JSON.
pub fn describe_input(raw_input: Option<&serde_json::Value>) -> Option<String> {
    let value = raw_input?;

    for key in ["command", "cmd", "script", "query", "path", "file_path", "url"] {
        if let Some(found) = value.get(key).and_then(|v| v.as_str()) {
            return Some(found.to_string());
        }
    }

    match value {
        serde_json::Value::String(text) => Some(text.clone()),
        serde_json::Value::Object(map) if map.is_empty() => None,
        other => Some(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_client_protocol::PermissionOption;
    use serde_json::json;

    // --- ToolPolicy::decide -------------------------------------------
    //
    // This is the entire enforcement point for what an agent may do (see the
    // module doc comment), so every kind against every policy is checked
    // explicitly rather than sampled — a silent regression here changes what
    // Klets will let an agent do without anyone choosing that.

    const INSPECTION_KINDS: &[ToolKind] =
        &[ToolKind::Read, ToolKind::Search, ToolKind::Fetch, ToolKind::Think];
    const MUTATING_KINDS: &[ToolKind] =
        &[ToolKind::Edit, ToolKind::Delete, ToolKind::Move, ToolKind::SwitchMode];

    #[test]
    fn off_denies_everything_including_inspection() {
        for kind in INSPECTION_KINDS.iter().chain(MUTATING_KINDS).chain([&ToolKind::Execute]) {
            assert_eq!(ToolPolicy::Off.decide(*kind), Decision::Deny, "kind {kind:?}");
        }
    }

    #[test]
    fn read_only_allows_inspection_but_denies_execution() {
        for kind in INSPECTION_KINDS {
            assert_eq!(ToolPolicy::ReadOnly.decide(*kind), Decision::Allow, "kind {kind:?}");
        }
        assert_eq!(ToolPolicy::ReadOnly.decide(ToolKind::Execute), Decision::Deny);
    }

    #[test]
    fn ask_to_run_allows_inspection_and_asks_before_executing() {
        for kind in INSPECTION_KINDS {
            assert_eq!(ToolPolicy::AskToRun.decide(*kind), Decision::Allow, "kind {kind:?}");
        }
        assert_eq!(ToolPolicy::AskToRun.decide(ToolKind::Execute), Decision::Ask);
    }

    #[test]
    fn mutating_kinds_are_always_denied_regardless_of_policy() {
        // Edits are refused outright under every policy, including the most
        // permissive one — Klets is a question box, not a coding session.
        for policy in [ToolPolicy::Off, ToolPolicy::ReadOnly, ToolPolicy::AskToRun] {
            for kind in MUTATING_KINDS {
                assert_eq!(policy.decide(*kind), Decision::Deny, "policy {policy:?}, kind {kind:?}");
            }
        }
    }

    #[test]
    fn an_unrecognised_kind_is_treated_like_execute() {
        // `ToolKind::default()` is what an agent gets when it omits `kind`
        // entirely; it must not be treated as more trustworthy than Execute.
        assert_eq!(ToolPolicy::AskToRun.decide(ToolKind::default()), Decision::Ask);
        assert_eq!(ToolPolicy::ReadOnly.decide(ToolKind::default()), Decision::Deny);
    }

    // --- option_for -----------------------------------------------------

    fn option(id: &'static str, kind: PermissionOptionKind) -> PermissionOption {
        PermissionOption::new(id, format!("{kind:?}"), kind)
    }

    #[test]
    fn option_for_allow_prefers_once_over_always() {
        let options = vec![
            option("always", PermissionOptionKind::AllowAlways),
            option("once", PermissionOptionKind::AllowOnce),
        ];
        assert_eq!(option_for(&options, true).map(|id| id.0.to_string()), Some("once".into()));
    }

    #[test]
    fn option_for_deny_prefers_once_over_always() {
        let options = vec![
            option("always", PermissionOptionKind::RejectAlways),
            option("once", PermissionOptionKind::RejectOnce),
        ];
        assert_eq!(option_for(&options, false).map(|id| id.0.to_string()), Some("once".into()));
    }

    #[test]
    fn option_for_falls_back_to_the_only_kind_offered() {
        let options = vec![option("always", PermissionOptionKind::AllowAlways)];
        assert_eq!(option_for(&options, true).map(|id| id.0.to_string()), Some("always".into()));
    }

    #[test]
    fn option_for_returns_none_when_no_option_matches_the_outcome() {
        // An agent offering only allow options while Klets wants to deny: the
        // caller must fall back to Cancelled rather than pick the wrong one.
        let options = vec![option("once", PermissionOptionKind::AllowOnce)];
        assert_eq!(option_for(&options, false), None);
    }

    // --- describe_input ---------------------------------------------------

    #[test]
    fn describe_input_extracts_the_command_field() {
        assert_eq!(
            describe_input(Some(&json!({ "command": "rm -rf /" }))),
            Some("rm -rf /".to_string()),
        );
    }

    #[test]
    fn describe_input_tries_known_keys_in_order() {
        // `command` is checked before `url`, so a payload with both surfaces
        // the more specific one.
        assert_eq!(
            describe_input(Some(&json!({ "url": "https://example.com", "command": "curl" }))),
            Some("curl".to_string()),
        );
    }

    #[test]
    fn describe_input_falls_back_to_a_bare_string() {
        assert_eq!(describe_input(Some(&json!("just a string"))), Some("just a string".to_string()));
    }

    #[test]
    fn describe_input_returns_none_for_an_empty_object() {
        assert_eq!(describe_input(Some(&json!({}))), None);
    }

    #[test]
    fn describe_input_returns_none_when_absent() {
        assert_eq!(describe_input(None), None);
    }

    // --- PermissionBroker ---------------------------------------------

    #[tokio::test]
    async fn resolve_delivers_the_decision_to_the_matching_receiver() {
        let broker = PermissionBroker::new();
        let (id, receiver) = broker.register();

        broker.resolve(id, true);

        assert_eq!(receiver.await, Ok(true));
    }

    #[tokio::test]
    async fn resolve_of_an_unknown_id_is_a_harmless_no_op() {
        let broker = PermissionBroker::new();
        let (id, receiver) = broker.register();

        broker.resolve(id + 999, true);
        broker.forget(id);

        // The real receiver was never resolved, so awaiting it observes the
        // sender being dropped rather than a stray answer for someone else.
        assert!(receiver.await.is_err());
    }

    #[tokio::test]
    async fn forget_prevents_a_later_resolve_from_doing_anything() {
        let broker = PermissionBroker::new();
        let (id, receiver) = broker.register();

        broker.forget(id);
        broker.resolve(id, true);

        assert!(receiver.await.is_err());
    }

    #[tokio::test]
    async fn clear_denies_every_outstanding_request() {
        let broker = PermissionBroker::new();
        let (_, first) = broker.register();
        let (_, second) = broker.register();

        broker.clear();

        assert_eq!(first.await, Ok(false));
        assert_eq!(second.await, Ok(false));
    }

    #[test]
    fn register_hands_out_distinct_ids() {
        let broker = PermissionBroker::new();
        let (a, _) = broker.register();
        let (b, _) = broker.register();
        assert_ne!(a, b);
    }
}
