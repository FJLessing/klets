<script lang="ts" setup>
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import type { UnlistenFn } from "@tauri-apps/api/event";
import ChatMessageItem from "@/components/app/ChatMessageItem.vue";
import { IpcService } from "@/services/ipc";
import { useChatStore } from "@/stores/chatstore";
import { useSettingsStore } from "@/stores/settingsstore";

const chatStore = useChatStore();
const settingsStore = useSettingsStore();

const input = ref<HTMLTextAreaElement | null>(null);
const frame = ref<HTMLElement | null>(null);
const shell = ref<HTMLElement | null>(null);
const transcript = ref<HTMLElement | null>(null);
const draft = ref("");
const showProviders = ref(false);
const showCommands = ref(false);

const unlisteners: UnlistenFn[] = [];

const placeholder = computed(() => {
	if (!settingsStore.hasAnyProvider) return "Add an API key in settings to get started";
	return `Ask ${settingsStore.activeProvider?.name ?? "anything"}…`;
});

const canSend = computed(
	() => draft.value.trim().length > 0 && !chatStore.isStreaming && settingsStore.hasAnyProvider,
);

/**
 * Grow the window with the content instead of scrolling a fixed-size box.
 *
 * `scrollHeight` is the card's full content height even while `max-height`
 * constrains it, so the window keeps requesting the size the content wants and
 * the Rust side decides where to stop. The frame's padding is added on top so
 * the shadow is never clipped.
 */
let heightFrame = 0;
let lastRequestedHeight = 0;

/**
 * Coalesce height updates to one per frame.
 *
 * Streaming fires a change per token, and each resize is a window-manager
 * operation, so measuring on every one is both wasteful and visibly jittery.
 */
function scheduleWindowHeight() {
	if (heightFrame) return;
	heightFrame = requestAnimationFrame(() => {
		heightFrame = 0;
		void syncWindowHeight();
	});
}

async function syncWindowHeight() {
	await nextTick();
	const card = shell.value;
	const gutter = frame.value;
	if (!card || !gutter) return;

	const style = getComputedStyle(gutter);
	const padding = parseFloat(style.paddingTop) + parseFloat(style.paddingBottom);

	// The transcript is its own scroll container, so once it starts scrolling
	// its overflow stops showing up in the card's scrollHeight. Measuring the
	// card alone therefore reports the size it already is, and the window can
	// never grow past whatever made it scroll — so measure the chrome and the
	// transcript's true content height separately.
	const list = transcript.value;
	const wanted = list
		? card.offsetHeight - list.clientHeight + list.scrollHeight
		: card.scrollHeight;

	if (wanted <= 0) return;

	const target = Math.round(wanted + padding);
	if (target === lastRequestedHeight) return;

	lastRequestedHeight = target;
	await IpcService.resizeLauncher(target);
}

async function scrollToBottom() {
	await nextTick();
	const element = transcript.value;
	if (element) element.scrollTop = element.scrollHeight;
}

function focusInput() {
	nextTick(() => {
		input.value?.focus();
		input.value?.select();
	});
}

function autoGrowInput() {
	const element = input.value;
	if (!element) return;
	element.style.height = "auto";
	// Cap at roughly five lines so the composer never swallows the transcript.
	element.style.height = `${Math.min(element.scrollHeight, 132)}px`;
}

async function send() {
	if (!canSend.value) return;
	const text = draft.value;
	draft.value = "";
	autoGrowInput();
	await chatStore.send(text);
}

/**
 * All launcher shortcuts live on the window rather than the textarea, so they
 * still fire after clicking a link or a code block's copy button.
 */
