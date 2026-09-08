# AGENTS.md

Context for any coding agent (human-directed or autonomous) working in this
repository. If you're a human, `README.md` is probably what you want instead —
this file is deliberately more mechanical and more honest about rough edges.

## What this is

Klets is a Spotlight-style desktop launcher: a global hotkey opens a floating
input bar, you type a question, and an ACP (Agent Client Protocol) agent
answers as streamed markdown. It is **not** a coding assistant UI — it's a
quick-answer box that happens to be able to let an agent read real files and
run commands, gated by an explicit permission policy.

Klets does not talk to any model API directly. It spawns an agent binary you
already have installed (Claude Code, Codex, gemini-cli, or OpenCode) and
speaks ACP — JSON-RPC 2.0 over stdio — to it, the same protocol Zed and
JetBrains use. Whatever that agent is signed in to is what answers.

Stack: Tauri 2 (Rust core) + Vue 3 + TypeScript (frontend). Developed on
Windows; builds, tests, and runs against a real agent on macOS (arm64) as
well, though macOS has had far less use. Single developer, no CI yet.

## Directory map

```
src/                          Vue 3 + TypeScript, two windows sharing one bundle
  App.vue                     Picks LauncherView or SettingsView by window label
  views/LauncherView.vue      The floating bar: input, transcript, provider/tools menus
  views/SettingsView.vue      Providers, auth mode, tool policy, prompt, working dir
  components/app/
    ChatMessageItem.vue       One transcript bubble: text, tools, permission rows
    MarkdownView.vue          Renders one message's markdown, delegates link/copy clicks
  stores/
    chatstore.ts              The turn/transcript state machine — see "Event ordering" below
    settingsstore.ts          Settings + provider status, mirrors the Rust AppSnapshot
  services/ipc.ts             The only file allowed to call `invoke`/`listen` directly
  helpers/
    types.ts                  TypeScript mirror of the Rust IPC/event contract
    markdown.ts               MarkdownIt config: external-link tagging, copy buttons

src-tauri/src/
  agent.rs                    Process spawn + ACP connection; one OS thread per agent
  providers.rs                The provider registry (data, not branches) + PATH/auth logic
  permissions.rs              ToolPolicy + the broker that carries UI decisions to the agent thread
  settings.rs                 Settings file (JSON) + OS keychain access
  commands.rs                 Every #[tauri::command] — the IPC surface
  events.rs                   AgentEvent (the wire format) + the EventSink trait
  windows.rs / shortcuts.rs   Window positioning/show-hide, global hotkey registration
  lib.rs                      Wiring: plugins, tray, window events, first-run logic
  main.rs                     Thin entry point only — put nothing else here

src-tauri/examples/acp_probe.rs   Headless CLI that drives the real agent pipeline;
                                  the primary way to verify a change without clicking a GUI
scripts/acp-smoke.mjs             Raw JSON-RPC probe, for inspecting wire traffic directly
```

## Core architecture, and why it's shaped this way

**The ACP connection is `!Send`.** `agent_client_protocol::ClientSideConnection`
is built on `LocalBoxFuture`, so it cannot live in Tauri's shared, thread-hopping
state. Each connected agent therefore gets its **own OS thread**, running a
current-thread Tokio runtime plus a `LocalSet` (`agent.rs::run_agent_thread`).
The rest of the app talks to that thread over an `mpsc` channel of
`AgentCommand` (`Prompt`, `Cancel`, `NewSession`, `Shutdown`). `AgentManager`
(Tauri-managed state) owns the channel sender and the `PermissionBroker`.

**Event emission goes through the `EventSink` trait, not `AppHandle` directly**
(`events.rs`). This is the single most load-bearing design decision in the
codebase: it lets `acp_probe` drive the *exact* production code path
(`drive_agent`, `KletsClient`, model selection, instruction-file writing —
everything) against a real agent binary, headlessly, with no Tauri app running
at all. If you add a code path that needs to reach the UI, make it take
`&impl EventSink`, not `&AppHandle`. Breaking this coupling is the easiest way
to make the codebase untestable again.

**The provider registry is declarative data, not branching code**
(`providers.rs::PROVIDERS`, a `&[ProviderSpec]`). Adding a sixth agent should
mean adding one array entry, not new `if`/`match` arms scattered through
`agent.rs`. Notable fields on `ProviderSpec`: `model_flag` (for agents with no
ACP model-selection, so the model goes on the command line instead) and
`auth_settings_override` (for agents that pin their sign-in method in a config
file with no CLI flag — currently only gemini-cli).

