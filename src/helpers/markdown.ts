import MarkdownIt from "markdown-it";
import hljs from "highlight.js/lib/common";

/**
 * Markdown renderer tuned for quick answers.
 *
 * Links are rendered with `data-external` so the launcher can intercept them
 * and hand them to the OS browser — a webview must never navigate away from
 * the app shell.
 */
const md: MarkdownIt = new MarkdownIt({
	html: false,
	linkify: true,
	breaks: true,
	highlight(code: string, language: string): string {
		if (language && hljs.getLanguage(language)) {
			try {
				return hljs.highlight(code, { language, ignoreIllegals: true }).value;
			} catch {
				// Fall through to auto-detection below.
			}
		}
		try {
			return hljs.highlightAuto(code).value;
		} catch {
			return md.utils.escapeHtml(code);
		}
	},
});

const defaultLinkOpen =
	md.renderer.rules.link_open ??
	((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));

md.renderer.rules.link_open = (tokens, idx, options, env, self) => {
	const token = tokens[idx];
	const href = token.attrGet("href") ?? "";
	if (/^https?:\/\//i.test(href)) {
		token.attrSet("data-external", href);
		token.attrSet("rel", "noopener noreferrer");
	}
	return defaultLinkOpen(tokens, idx, options, env, self);
};

const defaultFence =
	md.renderer.rules.fence ??
	((tokens, idx, options, _env, self) => self.renderToken(tokens, idx, options));

md.renderer.rules.fence = (tokens, idx, options, env, self) => {
	const rendered = defaultFence(tokens, idx, options, env, self);
	const language = tokens[idx].info.trim().split(/\s+/)[0] || "text";
	const encoded = md.utils.escapeHtml(tokens[idx].content);
	return `<div class="code-block" data-language="${md.utils.escapeHtml(language)}">
		<button class="code-block__copy" type="button" data-copy="${encoded}">Copy</button>
		${rendered}
	</div>`;
};

export function renderMarkdown(source: string): string {
	return md.render(source);
}

export function renderInline(source: string): string {
	return md.renderInline(source);
}