async function onKeydown(event: KeyboardEvent) {
	const inInput = document.activeElement === input.value;

	// Enter sends, Shift+Enter is a newline.
	if (event.key === "Enter" && !event.shiftKey && inInput) {
		event.preventDefault();
		await send();
		return;
	}

	if (event.key === "Escape") {
		event.preventDefault();
		if (showCommands.value) {
			showCommands.value = false;
			return;
		}
		if (showProviders.value) {
			showProviders.value = false;
			return;
		}
		if (chatStore.isStreaming) {
			await chatStore.cancel();
			return;
		}
		if (draft.value) {
			draft.value = "";
			autoGrowInput();
			return;
		}
		await IpcService.hideLauncher();
		return;
	}

	if (event.key === "n" && (event.ctrlKey || event.metaKey)) {
		event.preventDefault();
		await newChat();
		return;
	}

	if (event.key === "," && (event.ctrlKey || event.metaKey)) {
		event.preventDefault();
		await IpcService.openSettings();
		return;
	}

	// Typing anywhere should land in the composer.
	if (!inInput && event.key.length === 1 && !event.ctrlKey && !event.metaKey && !event.altKey) {
		input.value?.focus();
	}
}

async function newChat() {
	await chatStore.reset();
	draft.value = "";
	autoGrowInput();
	focusInput();
	await syncWindowHeight();
}

async function pickProvider(id: string) {
	showProviders.value = false;
	if (id === settingsStore.settings?.activeProvider) return;
	await settingsStore.selectProvider(id);
	// A different agent means a different conversation.
	await chatStore.reset();
	focusInput();
}

function toggleProviders() {
	showCommands.value = false;
	showProviders.value = !showProviders.value;
}

function toggleCommands() {
	showProviders.value = false;
	showCommands.value = !showCommands.value;
}

onMounted(async () => {
	window.addEventListener("keydown", onKeydown);
	await settingsStore.load();
	focusInput();
	await syncWindowHeight();

	unlisteners.push(
		await IpcService.onAgentEvent((event) => chatStore.handleEvent(event)),
		await IpcService.onFocus(async () => {
			// Re-detect agents: one may have been installed or signed in since.
			await settingsStore.load();
			focusInput();
		}),
	);
});

onBeforeUnmount(() => {
	window.removeEventListener("keydown", onKeydown);
	if (heightFrame) cancelAnimationFrame(heightFrame);
	unlisteners.forEach((unlisten) => unlisten());
});

watch(
	() => [chatStore.messages.length, chatStore.currentMessage?.text],
	async () => {
		scheduleWindowHeight();
		await scrollToBottom();
	},
);

watch(draft, autoGrowInput);

// Both panels are part of the layout, so the window has to grow for them.
watch([showProviders, showCommands], syncWindowHeight);
</script>