**Auth mode is explicit, never inferred** (`settings::AuthMode`). Injecting an
API key at an agent that's signed in to a subscription would silently switch
it to per-token billing — a real footgun this design specifically closes.
`AuthMode::Subscription` mode actively **clears** any inherited API key env
var (`command.env_remove`) before spawning, rather than merely not setting one,
because a shell that already exports e.g. `ANTHROPIC_API_KEY` would otherwise
leak through via the child's inherited environment.

**Tool permission enforcement is a single choke point:**
`KletsClient::request_permission` in `agent.rs`, gated by `ToolPolicy::decide`
in `permissions.rs`. There is no other place agent tool use is checked.
**Important limitation, not a bug:** agents self-approve some tool calls
internally and never ask the client at all — confirmed empirically: Claude
Code runs `echo hi` without consulting Klets, but asks before `rm`. Klets is a
policy layer over *what an agent chooses to ask about*, not a sandbox. Do not
describe this as "Klets prevents X" in any UI copy or docs — say "Klets
refuses X when asked."

**Permission decisions cross a thread boundary via a `oneshot` broker**
(`permissions::PermissionBroker`). `request_permission` registers a pending
decision, emits `AgentEvent::PermissionRequest`, and awaits the receiver with a
60s timeout (`DECISION_TIMEOUT`) so a request raised while the launcher is
hidden can't stall a turn forever. `respond_permission` (a Tauri command) is
the only way anything resolves it. `AgentManager::cancel`/`stop` call
`broker.clear()`, which force-denies everything outstanding — see the "Event
ordering" note below for why the frontend has to handle this race correctly.

**Model selection is dual-path with read-back verification**
(`agent.rs::select_model`). Some agents use `session/set_model`; OpenCode uses
a `session/set_config_option` with `category: Model` instead (this is also how
OpenCode Go, Zen, and — historically — Gemini-via-OpenCode were told apart,
being the same binary with different model ids). The response is **read back**
and compared against what was requested, because agents don't always reject an
unknown model — some silently keep their own default, which would otherwise
have the UI claim one model answered while another one actually did. If the
model didn't apply, an `AgentEvent::Notice` says so, naming both models.

**The system prompt is written to disk, not injected over the protocol.**
There is no ACP field for a system prompt. `agent.rs::write_instructions`
writes the same text as `AGENTS.md`, `CLAUDE.md`, and `GEMINI.md` into the
session's working directory before the agent starts (each agent reads a
different filename), skips any file that already exists (never clobbers real
project instructions), and deletes its own copies on shutdown.

