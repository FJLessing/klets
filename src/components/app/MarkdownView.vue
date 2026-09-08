<script lang="ts" setup>
import { computed, ref } from "vue";
import { renderMarkdown } from "@/helpers/markdown";
import { IpcService } from "@/services/ipc";

const props = defineProps<{ source: string; }>();

const container = ref<HTMLElement | null>(null);
const html = computed(() => renderMarkdown(props.source));

/**
 * Links and copy buttons are inside rendered HTML, so they are handled with a
 * single delegated listener rather than per-node Vue bindings.
 *
 * Every `<a>` is matched, not just `[data-external]`: a webview must never
 * navigate away from the app shell (see the comment in markdown.ts), and a
 * relative or `mailto:` link has nowhere else useful to go, so it is simply
 * swallowed rather than left to the default in-app navigation.
 */
async function onClick(event: MouseEvent) {
	const target = (event.target as HTMLElement | null)?.closest("a, [data-copy]");
	if (!target) return;

	event.preventDefault();

	const href = target.getAttribute("data-external");
	if (href) {
		try {
			await IpcService.openExternal(href);
		} catch (err) {
			// Not user-actionable (no default browser, OS-level failure, or a
			// permission scope regression) — surfaced for debugging rather
			// than silently swallowed, since preventDefault already ran.
			console.error(`Klets: could not open ${href} in the system browser`, err);
		}
		return;
	}

	const code = target.getAttribute("data-copy");
	if (code !== null) {
		await navigator.clipboard.writeText(code);
		target.textContent = "Copied";
		window.setTimeout(() => {
			target.textContent = "Copy";
		}, 1400);
	}
}
</script>

<template>
	<div ref="container" class="markdown" @click="onClick" v-html="html"></div>
</template>

<style scoped>
.markdown {
	font-size: 0.95rem;
	line-height: 1.6;
	color: var(--text);
	overflow-wrap: anywhere;
}

.markdown :deep(> *:first-child) {
	margin-top: 0;
}

.markdown :deep(> *:last-child) {
	margin-bottom: 0;
}

.markdown :deep(p) {
	margin: 0 0 0.7em;
}

.markdown :deep(h1),
.markdown :deep(h2),
.markdown :deep(h3),
.markdown :deep(h4) {
	margin: 1.2em 0 0.5em;
	line-height: 1.3;
	font-weight: 600;
}

.markdown :deep(h1) { font-size: 1.3rem; }
.markdown :deep(h2) { font-size: 1.15rem; }
.markdown :deep(h3) { font-size: 1.02rem; }

.markdown :deep(ul),
.markdown :deep(ol) {
	margin: 0 0 0.7em;
	padding-left: 1.35em;
}

.markdown :deep(li) {
	margin-bottom: 0.25em;
}

.markdown :deep(a) {
	color: var(--accent-text);
	text-decoration: underline;
	text-underline-offset: 0.15em;
	cursor: pointer;
}

.markdown :deep(a:hover) {
	color: var(--text);
}

.markdown :deep(blockquote) {
	margin: 0 0 0.7em;
	padding: 0.1em 0 0.1em 0.9em;
	border-left: 0.15rem solid var(--border-strong);
	color: var(--text-muted);
}

.markdown :deep(code) {
	font-family: var(--font-mono);
	font-size: 0.86em;
	background: var(--surface-code);
	border: 0.0625rem solid var(--border);
	border-radius: 0.3rem;
	padding: 0.1em 0.35em;
}

.markdown :deep(.code-block) {
	position: relative;
	margin: 0 0 0.8em;
}

.markdown :deep(.code-block__copy) {
	position: absolute;
	top: 0.45rem;
	right: 0.45rem;
	z-index: 1;
	padding: 0.2rem 0.5rem;
	font-size: 0.7rem;
	font-family: var(--font-sans);
	color: var(--text-muted);
	background: var(--surface-raised);
	border: 0.0625rem solid var(--border);
	border-radius: 0.35rem;
	opacity: 0;
	cursor: pointer;
	transition: opacity 0.15s ease, color 0.15s ease;
}

.markdown :deep(.code-block:hover .code-block__copy) {
	opacity: 1;
}

.markdown :deep(.code-block__copy:hover) {
	color: var(--text);
}

.markdown :deep(pre) {
	margin: 0;
	padding: 0.75rem 0.9rem;
	overflow-x: auto;
	background: var(--surface-code);
	border: 0.0625rem solid var(--border);
	border-radius: 0.6rem;
}

.markdown :deep(pre code) {
	padding: 0;
	border: none;
	background: none;
	font-size: 0.82rem;
	line-height: 1.55;
}

.markdown :deep(table) {
	width: 100%;
	margin: 0 0 0.8em;
	border-collapse: collapse;
	font-size: 0.88rem;
}

.markdown :deep(th),
.markdown :deep(td) {
	padding: 0.35rem 0.55rem;
	border: 0.0625rem solid var(--border);
	text-align: left;
}

.markdown :deep(th) {
	background: var(--surface-raised);
	font-weight: 600;
}

.markdown :deep(hr) {
	margin: 1em 0;
	border: none;
	border-top: 0.0625rem solid var(--border);
}
</style>
