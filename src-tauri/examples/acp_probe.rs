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
            AgentEvent::PermissionDenied { title, .. } => println!("\n[denied] {title}"),
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

    let plan = LaunchPlan {
        provider_id: binary.clone(),
        provider_name: binary.clone(),
        binary: klets_lib::providers::resolve_binary(binary)
            .unwrap_or_else(|| panic!("'{binary}' is not on PATH")),
        args: rest.to_vec(),
        env: Vec::new(),
        cwd,
        model: std::env::var("KLETS_MODEL").ok(),
        login_hint: format!("{binary} login"),
    };

    println!("[launch] {} {:?}", plan.binary.display(), plan.args);

    let (finished_tx, finished_rx) = std_mpsc::channel();
    let (tx, rx) = mpsc::unbounded_channel();
    let sink = PrintSink { finished: finished_tx };

    let worker = std::thread::spawn(move || run_agent_thread(sink, plan, rx));

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
