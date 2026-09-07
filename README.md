# Klets

A Spotlight-style launcher for quick answers from ACP coding agents. Press
`Ctrl+Space`, ask a question, read the answer as rendered markdown. Press `Esc`
and it's gone.

Klets is an [Agent Client Protocol](https://agentclientprotocol.com) client. It
doesn't talk to model APIs directly — it spawns an agent you already have
installed and speaks ACP (JSON-RPC 2.0 over stdio) to it, the same way Zed and
JetBrains do. Whatever that agent is signed in to is what answers.

## Providers

| Provider | Binary | Install |
|---|---|---|
| Claude | `claude-agent-acp` | `npm i -g @agentclientprotocol/claude-agent-acp` |
| Codex | `codex-acp` | `npm i -g @agentclientprotocol/codex-acp` |
| Gemini | `gemini --experimental-acp` | `npm i -g @google/gemini-cli` |
| OpenCode Go | `opencode acp` | `curl -fsSL https://opencode.ai/install \| bash` |
| OpenCode Zen | `opencode acp` | same binary as Go |

OpenCode Go and Zen are one binary told apart by model id — `opencode-go/…`
versus `opencode/…` — which Klets sets per session.

Agents must be installed yourself; Klets detects them on PATH and shows the
install command for the ones it can't find.

### Authentication

An API key is **optional**. Klets checks three sources, cheapest first, and
never spawns an agent to find out:

1. A key saved in Klets (stored in the OS keychain, never on disk in plain text)
2. An API key already exported in your environment
3. The credential file the agent's own `login` flow writes

So if you already run `opencode`, `claude`, or `codex` in a terminal, Klets
works with no setup. Credential files can't reveal that a token has expired, so
if an agent refuses the session Klets shows the failure with the sign-in command
for that provider.

## Safe by design

Klets is a question box, not a coding session. It declares **no filesystem and
no terminal capabilities** over ACP, and refuses every tool permission request,
noting it in the transcript. Sessions run in a scratch directory in the app data
folder, never a real project. Agents can answer, but they can't touch anything.

## Using it

| Key | Action |
|---|---|
| `Ctrl+Space` | Show or hide the launcher (rebindable) |
| `Enter` | Send |
| `Shift+Enter` | Newline |
| `Esc` | Stop a running answer, clear the input, then hide |
| `Ctrl+N` | New chat |
| `Ctrl+,` | Settings |

The conversation stays alive while Klets runs, so you can hide the window and
come back to it. Nothing is written to disk. Links open in your browser, and
code blocks have a copy button.

## Development

Requires Rust and Node 18+.

```bash
npm install
npm run tauri dev      # run
npm run tauri build    # Windows installer -> src-tauri/target/release/bundle/nsis
```

### Probing an agent

`acp_probe` runs the app's real agent pipeline headlessly — process spawn, ACP
handshake, session, streaming prompt — and prints every event. Use it to check a
provider works, or to debug one that doesn't:

```bash
cd src-tauri
cargo run --example acp_probe -- opencode acp -- "What is 2+2?"
cargo run --example acp_probe -- gemini --experimental-acp -- "Say hi"
KLETS_MODEL=opencode-go/kimi-k3 cargo run --example acp_probe -- opencode acp
```

`scripts/acp-smoke.mjs` does the same at the raw JSON-RPC level, for when you
need to see the wire traffic itself.

## Architecture

```
src/                    Vue 3 + TypeScript launcher and settings UI
  services/ipc.ts       every call into Rust
  stores/               Pinia (chat transcript, settings)
src-tauri/src/
  agent.rs              process spawn + ACP connection, one thread per agent
  providers.rs          the five providers, PATH resolution, auth detection
  events.rs             EventSink — what the pipeline streams out
  settings.rs           settings file + OS keychain
  windows.rs            launcher positioning and show/hide
```

The ACP client connection is `!Send`, so each agent runs on its own thread with
a current-thread Tokio runtime and a `LocalSet`; the app talks to it over an
mpsc channel. Prompts are dispatched as concurrent tasks so `session/cancel`
still lands mid-turn.

Event emission goes through the `EventSink` trait rather than `AppHandle`, which
is what lets `acp_probe` drive the same pipeline without a running app.
