//! Headless probe for the agent pipeline.
//!
//! Runs the exact code path the app uses — process spawn, ACP handshake,
//! session, streaming prompt — against a real agent binary, and prints every
//! event. Useful for verifying a provider works before wiring it into the
//! launcher, and for debugging an agent that misbehaves.
//!
//! ```text
//! cargo run --example acp_probe -- opencode acp -- "What is 2+2?"
//! cargo run --example acp_probe -- gemini --experimental-acp -- "Say hi"
//! ```

use std::sync::mpsc as std_mpsc;
use std::time::Duration;

use klets_lib::agent::{run_agent_thread, AgentCommand, LaunchPlan};
use klets_lib::events::{AgentEvent, EventSink};
use tokio::sync::mpsc;

#[derive(Clone)]
struct PrintSink {
    finished: std_mpsc::Sender<()>,
    /// Answers approval prompts the way the UI would.
    broker: klets_lib::permissions::PermissionBroker,
    /// Tool kinds to approve, from KLETS_APPROVE.
    approve: Vec<String>,
}

impl EventSink for PrintSink {
    fn emit(&self, event: AgentEvent) {
        match event {
            AgentEvent::Status { state, detail, .. } => {
                println!("[status] {state:?}{}", detail.map(|d| format!(" — {d}")).unwrap_or_default());
            }
            AgentEvent::Chunk { text, .. } => print!("{text}"),
            AgentEvent::Thought { text, .. } => eprint!("\x1b[2m{text}\x1b[0m"),
            AgentEvent::Tool { title, status, .. } => println!("\n[tool] {title} ({status})"),
            AgentEvent::PermissionDenied { title, kind, .. } => {
                println!("\n[denied] {title} ({kind})")
            }
            AgentEvent::PermissionRequest { request, .. } => {
                let allow = self.approve.iter().any(|k| k == &request.kind);
                println!(
                    "\n[ask] {} ({}){} -> {}",
                    request.title,
                    request.kind,
                    request
                        .detail
                        .as_ref()
                        .map(|d| format!(": {d}"))
                        .unwrap_or_default(),
                    if allow { "ALLOW" } else { "DENY" }
                );
                self.broker.resolve(request.id, allow);
            }
            AgentEvent::PermissionResolved { timed_out, .. } if timed_out => {
                println!("\n[ask] timed out")
            }
            AgentEvent::PermissionResolved { .. } => {}
            AgentEvent::Capabilities { commands, .. } => {
                println!("[capabilities] {} commands available", commands.len());
                for c in commands.iter().take(8) {
                    println!("    /{}", c.name);
                }
            }
            AgentEvent::Notice { message, .. } => println!("\n[notice] {message}"),
            AgentEvent::Done { stop_reason, .. } => {
                println!("\n[done] {}", stop_reason.unwrap_or_default());
                let _ = self.finished.send(());
            }
            AgentEvent::Error { message, .. } => {
                println!("\n[error] {message}");
                let _ = self.finished.send(());
            }
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let split = args.iter().position(|a| a == "--");
    let (command_parts, prompt) = match split {
        Some(index) => (
            &args[..index],
            args[index + 1..].join(" "),
        ),
        None => (&args[..], "Reply with exactly: klets ok".to_string()),
    };

    let Some((binary, rest)) = command_parts.split_first() else {
        eprintln!("usage: cargo run --example acp_probe -- <binary> [args...] -- <prompt>");
        std::process::exit(2);
    };

    let cwd = std::env::temp_dir().join("klets-probe");
    std::fs::create_dir_all(&cwd).expect("workspace");

    // Secrets are read straight from the Klets keychain and injected into the
    // child, so a probe run never has to echo a key onto a command line.
    let mut env = Vec::new();
    if let (Ok(key_id), Ok(var)) = (std::env::var("KLETS_KEY_ID"), std::env::var("KLETS_KEY_ENV")) {
        match klets_lib::settings::get_api_key(&key_id) {
            Some(secret) => {
                println!("[auth] injecting {var} from keychain entry '{key_id}'");
                env.push((var, secret));
            }
            None => println!("[auth] no keychain entry '{key_id}' — continuing without it"),
        }
    }

    let unset_env: Vec<String> = std::env::var("KLETS_UNSET")
        .map(|raw| raw.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    if !unset_env.is_empty() {
        println!("[auth] clearing inherited {}", unset_env.join(", "));
    }

    let plan = LaunchPlan {
        provider_id: binary.clone(),
        provider_name: binary.clone(),
        binary: klets_lib::providers::resolve_binary(binary)
            .unwrap_or_else(|| panic!("'{binary}' is not on PATH")),
        args: rest.to_vec(),
        env,
        unset_env,
        cwd,
        model: std::env::var("KLETS_MODEL").ok(),
        tool_policy: match std::env::var("KLETS_POLICY").as_deref() {
            Ok("off") => klets_lib::permissions::ToolPolicy::Off,
            Ok("readonly") => klets_lib::permissions::ToolPolicy::ReadOnly,
            _ => klets_lib::permissions::ToolPolicy::AskToRun,
        },
        system_prompt: std::env::var("KLETS_PROMPT").unwrap_or_default(),
        select_model_over_acp: std::env::var("KLETS_MODEL_ARG").is_err(),
        login_hint: format!("{binary} login"),
    };

    println!("[launch] {} {:?}", plan.binary.display(), plan.args);

    let (finished_tx, finished_rx) = std_mpsc::channel();
    let (tx, rx) = mpsc::unbounded_channel();
    let broker = klets_lib::permissions::PermissionBroker::new();
    let sink = PrintSink {
        finished: finished_tx,
        broker: broker.clone(),
        approve: std::env::var("KLETS_APPROVE")
            .map(|raw| raw.split(',').map(|s| s.trim().to_string()).collect())
            .unwrap_or_default(),
    };

    let worker = std::thread::spawn(move || run_agent_thread(sink, plan, broker, rx));

    tx.send(AgentCommand::Prompt { turn: 1, text: prompt })
        .expect("send prompt");

    match finished_rx.recv_timeout(Duration::from_secs(180)) {
        Ok(()) => println!("[probe] turn complete"),
        Err(_) => println!("[probe] timed out"),
    }

    let _ = tx.send(AgentCommand::Shutdown);
    drop(tx);
    let _ = worker.join();
}
