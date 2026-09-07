import { describe, expect, it } from "vitest";
import { renderMarkdown } from "@/helpers/markdown";

describe("renderMarkdown", () => {
	it("marks http(s) links as external so the launcher can hand them to the OS browser", () => {
		const html = renderMarkdown("[docs](https://example.com/docs)");
		expect(html).toContain('data-external="https://example.com/docs"');
		expect(html).toContain('rel="noopener noreferrer"');
	});

	it("does not mark a relative link as external", () => {
		// A webview must never navigate away from the app shell, but a
		// relative link has nowhere else to send the click, so `data-external`
		// (which triggers exactly that) must not be present.
		const html = renderMarkdown("[here](/local/path)");
		expect(html).not.toContain("data-external");
	});

	it("does not mark a non-http scheme as external", () => {
		const html = renderMarkdown("[mail](mailto:someone@example.com)");
		expect(html).not.toContain("data-external");
	});

	it("wraps a fenced code block with a copy button carrying the raw source", () => {
		const html = renderMarkdown("```js\nconst x = 1;\n```");
		expect(html).toContain('class="code-block"');
		expect(html).toContain('data-language="js"');
		expect(html).toContain("data-copy=");
		// The copy button must carry the literal source, not the
		// syntax-highlighted HTML that gets rendered alongside it.
		expect(html).toContain("const x = 1;");
	});

	it("escapes HTML-significant characters in the copy button's payload", () => {
		const html = renderMarkdown('```js\nconst x = "<b>" && a > b;\n```');
		expect(html).not.toContain('data-copy="const x = "<b>"');
		expect(html).toContain("&lt;b&gt;");
	});

	it("renders plain prose as a paragraph", () => {
		const html = renderMarkdown("just an answer");
		expect(html.trim()).toBe("<p>just an answer</p>");
	});

	it("never renders raw HTML from the source", () => {
		// `html: false` in the MarkdownIt config: an agent's answer is
		// untrusted content rendered with `v-html`, so a script tag or similar
		// must come out as inert escaped text, not live markup.
		const html = renderMarkdown("<script>alert(1)</script>");
		expect(html).not.toContain("<script>alert(1)</script>");
	});
});
