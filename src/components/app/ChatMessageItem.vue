<script lang="ts" setup>
import { computed, ref } from "vue";
import MarkdownView from "@/components/app/MarkdownView.vue";
import { IpcService } from "@/services/ipc";
import { MessageRole, type ChatMessage, type PendingPermission } from "@/helpers/types";

const props = defineProps<{ message: ChatMessage; }>();

async function respond(request: PendingPermission, allow: boolean) {
	// Mark it locally so the buttons cannot be pressed twice; the core also
	// emits a resolved event, which sets the same field.
	request.outcome = allow ? "allowed" : "denied";
	await IpcService.respondPermission(request.id, allow);
}

function outcomeLabel(request: PendingPermission): string {
	if (request.outcome === "allowed") return "Allowed";
	if (request.outcome === "timedOut") return "No answer — refused";
	return "Denied";
}

const showThoughts = ref(false);

const isUser = computed(() => props.message.role === MessageRole.User);
const hasThoughts = computed(() => props.message.thoughts.trim().length > 0);
const isThinking = computed(
	() => props.message.streaming && props.message.text.trim().length === 0,
);

/**
 * The caret is appended to the markdown source rather than rendered beside it,
 * so it sits inline at the end of the last line instead of on its own row.
 */
const displaySource = computed(() =>
	props.message.streaming ? `${props.message.text}\u2009▍` : props.message.text,
);
</script>

<template>
	<div class="message" :class="{ 'message--user': isUser }">
		<div v-if="isUser" class="message__user">{{ message.text }}</div>

		<template v-else>
			<div v-if="message.tools.length" class="message__tools">
				<span
					v-for="tool in message.tools"
					:key="tool.id"
					class="message__tool"
					:class="`message__tool--${tool.status || 'pending'}`"
				>
					{{ tool.title }}
				</span>
			</div>

			<button
				v-if="hasThoughts"
				type="button"
				class="message__thoughts-toggle"
				@click="showThoughts = !showThoughts"
			>
				{{ showThoughts ? "Hide reasoning" : "Show reasoning" }}
			</button>
			<div v-if="hasThoughts && showThoughts" class="message__thoughts">
				{{ message.thoughts }}
			</div>

			<div
				v-for="request in message.permissions"
				:key="request.id"
				class="permission"
				:class="{ 'permission--answered': !!request.outcome }"
			>
				<div class="permission__body">
					<span class="permission__title">{{ request.title }}</span>
					<code v-if="request.detail" class="permission__detail">{{ request.detail }}</code>
				</div>

				<div v-if="!request.outcome" class="permission__actions">
					<button type="button" class="permission__deny" @click="respond(request, false)">
						Deny
					</button>
					<button type="button" class="permission__allow" @click="respond(request, true)">
						Allow
					</button>
				</div>
				<span v-else class="permission__outcome">{{ outcomeLabel(request) }}</span>
			</div>

			<div v-if="isThinking" class="message__thinking" aria-label="Thinking">
				<span></span><span></span><span></span>
			</div>

			<MarkdownView v-if="message.text" :source="displaySource" />

			<p v-for="notice in message.notices" :key="notice" class="message__notice">
				{{ notice }}
			</p>

			<p v-if="message.error" class="message__error">{{ message.error }}</p>
		</template>
	</div>
</template>

<style scoped>
.message {
	padding: 0 0 1.1rem;
}

.message--user {
	display: flex;
	justify-content: flex-end;
	padding-bottom: 0.9rem;
}

.message__user {
	max-width: 85%;
	padding: 0.5rem 0.85rem;
	background: var(--surface-raised);
	border: 0.0625rem solid var(--border);
	border-radius: 0.9rem 0.9rem 0.25rem 0.9rem;
	font-size: 0.92rem;
	line-height: 1.5;
	color: var(--text);
	white-space: pre-wrap;
	overflow-wrap: anywhere;
}

.message__tools {
	display: flex;
	flex-wrap: wrap;
	gap: 0.35rem;
	margin-bottom: 0.55rem;
}

.message__tool {
	padding: 0.15rem 0.5rem;
	font-size: 0.72rem;
	color: var(--text-muted);
	background: var(--surface-raised);
	border: 0.0625rem solid var(--border);
	border-radius: 1rem;
}

.message__tool--completed {
	color: var(--accent-text);
}

.message__tool--failed {
	color: var(--danger);
}

.message__thoughts-toggle {
	margin-bottom: 0.4rem;
	padding: 0;
	font-size: 0.75rem;
	color: var(--text-faint);
	background: none;
	border: none;
	cursor: pointer;
}

.message__thoughts-toggle:hover {
	color: var(--text-muted);
}

.message__thoughts {
	margin-bottom: 0.6rem;
	padding: 0.55rem 0.7rem;
	font-size: 0.82rem;
	line-height: 1.55;
	color: var(--text-faint);
	background: var(--surface-raised);
	border-radius: 0.5rem;
	white-space: pre-wrap;
}

.permission {
	display: flex;
	align-items: center;
	justify-content: space-between;
	gap: 0.75rem;
	margin: 0 0 0.6rem;
	padding: 0.5rem 0.6rem 0.5rem 0.7rem;
	background: var(--surface-raised);
	border: 0.0625rem solid var(--accent);
	border-radius: 0.5rem;
}

.permission--answered {
	border-color: var(--border);
}

.permission__body {
	min-width: 0;
	display: flex;
	flex-direction: column;
	gap: 0.25rem;
}

.permission__title {
	font-size: 0.82rem;
	color: var(--text);
}

.permission__detail {
	font-family: var(--font-mono);
	font-size: 0.75rem;
	color: var(--text-muted);
	overflow-wrap: anywhere;
}

.permission__actions {
	flex: none;
	display: flex;
	gap: 0.35rem;
}

.permission__allow,
.permission__deny {
	padding: 0.25rem 0.65rem;
	font-family: inherit;
	font-size: 0.78rem;
	border-radius: 0.35rem;
	border: 0.0625rem solid var(--border-strong);
	background: none;
	color: var(--text-muted);
	cursor: pointer;
}

.permission__allow {
	color: var(--text);
	border-color: var(--accent);
}

.permission__allow:hover {
	background: var(--surface-hover);
}

.permission__deny:hover {
	color: var(--danger);
	border-color: var(--danger);
}

.permission__outcome {
	flex: none;
	font-size: 0.75rem;
	color: var(--text-faint);
}

.message__thinking {
	display: flex;
	gap: 0.28rem;
	padding: 0.35rem 0;
}

.message__thinking span {
	width: 0.35rem;
	height: 0.35rem;
	border-radius: 50%;
	background: var(--text-faint);
	animation: pulse 1.2s ease-in-out infinite;
}

.message__thinking span:nth-child(2) { animation-delay: 0.15s; }
.message__thinking span:nth-child(3) { animation-delay: 0.3s; }

.message__notice {
	margin: 0.5rem 0 0;
	font-size: 0.78rem;
	color: var(--text-faint);
}

.message__error {
	margin: 0.5rem 0 0;
	padding: 0.5rem 0.7rem;
	font-size: 0.82rem;
	color: var(--danger);
	background: var(--danger-surface);
	border-radius: 0.5rem;
	white-space: pre-wrap;
}

@keyframes pulse {
	0%, 60%, 100% { opacity: 0.25; transform: translateY(0); }
	30% { opacity: 1; transform: translateY(-0.15rem); }
}
</style>
