<script lang="ts" setup>
import { computed, onMounted, reactive, ref } from "vue";
import { IpcService } from "@/services/ipc";
import { useSettingsStore } from "@/stores/settingsstore";
import { AuthMode, ToolPolicy, type ProviderStatus } from "@/helpers/types";

const settingsStore = useSettingsStore();

const keyDrafts = reactive<Record<string, string>>({});
const modelDrafts = reactive<Record<string, string>>({});
const capturingHotkey = ref(false);
const promptDraft = ref("");

const TOOL_POLICIES: Array<{ value: ToolPolicy; label: string; description: string }> = [
	{ value: ToolPolicy.Off, label: "Off", description: "Answers come from the model alone." },
	{
		value: ToolPolicy.ReadOnly,
		label: "Read-only",
		description: "Reads, searches and fetches are allowed. Nothing runs.",
	},
	{
		value: ToolPolicy.AskToRun,
		label: "Ask to run",
		description: "Reads happen freely; running a command asks first.",
	},
];

const activePolicy = computed(
	() => TOOL_POLICIES.find((p) => p.value === settingsStore.settings?.toolPolicy) ?? TOOL_POLICIES[2],
);

const promptDirty = computed(
	() => promptDraft.value.trim() !== (settingsStore.settings?.systemPrompt ?? "").trim(),
);

async function refresh() {
	await settingsStore.load();
	settingsStore.providers.forEach((provider) => {
		modelDrafts[provider.id] = provider.model ?? provider.defaultModel ?? "";
	});
	promptDraft.value = settingsStore.settings?.systemPrompt ?? "";
}

onMounted(refresh);

async function setToolPolicy(policy: ToolPolicy) {
	await settingsStore.save({ toolPolicy: policy });
}

async function savePrompt() {
	await settingsStore.save({ systemPrompt: promptDraft.value });
}

async function resetPrompt() {
	promptDraft.value = settingsStore.defaultSystemPrompt;
	await settingsStore.save({ systemPrompt: promptDraft.value });
}

async function browseWorkingDir() {
	const chosen = await IpcService.pickFolder(settingsStore.settings?.workingDir);
	if (chosen) await settingsStore.setWorkingDir(chosen);
}

async function clearWorkingDir() {
	await settingsStore.setWorkingDir(null);
}

function statusLabel(provider: ProviderStatus): string {
	if (!provider.binaryPath) return "Not installed";
	if (!provider.authenticated) {
		return provider.authMode === AuthMode.ApiKey ? "Needs API key" : "Needs sign-in";
	}
	return provider.authMode === AuthMode.ApiKey ? "Ready · API key" : "Ready · subscription";
}

function modeLabel(mode: AuthMode): string {
	return mode === AuthMode.Subscription ? "Subscription" : "API key";
}

/** The key field only matters in API-key mode. */
function needsKeyField(provider: ProviderStatus): boolean {
	return provider.authMode === AuthMode.ApiKey;
}

async function saveKey(provider: ProviderStatus) {
	const value = keyDrafts[provider.keyId] ?? "";
	await settingsStore.saveApiKey(provider.keyId, value);
	keyDrafts[provider.keyId] = "";
}

async function saveModel(provider: ProviderStatus) {
	await settingsStore.setModel(provider.id, modelDrafts[provider.id] ?? "");
}

/**
 * Capture the next chord and store it as a Tauri accelerator string.
 * The core validates it and rolls back if another app owns the shortcut.
 */
function onHotkeyKeydown(event: KeyboardEvent) {
	event.preventDefault();

	const key = event.key;
	if (["Control", "Shift", "Alt", "Meta"].includes(key)) return;

	const parts: string[] = [];
	if (event.ctrlKey) parts.push("Ctrl");
	if (event.shiftKey) parts.push("Shift");
	if (event.altKey) parts.push("Alt");
	if (event.metaKey) parts.push("Super");

	const named: Record<string, string> = { " ": "Space", Escape: "Escape", Enter: "Enter" };
	parts.push(named[key] ?? key.toUpperCase());

	capturingHotkey.value = false;
	settingsStore.save({ hotkey: parts.join("+") });
}