<template>
	<!--
		The frame is transparent padding that gives the card's shadow somewhere
		to fall. Without it the window is sized flush to the card and the OS
		clips the shadow into a hard-edged band.
	-->
	<div ref="frame" class="launcher-frame">
		<div ref="shell" class="launcher">
			<div class="launcher__bar">
				<svg class="launcher__glyph" viewBox="0 0 24 24" aria-hidden="true">
					<path
						d="M12 3c4.97 0 9 3.36 9 7.5s-4.03 7.5-9 7.5c-.9 0-1.77-.11-2.59-.32L5 20.5l.94-3.2C4.13 15.93 3 13.83 3 11.5 3 7.36 7.03 3 12 3z"
						fill="none"
						stroke="currentColor"
						stroke-width="1.6"
						stroke-linejoin="round"
					/>
				</svg>

				<textarea
					ref="input"
					v-model="draft"
					class="launcher__input"
					:placeholder="placeholder"
					rows="1"
					spellcheck="false"
					autocomplete="off"
				></textarea>

				<button
					v-if="chatStore.isStreaming"
					type="button"
					class="launcher__action launcher__action--stop"
					title="Stop (Esc)"
					@click="chatStore.cancel()"
				>
					Stop
				</button>

				<button
					v-if="chatStore.commands.length"
					type="button"
					class="launcher__action"
					:title="`${chatStore.commands.length} tools this agent can use`"
					@click="toggleCommands"
				>
					{{ chatStore.commands.length }} tools
				</button>

				<button
					type="button"
					class="launcher__action"
					:title="`Provider: ${settingsStore.activeProvider?.name ?? 'none'}`"
					@click="toggleProviders"
				>
					{{ settingsStore.activeProvider?.name ?? "Set up" }}
				</button>
			</div>

			<!--
				Rendered in flow rather than as an overlay: the launcher window is
				sized to its content, so an absolutely positioned menu would be
				clipped by the window frame.
			-->
			<ul v-if="showProviders" class="launcher__menu">
				<li v-for="provider in settingsStore.readyProviders" :key="provider.id">
					<button
						type="button"
						class="launcher__menu-item"
						:class="{
							'launcher__menu-item--active':
								provider.id === settingsStore.settings?.activeProvider,
						}"
						@click="pickProvider(provider.id)"
					>
						<span>{{ provider.name }}</span>
						<span v-if="provider.model" class="launcher__menu-hint">
							{{ provider.model.split("/").pop() }}
						</span>
					</button>
				</li>
				<li v-if="!settingsStore.readyProviders.length" class="launcher__menu-empty">
					No agents are installed yet.
				</li>
				<li class="launcher__menu-divider"></li>
				<li>
					<button type="button" class="launcher__menu-item" @click="IpcService.openSettings()">
						Settings…
					</button>
				</li>
			</ul>

			<!--
				What the agent actually loaded — its own MCP servers and skills,
				not anything Klets configures. Capped and independently
				scrollable so a long list (Claude commonly reports 100+) never
				forces the window past its own maximum height.
			-->
			<div v-if="showCommands" class="launcher__commands">
				<p class="launcher__commands-hint">
					Discovered from {{ settingsStore.activeProvider?.name }}'s own config:
				</p>
				<ul class="launcher__commands-list">
					<li v-for="command in chatStore.commands" :key="command.name">
						<span class="launcher__command-name">/{{ command.name }}</span>
						<span v-if="command.description" class="launcher__command-desc">
							{{ command.description }}
						</span>
					</li>
				</ul>
			</div>

			<div v-if="chatStore.statusLabel" class="launcher__status">
				{{ chatStore.statusLabel }}
			</div>

			<div v-if="!chatStore.isEmpty" ref="transcript" class="launcher__transcript">
				<ChatMessageItem
					v-for="message in chatStore.messages"
					:key="message.id"
					:message="message"
				/>
			</div>

			<div v-if="!chatStore.isEmpty" class="launcher__footer">
				<button type="button" class="launcher__hint-button" @click="newChat">New chat</button>
				<span class="launcher__hint">Ctrl+N</span>
				<span class="launcher__spacer"></span>
				<span class="launcher__hint">Esc to close</span>
			</div>

			<div v-else-if="!settingsStore.hasAnyProvider && !settingsStore.isLoading" class="launcher__setup">
				No agent is ready yet.
				<button type="button" class="launcher__hint-button" @click="IpcService.openSettings()">
					Open settings
				</button>
			</div>
		</div>
	</div>
</template>

<style scoped>
/*
 * Transparent gutter around the card. Its size must stay in step with the
 * card's box-shadow, and `syncWindowHeight` adds it to the requested window
 * height so the shadow always has room to fade out.
 */
.launcher-frame {
	display: flex;
	flex-direction: column;
	height: 100%;
	padding: 1.25rem 1.75rem 2.25rem;
}

.launcher {
	display: flex;
	flex-direction: column;
	/* Cap at the frame so long answers scroll inside the card rather than
	   overflowing past the bottom of the window. */
	max-height: 100%;
	overflow: hidden;
	background: var(--surface);
	border: 0.0625rem solid var(--border-strong);
	border-radius: 0.9rem;
	box-shadow: 0 0.75rem 2rem rgba(0, 0, 0, 0.5);
}

.launcher__bar {
	display: flex;
	/* Chrome keeps its size; only the transcript gives way. */
	flex: none;
	align-items: flex-start;
	gap: 0.6rem;
	padding: 0.85rem 0.9rem;
}

.launcher__glyph {
	flex: none;
	width: 1.25rem;
	height: 1.25rem;
	margin-top: 0.2rem;
	color: var(--accent-text);
}

.launcher__input {
	flex: 1;
	min-width: 0;
	max-height: 8.25rem;
	padding: 0;
	font-family: inherit;
	font-size: 1.05rem;
	line-height: 1.5;
	color: var(--text);
	background: none;
	border: none;
	outline: none;
	resize: none;
}

.launcher__input::placeholder {
	color: var(--text-faint);
}

