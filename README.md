# Klets

**A Spotlight-style launcher for asking your AI coding agent a quick question
— without opening an editor.**

Press a hotkey. A floating bar appears. Type a question. Get an answer as
rendered markdown, streamed in real time, with the same tools and knowledge
your agent already has. Press `Esc`. It's gone. No new window in your taskbar,
no editor to open, no context to lose.

*(Klets is Afrikaans/Dutch for "chat".)*

Klets doesn't run its own model or call any API directly. It spawns whichever
coding agent you already have installed and signed in to — **Claude Code,
Codex, Gemini, or OpenCode** — and speaks
[Agent Client Protocol](https://agentclientprotocol.com) (JSON-RPC 2.0 over
stdio) to it, the same protocol Zed and JetBrains use. Whatever subscription
or API key already authenticates that agent is what answers — Klets adds no
billing of its own.

> **Status:** early-stage personal project, built and tested on Windows only.
> The stack (Tauri) architecturally supports macOS and Linux, but neither has
> been built or run — treat that as unverified. Issues and feedback welcome.

- **No editor to open.** A global hotkey opens a floating bar over whatever
  you're doing; answering closes it again.
- **Uses your existing agent, not a new subscription.** Claude Code, Codex,
  Gemini, or OpenCode — whichever you already run, however it's already
  authenticated (subscription login or an API key).