async function toggle(field: "hideOnBlur" | "launchAtLogin", value: boolean) {
	await settingsStore.save({ [field]: value });
}
</script>

<template>
	<div class="settings">
		<header class="settings__header">
			<h1 class="settings__title">Klets</h1>
			<p class="settings__subtitle">
				Quick answers from ACP agents. Edits are always refused; reads and running a
				command are governed by the tool policy below.
			</p>
		</header>

		<p v-if="settingsStore.error" class="settings__error">{{ settingsStore.error }}</p>

		<section class="settings__section">
			<h2 class="settings__heading">Behaviour</h2>

			<div class="settings__row">
				<div>
					<span class="settings__label">Launcher shortcut</span>
					<span class="settings__help">Press the keys you want to use.</span>
				</div>
				<button
					type="button"
					class="settings__hotkey"
					:class="{ 'settings__hotkey--capturing': capturingHotkey }"
					@click="capturingHotkey = true"
					@blur="capturingHotkey = false"
					@keydown="onHotkeyKeydown"
				>
					{{ capturingHotkey ? "Press keys…" : settingsStore.hotkey }}
				</button>
			</div>

			<div class="settings__row">
				<div>
					<span class="settings__label">Hide when it loses focus</span>
					<span class="settings__help">Behaves like Spotlight.</span>
				</div>
				<input
					type="checkbox"
					class="settings__checkbox"
					:checked="settingsStore.settings?.hideOnBlur"
					@change="toggle('hideOnBlur', ($event.target as HTMLInputElement).checked)"
				/>
			</div>

			<div class="settings__row">
				<div>
					<span class="settings__label">Launch at login</span>
					<span class="settings__help">Keep the shortcut available after a restart.</span>
				</div>
				<input
					type="checkbox"
					class="settings__checkbox"
					:checked="settingsStore.settings?.launchAtLogin"
					@change="toggle('launchAtLogin', ($event.target as HTMLInputElement).checked)"
				/>
			</div>
		</section>

		<section class="settings__section">
			<h2 class="settings__heading">Tools</h2>

			<div class="settings__row settings__row--stack">
				<div>
					<span class="settings__label">What agents are allowed to do</span>
					<span class="settings__help">{{ activePolicy.description }}</span>
				</div>
				<div class="segmented">
					<button
						v-for="policy in TOOL_POLICIES"
						:key="policy.value"
						type="button"
						class="segmented__option"
						:class="{ 'segmented__option--active': policy.value === settingsStore.settings?.toolPolicy }"
						:disabled="settingsStore.isSaving"
						@click="setToolPolicy(policy.value)"
					>
						{{ policy.label }}
					</button>
				</div>
			</div>

			<div class="settings__row settings__row--stack">
				<div>
					<span class="settings__label">Working directory</span>
					<span class="settings__help">
						Reads are allowed here. Defaults to a folder Klets owns, so nothing real is
						exposed until you choose one.
					</span>
				</div>
				<div class="provider__input-row">
					<input
						type="text"
						class="provider__input"
						readonly
						:value="settingsStore.settings?.workingDir || 'Managed scratch directory (default)'"
					/>
					<button type="button" class="provider__button" @click="browseWorkingDir">
						Browse…
					</button>
					<button
						v-if="settingsStore.settings?.workingDir"
						type="button"
						class="provider__button"
						@click="clearWorkingDir"
					>
						Reset
					</button>
				</div>
			</div>

			<div class="settings__row settings__row--stack settings__row--last">
				<div>
					<span class="settings__label">System prompt</span>
					<span class="settings__help">
						Written into the working directory as AGENTS.md, CLAUDE.md and GEMINI.md at the
						start of every session.
					</span>
				</div>
				<textarea
					v-model="promptDraft"
					class="settings__prompt"
					rows="7"
					spellcheck="false"
				></textarea>
				<div class="settings__prompt-actions">
					<button type="button" class="provider__link" @click="resetPrompt">
						Reset to default
					</button>
					<span class="settings__spacer"></span>
					<button
						type="button"
						class="provider__button"
						:disabled="!promptDirty || settingsStore.isSaving"
						@click="savePrompt"
					>
						Save
					</button>
				</div>
			</div>
		</section>

		<section class="settings__section">
			<div class="settings__section-header">
				<h2 class="settings__heading">Providers</h2>
				<button type="button" class="provider__link" @click="refresh">Re-scan</button>
			</div>

			<article v-for="provider in settingsStore.providers" :key="provider.id" class="provider">
				<header class="provider__header">
					<span class="provider__name">{{ provider.name }}</span>
					<span
						class="provider__badge"
						:class="{
							'provider__badge--ready': provider.binaryPath && provider.authenticated,
							'provider__badge--missing': !provider.binaryPath,
						}"
					>
						{{ statusLabel(provider) }}
					</span>
				</header>

				<p v-if="!provider.binaryPath" class="provider__install">
					Install the agent, then reopen this window:
					<code class="provider__code">{{ provider.installHint }}</code>
				</p>
				<template v-else>
					<p class="provider__path">
						{{ provider.binaryPath }}
						<span v-if="provider.runsVia" class="provider__via">
							· runs through {{ provider.runsVia }}
						</span>
					</p>

					<div v-if="provider.authModes.length > 1" class="provider__modes">
						<button
							v-for="mode in provider.authModes"
							:key="mode"
							type="button"
							class="provider__mode"
							:class="{ 'provider__mode--active': mode === provider.authMode }"
							:disabled="settingsStore.isSaving"
							@click="settingsStore.setAuthMode(provider.id, mode)"
						>
							{{ modeLabel(mode) }}
						</button>
					</div>

					<p
						v-if="!provider.authenticated && provider.authMode === AuthMode.Subscription"
						class="provider__install"
					>
						Sign in from a terminal:
						<code class="provider__code">{{ provider.loginHint }}</code>
					</p>
				</template>

				<div v-if="provider.binaryPath && needsKeyField(provider)" class="provider__field">
					<label class="provider__label" :for="`key-${provider.id}`">
						{{ provider.keyLabel }}
					</label>
					<div class="provider__input-row">
						<input
							:id="`key-${provider.id}`"
							v-model="keyDrafts[provider.keyId]"
							type="password"
							class="provider__input"
							:placeholder="provider.hasApiKey ? '••••••••  (saved)' : 'Paste your API key'"
							autocomplete="off"
						/>
						<button
							type="button"
							class="provider__button"
							:disabled="settingsStore.isSaving"
							@click="saveKey(provider)"
						>
							Save
						</button>
					</div>
					<button
						type="button"
						class="provider__link"
						@click="IpcService.openExternal(provider.keyUrl)"
					>
						Get a key →
					</button>
				</div>

				<div v-if="provider.binaryPath" class="provider__field">
					<label class="provider__label" :for="`model-${provider.id}`">Model</label>
					<div class="provider__input-row">
						<input
							:id="`model-${provider.id}`"
							v-model="modelDrafts[provider.id]"
							type="text"
							class="provider__input"
							:placeholder="provider.defaultModel ?? 'agent default'"
							autocomplete="off"
						/>
						<button
							type="button"
							class="provider__button"
							:disabled="settingsStore.isSaving"
							@click="saveModel(provider)"
						>
							Save
						</button>
					</div>
				</div>
			</article>
		</section>

		<footer class="settings__footer">
			<span v-if="settingsStore.notice" class="settings__notice">{{ settingsStore.notice }}</span>
			<span class="settings__spacer"></span>
			<button type="button" class="provider__button" @click="IpcService.closeSettings()">
				Close
			</button>
		</footer>
	</div>