**Windows `.cmd`/PATHEXT resolution is a real, previously-hit bug.** npm
installs global binaries as `.cmd` shims; a bare `Command::new("claude-agent-acp")`
does not resolve them. `providers::resolve_binary` walks `PATHEXT` manually.
Separately, gemini-cli ships a `.ps1` beside its `.cmd` — `.ps1` must **not**
match (Windows can't `CreateProcess` a PowerShell script directly), which is
exactly what `providers::tests::find_on_path_picks_the_cmd_shim_over_a_same_named_ps1`
locks in.

**macOS has three things Windows never needed.** (1) A Finder/Dock-launched
app inherits launchd's minimal `PATH`, not the shell's, so every agent binary
would resolve as "not installed" and npm shims would fail on `#!/usr/bin/env
node`. `providers::inherit_login_shell_path` asks `$SHELL -ilc` for the real
PATH and rewrites the process's own, first thing in `run()` while it is still
single-threaded (env mutation is not thread-safe on Unix). (2) `keyring` needs
the `apple-native` feature; without it, it silently uses an in-memory mock
that reports every save as successful and forgets it. (3) `skipTaskbar` is
unsupported, so `lib.rs` sets `ActivationPolicy::Accessory` (no Dock icon, no
Cmd+Tab entry) and `transparent: true` needs `macOSPrivateApi` plus the
`macos-private-api` Cargo feature or the launcher renders as an opaque box.

**Linux needs the same keyring treatment, just a different feature.**
`Cargo.toml` also enables `linux-native-sync-persistent` (Secret Service —
GNOME Keyring/KWallet — falling back to the kernel keyring when no Secret
Service is running) and `crypto-rust` (so the Linux build doesn't need
libssl-dev). The Linux-only dependencies these pull in are gated on
`target_os = "linux"` inside the `keyring` crate itself, so they add nothing
to the macOS or Windows binary — confirmed by `cargo build` on macOS only
adding them to `Cargo.lock`, never compiling them.

**Claude Code's macOS credentials are in the Keychain, and Klets now checks
for them without ever reading the secret.** `providers::keychain_item_exists`
shells out to `/usr/bin/security find-generic-password -s <service>` and
looks only at the exit status — deliberately never passing `-w`, which would
read the stored secret and trigger the OS's "klets wants to access..."
prompt for a value Klets never needed. `ProviderSpec::credential_keychain_services`
is declarative data like everything else in the registry (empty for every
provider except Claude), checked in `is_authenticated` alongside
`credential_paths`, and a no-op returning `false` on non-macOS targets.

**Window resize is done in physical pixels, on purpose.** `resize_launcher`
converts height only, in physical pixels, and passes the existing width
through untouched. Converting width to logical units and back does not
round-trip cleanly under fractional DPI scaling; doing that on every streamed
token caused a visible horizontal jitter. The frontend also coalesces height
requests to one per animation frame (`scheduleWindowHeight` in
`LauncherView.vue`) and skips no-op resizes on the Rust side.

## Event ordering matters more than it looks (`stores/chatstore.ts`)

`handleEvent` processes events in a specific, deliberate order, and getting
this wrong silently loses UI state rather than crashing:

1. `status` — handled first, independent of any message.
2. `capabilities` — independent of any message.
3. `notice` — attaches to `currentMessage` **before** the streaming guard.
4. `permissionRequest` / `permissionResolved` — **also before** the streaming
   guard. This was a real bug: cancelling a turn (or the agent crashing)
   resolves any outstanding permission via `broker.clear()`, which **races**
   against the `done`/`error` event that closes the message. If that race is
   lost and this check were after the guard, the resolution is silently
   dropped and the UI is left showing live Allow/Deny buttons for a decision
   that's already been made. See
   `chatstore.test.ts > "resolves a pending permission even when it arrives
   after the turn has closed"` — this test fails if the ordering regresses.
5. Only *then* — `if (!message.streaming) return;` — the guard against a
   *content* event (chunk/thought/tool) reopening an already-closed bubble.

If you add a new event type, decide deliberately which side of that guard it
belongs on, and write the test for it, not just the happy path.

## Conventions

**Rust:** `cargo clippy --all-targets` must be clean before considering work
done — it has been at every point in this project's history; don't let it
regress. Doc comments explain *why*, not *what* (re-read any file in
`src-tauri/src/` for the house style before adding more). Tests live in
`#[cfg(test)] mod tests` at the bottom of the file they test, using free
functions extracted from public methods when a signature needs to be
deterministic (see `resolve_working_dir`, `find_on_path`, `parse_pathext` —
all pulled out specifically so tests don't have to mutate global process state
like `PATH`/`PATHEXT`/`cwd`, which is unsafe under parallel test execution).

**Vue/TypeScript:** Composition API, `<script setup lang="ts">` only. **Tabs**
for indentation, not spaces — check any existing file before assuming
otherwise. Semicolons on every statement. Pinia stores use the **Options
Store** shape (`state`/`getters`/`actions`), not `setup` stores. No Tailwind —
styling is scoped `<style scoped>` per component plus CSS custom properties in
`src/assets/css/main.css`; use `rem`/`em`/`%`, never raw `px`. `IpcService`
(`services/ipc.ts`) is the *only* file that imports `@tauri-apps/api/*` or
`@tauri-apps/plugin-*` directly — stores and components go through it, which
is also what makes them mockable in tests (`vi.mock("@/services/ipc", ...)`).

**Commits:** no AI attribution, no "Generated with"/"Co-Authored-By" trailers,
regardless of which agent or tool made the change.

## Building, running, testing

```bash
npm install                          # once
npm run tauri dev                    # run the app
npm run tauri build                  # release build: NSIS/MSI on Windows, .app/.dmg on macOS

cd src-tauri && cargo test           # Rust unit tests (fast, no agent needed)
cd src-tauri && cargo clippy --all-targets
npm run test                         # Vitest — frontend store/helper logic
npx vue-tsc --noEmit                 # frontend typecheck
```

Rust tests currently cover: PATH/PATHEXT resolution, auth-mode fallback and
settings backward-compatibility (a real existing `settings.json` predates
several fields — see `settings::tests::old_settings_file_gets_safe_new_defaults`),
the full `ToolPolicy::decide` matrix, the permission broker, and the exact
JSON tag strings the frontend's TypeScript types depend on (`auth_mode_serializes_to_the_exact_strings_the_frontend_expects` and
similar — **if you rename a Rust enum variant's serde representation, update
`src/helpers/types.ts` in the same change, or these tests will pass while the
frontend silently breaks**, since nothing currently checks the two sides
against each other automatically).

Frontend tests cover the chat/settings store state machines with
`IpcService` mocked out (see `chatstore.test.ts`, `settingsstore.test.ts`) and
the markdown renderer's security-relevant behavior (`markdown.test.ts`: no raw
HTML ever renders, since agent output is untrusted content piped through
`v-html`).