- **Reads real files and runs commands, with a policy you control.** Off,
  read-only, or ask-before-running — see [Tools](#tools) below. Edits are
  always refused.
- **Markdown that actually renders**, with syntax-highlighted code, a copy
  button per block, and links that open in your browser instead of hijacking
  the launcher.
- **Nothing persists.** No chat history on disk, no telemetry, no account.

## Providers

| Provider | Binary | Install |
|---|---|---|
| Claude | `claude-agent-acp` | `npm i -g @agentclientprotocol/claude-agent-acp` |
| Codex | `codex-acp` | `npm i -g @agentclientprotocol/codex-acp` |
| Gemini | `gemini --acp` | `npm i -g @google/gemini-cli` |
| OpenCode Go | `opencode acp` | `curl -fsSL https://opencode.ai/install \| bash` |
| OpenCode Zen | `opencode acp` | same binary as Go |

OpenCode Go and Zen are one binary told apart by model id — `opencode-go/…`
versus `opencode/…` — which Klets sets per session.

**Gemini needs an auth override.** gemini-cli stores its sign-in method in
`~/.gemini/settings.json`, and the default `oauth-personal` authenticates
against Gemini Code Assist for individuals, which Google has discontinued.
There is no flag to switch it, and in ACP mode there is no interactive prompt to
fall back to, so the session simply fails. Klets writes a private settings file
pinning `gemini-api-key` and passes it as gemini-cli's *system* settings, which
outrank user and workspace settings and are not gated behind folder trust. Your
own `~/.gemini/settings.json` is never touched.

Routing Gemini through OpenCode also works and needs no override, but OpenCode's
catalogue currently stops at `gemini-3.1-flash-lite`, whereas gemini-cli talks to
Google directly and reaches current models.

Agents must be installed yourself; Klets detects them on PATH and shows the
install command for the ones it can't find.

### Authentication

Each provider has an explicit **sign-in method**, chosen in settings rather than
inferred:

- **Subscription** — the agent's own login. Klets injects nothing and actively
  clears any inherited API key, so a Claude Pro/Max/Team login is never silently
  switched onto per-token API billing.
- **API key** — a key you paste into Klets, stored in the OS keychain and
  injected only into that agent's process.

Claude, Codex and OpenCode support both. Gemini is API-key only.

A provider is offered in the launcher once it is installed and can authenticate.
Credential files can't reveal that a token has expired, so if an agent refuses
the session Klets reports it with the sign-in command for that provider.

## Tools

Agents use their own built-in tools (reading files, searching, fetching URLs,
running commands) and just need Klets to say yes. Every `session/request_permission`
is decided by a policy keyed on the tool's kind, in **Settings → Tools**:

| Policy | Read, search, fetch | Edit, delete, move | Run a command |
|---|---|---|---|
| Off | Refused | Refused | Refused |
| Read-only | Allowed | Refused | Refused |
| Ask to run (default) | Allowed | Refused | Asks in the transcript |

Edits are always refused outright — Klets is a question box, not a coding
session — with a note in the transcript explaining why. An unanswered request
refuses itself after 60 seconds, and pending requests bring the launcher
forward if it's hidden, since answering one is exactly when you want to see it.

**This is a policy layer over what an agent chooses to ask about, not a
sandbox.** Claude Code (and likely the others) classifies some commands as
trivially safe and runs them without consulting the client at all — `echo hi`
never reaches Klets, while `rm` does. What Klets refuses, it refuses
absolutely; what an agent auto-approves internally happens regardless of the
policy set here.

**Working directory** (Settings → Tools) is where agents run and what they can
read. It defaults to a folder Klets owns in its app-data directory — empty on
first run, so a fresh install starts with nothing to read regardless of
policy. Pointing it at a real folder is a deliberate, explicit choice.

**System prompt** (Settings → Tools) is written into the working directory as
`AGENTS.md`, `CLAUDE.md` and `GEMINI.md` at the start of every session — each
agent reads a different filename, so the same text goes under all three.
Existing files in that directory are never overwritten, and Klets' own copies
are removed when the session ends.

**Tool discovery.** The launcher shows a small "N tools" chip once an agent has
answered at least once, listing whatever it reports as available. This is the
agent's *own* MCP servers and skills — the ones already configured in its own
`opencode.json`, Claude Code settings, and so on — not anything Klets
provisions. It differs per provider (OpenCode commonly reports ~100, Gemini
around 20) and resets when you switch providers or start a new chat.

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

### Tests

```bash
cd src-tauri && cargo test              # Rust unit tests — no agent binaries needed
cd src-tauri && cargo clippy --all-targets

npm run test                            # Vitest — Pinia store logic, markdown rendering
npx vue-tsc --noEmit                    # frontend typecheck
```

Rust tests cover PATH/`.cmd`-shim resolution, auth-mode fallback, settings
backward-compatibility, the full tool-policy decision matrix, the permission
broker, and the exact JSON strings the frontend depends on. Frontend tests
cover the chat/settings store state machines (with the Tauri bridge mocked
out) and the markdown renderer's security-relevant behavior — agent output is
untrusted content rendered with `v-html`, so no raw HTML may ever pass
through. See [`AGENTS.md`](AGENTS.md) for what each test file actually locks
down and why.

### Probing an agent

`acp_probe` runs the app's real agent pipeline headlessly — process spawn, ACP
handshake, session, streaming prompt — and prints every event. Use it to check a
provider works, or to debug one that doesn't:

```bash
cd src-tauri
cargo run --example acp_probe -- opencode acp -- "What is 2+2?"
cargo run --example acp_probe -- claude-agent-acp -- "Say hi"
```

It reads the same environment Klets would build:

| Variable | Effect |
|---|---|
| `KLETS_MODEL` | Model to request for the session |
| `KLETS_KEY_ID` | Klets keychain entry to read a secret from |
| `KLETS_KEY_ENV` | Variable to inject that secret as |
| `KLETS_UNSET` | Comma-separated variables to clear before spawning |
| `KLETS_POLICY` | `off` / `readonly` / anything else = ask-to-run (the default) |
| `KLETS_PROMPT` | System prompt, written as `AGENTS.md`/`CLAUDE.md`/`GEMINI.md` |
| `KLETS_APPROVE` | Comma-separated tool kinds to auto-answer Allow for prompts |

`KLETS_APPROVE` answers `[ask]` prompts the way the settings UI would, so you
can prove policy end to end without a GUI — including that a request for a
kind *not* in the list gets denied rather than silently allowed:

```bash
KLETS_APPROVE=read,search,fetch,execute \
  cargo run --example acp_probe -- claude-agent-acp -- "run: echo hi"
```

`KLETS_KEY_ID` pulls straight from the keychain so a probe run never needs a key
on the command line:

```bash
KLETS_KEY_ID=gemini KLETS_KEY_ENV=GOOGLE_GENERATIVE_AI_API_KEY \
  KLETS_MODEL=google/gemini-3.1-flash-lite \
  cargo run --example acp_probe -- opencode acp -- "Say hi"

# Prove an agent uses its subscription rather than an API key
KLETS_UNSET=ANTHROPIC_API_KEY cargo run --example acp_probe -- claude-agent-acp
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
  permissions.rs        tool policy + the broker that carries decisions to it
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

## Contributing

There's no formal process yet — issues and pull requests are welcome. Read
[`AGENTS.md`](AGENTS.md) first: it's written for exactly this, covering the
architecture decisions behind the code (and why they're shaped that way), the
project's conventions, and how to verify a change against a real agent without
clicking through the GUI.

## License

[MIT](LICENSE) © FJ Lessing