</template>

<style scoped>
/*
 * The launcher needs `body { overflow: hidden }` so the floating panel is
 * never scrollable, so the settings window owns its own scroll container.
 */
.settings {
	height: 100vh;
	overflow-y: auto;
	padding: 1.5rem 1.75rem 1.25rem;
	background: var(--surface-app);
}

.settings__header {
	margin-bottom: 1.5rem;
}

.settings__title {
	margin: 0 0 0.3rem;
	font-size: 1.35rem;
	font-weight: 600;
	color: var(--text);
}

.settings__subtitle {
	margin: 0;
	max-width: 32rem;
	font-size: 0.85rem;
	line-height: 1.5;
	color: var(--text-muted);
}

.settings__error {
	margin: 0 0 1rem;
	padding: 0.6rem 0.8rem;
	font-size: 0.85rem;
	color: var(--danger);
	background: var(--danger-surface);
	border-radius: 0.5rem;
}

.settings__section {
	margin-bottom: 1.75rem;
}

.settings__section-header {
	display: flex;
	align-items: baseline;
	justify-content: space-between;
	gap: 1rem;
}

.settings__heading {
	margin: 0 0 0.75rem;
	font-size: 0.75rem;
	font-weight: 600;
	letter-spacing: 0.08em;
	text-transform: uppercase;
	color: var(--text-faint);
}