.launcher__action {
	flex: none;
	margin-top: 0.1rem;
	padding: 0.25rem 0.6rem;
	font-family: inherit;
	font-size: 0.75rem;
	color: var(--text-muted);
	background: var(--surface-raised);
	border: 0.0625rem solid var(--border);
	border-radius: 0.45rem;
	cursor: pointer;
	transition: color 0.15s ease, border-color 0.15s ease;
}

.launcher__action:hover {
	color: var(--text);
	border-color: var(--border-strong);
}

.launcher__action--stop {
	color: var(--danger);
}

.launcher__menu {
	flex: none;
	margin: 0;
	padding: 0.3rem 0.55rem 0.5rem;
	list-style: none;
	border-top: 0.0625rem solid var(--border);
}

.launcher__menu-empty {
	padding: 0.4rem 0.55rem;
	font-size: 0.78rem;
	color: var(--text-faint);
}

.launcher__menu-item {
	display: flex;
	align-items: center;
	justify-content: space-between;
	gap: 0.5rem;
	width: 100%;
	padding: 0.4rem 0.55rem;
	font-family: inherit;
	font-size: 0.82rem;
	text-align: left;
	color: var(--text);
	background: none;
	border: none;
	border-radius: 0.4rem;
	cursor: pointer;
}

.launcher__menu-item:hover {
	background: var(--surface-hover);
}

.launcher__menu-item--active {
	color: var(--accent-text);
}

.launcher__menu-hint {
	font-size: 0.7rem;
	color: var(--text-faint);
}

.launcher__menu-divider {
	height: 0.0625rem;
	margin: 0.25rem 0;
	background: var(--border);
}

.launcher__commands {
	flex: none;
	padding: 0.6rem 0.9rem 0.2rem;
	border-top: 0.0625rem solid var(--border);
}

.launcher__commands-hint {
	margin: 0 0 0.4rem;
	font-size: 0.74rem;
	color: var(--text-faint);
}

.launcher__commands-list {
	/*
	 * Capped and independently scrollable, unlike the transcript: a long
	 * discovery list (Claude commonly reports 100+ commands) should not force
	 * the window toward its own maximum height on its own.
	 */
	max-height: 13rem;
	margin: 0 0 0.5rem;
	padding: 0;
	overflow-y: auto;
	list-style: none;
}

.launcher__commands-list li {
	display: flex;
	flex-direction: column;
	gap: 0.1rem;
	padding: 0.3rem 0.1rem;
}

.launcher__commands-list li + li {
	border-top: 0.0625rem solid var(--border);
}

.launcher__command-name {
	font-family: var(--font-mono);
	font-size: 0.78rem;
	color: var(--accent-text);
}

.launcher__command-desc {
	font-size: 0.74rem;
	line-height: 1.4;
	color: var(--text-faint);
}

.launcher__status {
	flex: none;
	padding: 0 0.9rem 0.7rem 2.75rem;
	font-size: 0.78rem;
	color: var(--text-faint);
}

.launcher__transcript {
	/*
	 * Basis must stay `auto`, not 0: the card is content-sized until it hits
	 * its max-height, and a zero-basis item contributes no height to an
	 * auto-height flex container, so the transcript would collapse to nothing.
	 * `min-height: 0` then lets it shrink and scroll once the cap is reached.
	 */
	flex: 1 1 auto;
	min-height: 0;
	overflow-y: auto;
	padding: 0.4rem 0.9rem 0;
	border-top: 0.0625rem solid var(--border);
}

.launcher__footer,
.launcher__setup {
	display: flex;
	flex: none;
	align-items: center;
	gap: 0.5rem;
	padding: 0.5rem 0.9rem;
	font-size: 0.72rem;
	color: var(--text-faint);
	border-top: 0.0625rem solid var(--border);
}

.launcher__spacer {
	flex: 1;
}

.launcher__hint-button {
	padding: 0;
	font-family: inherit;
	font-size: 0.72rem;
	color: var(--accent-text);
	background: none;
	border: none;
	cursor: pointer;
}

.launcher__hint-button:hover {
	color: var(--text);
}
</style>