### Verifying against a real agent (no GUI needed)

`acp_probe` runs the actual production pipeline — spawn, ACP handshake,
session, streaming prompt, permission handling — against a real installed
agent binary, and prints every event. This is the primary way to verify a
change to `agent.rs`, `providers.rs`, or `permissions.rs` without a running
app or clicking through a GUI:

```bash
cd src-tauri
cargo run --example acp_probe -- claude-agent-acp -- "Say hi"

# Read the same environment Klets would build:
KLETS_MODEL=...            # model to request
KLETS_KEY_ID=... KLETS_KEY_ENV=...   # pull a secret from the keychain, inject as this var
KLETS_UNSET=VAR1,VAR2      # clear inherited vars before spawning
KLETS_POLICY=off|readonly|<anything else = ask-to-run>
KLETS_PROMPT="..."         # written as AGENTS.md/CLAUDE.md/GEMINI.md
KLETS_APPROVE=read,search,fetch,execute   # auto-answer Allow for these kinds when asked
```

`KLETS_APPROVE` is how policy gets verified end to end: a kind *not* in the
list gets denied rather than silently allowed, so you can prove a refusal as
well as an approval. `KLETS_KEY_ID`/`KLETS_KEY_ENV` read straight from the OS
keychain, so a probe run never needs a secret on the command line or in shell
history.

`scripts/acp-smoke.mjs` is a much lower-level fallback: raw JSON-RPC over
stdio with no Rust involved at all, for when you need to see literal wire
traffic while debugging an adapter itself rather than Klets' handling of it.

## Things that look wrong but are documented, intentional behavior

- **OpenCode Go can hang with zero output.** Verified (during development)
  that this reproduces identically via bare `opencode run`, with no Klets or
  ACP involved — it's an account/quota condition on OpenCode's side, not a
  bug here. Don't "fix" this by adding retry/timeout logic without
  re-confirming the underlying service is actually the cause.
- **Gemini does not use gemini-cli's own OAuth.** `oauth-personal` mode
  authenticates against a product Google discontinued for individuals, with
  no CLI flag to switch and no interactive fallback in ACP mode. Gemini is
  therefore **API-key only** (`auth_modes: &[AuthMode::ApiKey]`), and Klets
  writes a private `GEMINI_CLI_SYSTEM_SETTINGS_PATH` settings file to force
  `gemini-api-key` mode — system settings outrank user/workspace settings and
  aren't gated behind folder trust, and your real `~/.gemini/settings.json`
  is never touched.
- **A stale credential file can't prove a login still works.** `is_authenticated`
  is a cheap, offline check (keychain entry, env var, or a known credential
  file's mere *existence*). It cannot detect an expired or revoked token —
  that's only discovered when a real session fails, which is why auth
  failures get recovery instructions appended (`agent.rs::explain`) rather
  than being treated as a plain error.

## Known gaps / not yet done

- No CI configured.
- No frontend component/DOM tests (Vitest tests are store- and helper-level
  logic only, run in a Node environment — no `jsdom`, no rendering).
- Linux has never been built or run; the `keyring` Secret Service feature and
  the declarative provider registry should carry over cleanly (nothing in
  `providers.rs` is macOS/Windows-specific data), but PATH inheritance under
  a Linux desktop launcher, the global hotkey, and tray behavior are all
  unverified. macOS builds, passes all tests, and completes real agent turns
  via `acp_probe` and the bundled `.app`, but the GUI has had comparatively
  little hands-on use there. Known macOS rough edges, not yet addressed: the
  default `Ctrl+Space` hotkey is Control+Space (collides with the
  input-source switcher if more than one keyboard layout is enabled); the
  tray icon is not a monochrome template image; the launcher does not join
  all Spaces, so it won't appear over a full-screen app; `Cmd+Q` bypasses
  `WindowEvent::CloseRequested`, so `AgentManager::stop` is not called on
  that path.
- Single active session per provider; no persisted conversation history.
- MCP servers are **discovered**, not configured — Klets shows whatever an
  agent already reports (`AgentEvent::Capabilities`, driven by ACP's
  `session/update` → `available_commands_update`), but has no UI to add one
  itself.

## Secrets

Everything goes through the OS keychain (`keyring` crate on the Rust side,
`settings::get_api_key`/`set_api_key`). Never print, log, or write a real key
or token to a file, commit, or test fixture. `acp_probe`'s `KLETS_KEY_ID`
mechanism exists specifically so verification never needs one typed anywhere
in plaintext.