.settings__row {
	display: flex;
	align-items: center;
	justify-content: space-between;
	gap: 1rem;
	padding: 0.7rem 0;
	border-bottom: 0.0625rem solid var(--border);
}

.settings__label {
	display: block;
	font-size: 0.9rem;
	color: var(--text);
}

.settings__help {
	display: block;
	margin-top: 0.15rem;
	font-size: 0.78rem;
	color: var(--text-faint);
}

.settings__hotkey {
	min-width: 8rem;
	padding: 0.4rem 0.7rem;
	font-family: var(--font-mono);
	font-size: 0.82rem;
	color: var(--text);
	background: var(--surface-raised);
	border: 0.0625rem solid var(--border-strong);
	border-radius: 0.45rem;
	cursor: pointer;
}

.settings__hotkey--capturing {
	color: var(--accent-text);
	border-color: var(--accent);
}

.settings__checkbox {
	width: 1.1rem;
	height: 1.1rem;
	accent-color: var(--accent);
	cursor: pointer;
}

.settings__row--stack {
	flex-direction: column;
	align-items: stretch;
	gap: 0.5rem;
}

.settings__row--last {
	border-bottom: none;
}

.segmented {
	display: inline-flex;
	align-self: flex-start;
	padding: 0.15rem;
	background: var(--surface-raised);
	border: 0.0625rem solid var(--border);
	border-radius: 0.45rem;
}

.segmented__option {
	padding: 0.3rem 0.75rem;
	font-family: inherit;
	font-size: 0.8rem;
	color: var(--text-muted);
	background: none;
	border: none;
	border-radius: 0.35rem;
	cursor: pointer;
}

.segmented__option--active {
	color: var(--text);
	background: var(--surface-hover);
}

.segmented__option:disabled {
	cursor: default;
}

.settings__prompt {
	width: 100%;
	padding: 0.6rem 0.7rem;
	font-family: var(--font-mono);
	font-size: 0.82rem;
	line-height: 1.5;
	color: var(--text);
	background: var(--surface-raised);
	border: 0.0625rem solid var(--border-strong);
	border-radius: 0.5rem;
	resize: vertical;
	outline: none;
}

.settings__prompt:focus {
	border-color: var(--accent);
}

.settings__prompt-actions {
	display: flex;
	align-items: center;
	gap: 0.5rem;
}

.provider {
	margin-bottom: 1rem;
	padding: 0.9rem 1rem;
	background: var(--surface-raised);
	border: 0.0625rem solid var(--border);
	border-radius: 0.7rem;
}

.provider__header {
	display: flex;
	align-items: center;
	justify-content: space-between;
	margin-bottom: 0.5rem;
}

.provider__name {
	font-size: 0.95rem;
	font-weight: 600;
	color: var(--text);
}

.provider__badge {
	padding: 0.15rem 0.5rem;
	font-size: 0.7rem;
	color: var(--text-muted);
	background: var(--surface);
	border: 0.0625rem solid var(--border);
	border-radius: 1rem;
}

.provider__badge--ready {
	color: var(--success);
	border-color: var(--success);
}

.provider__badge--missing {
	color: var(--text-faint);
}

.provider__install {
	margin: 0 0 0.7rem;
	font-size: 0.8rem;
	color: var(--text-muted);
}

.provider__code {
	display: block;
	margin-top: 0.35rem;
	padding: 0.4rem 0.55rem;
	font-family: var(--font-mono);
	font-size: 0.76rem;
	color: var(--text);
	background: var(--surface-code);
	border-radius: 0.4rem;
	overflow-wrap: anywhere;
}

.provider__path {
	margin: 0 0 0.7rem;
	font-family: var(--font-mono);
	font-size: 0.72rem;
	color: var(--text-faint);
	overflow-wrap: anywhere;
}

.provider__field {
	margin-top: 0.7rem;
}

.provider__label {
	display: block;
	margin-bottom: 0.3rem;
	font-size: 0.78rem;
	color: var(--text-muted);
}

.provider__input-row {
	display: flex;
	gap: 0.4rem;
}

.provider__input {
	flex: 1;
	min-width: 0;
	padding: 0.45rem 0.6rem;
	font-family: inherit;
	font-size: 0.85rem;
	color: var(--text);
	background: var(--surface);
	border: 0.0625rem solid var(--border-strong);
	border-radius: 0.45rem;
	outline: none;
}

.provider__input:focus {
	border-color: var(--accent);
}

.provider__button {
	padding: 0.45rem 0.8rem;
	font-family: inherit;
	font-size: 0.82rem;
	color: var(--text);
	background: var(--surface-hover);
	border: 0.0625rem solid var(--border-strong);
	border-radius: 0.45rem;
	cursor: pointer;
}

.provider__button:hover:not(:disabled) {
	border-color: var(--accent);
}

.provider__button:disabled {
	opacity: 0.5;
	cursor: default;
}

.provider__via {
	color: var(--text-faint);
}

.provider__modes {
	display: inline-flex;
	margin-bottom: 0.5rem;
	padding: 0.15rem;
	background: var(--surface);
	border: 0.0625rem solid var(--border);
	border-radius: 0.45rem;
}

.provider__mode {
	padding: 0.25rem 0.7rem;
	font-family: inherit;
	font-size: 0.78rem;
	color: var(--text-muted);
	background: none;
	border: none;
	border-radius: 0.35rem;
	cursor: pointer;
}

.provider__mode--active {
	color: var(--text);
	background: var(--surface-hover);
}

.provider__mode:disabled {
	cursor: default;
}

.provider__link {
	margin-top: 0.4rem;
	padding: 0;
	font-family: inherit;
	font-size: 0.76rem;
	color: var(--accent-text);
	background: none;
	border: none;
	cursor: pointer;
}

.settings__footer {
	position: sticky;
	bottom: 0;
	display: flex;
	align-items: center;
	gap: 0.75rem;
	padding: 0.75rem 0 0.25rem;
	/* Keep Close reachable without scrolling to the end of the provider list. */
	background: var(--surface-app);
}

.settings__notice {
	font-size: 0.8rem;
	color: var(--success);
}

.settings__spacer {
	flex: 1;
}
</style>
