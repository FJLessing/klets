import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useChatStore } from "@/stores/chatstore";
import { AgentState, MessageRole } from "@/helpers/types";

// The store's actions call through `IpcService`, which itself calls the real
// Tauri bridge (`window.__TAURI_INTERNALS__`) — nonexistent outside a running
// app. Mocking the service at this boundary keeps these tests about the
// store's own state-transition logic, not Tauri's plumbing.
vi.mock("@/services/ipc", () => ({
	IpcService: {
		sendPrompt: vi.fn().mockResolvedValue(1),
		cancelTurn: vi.fn().mockResolvedValue(undefined),
		newChat: vi.fn().mockResolvedValue(2),
		warmAgent: vi.fn().mockResolvedValue(undefined),
		errorMessage: (error: unknown) =>
			typeof error === "string" ? error : error instanceof Error ? error.message : "Something went wrong",
	},
}));

import { IpcService } from "@/services/ipc";

beforeEach(() => {
	setActivePinia(createPinia());
	vi.mocked(IpcService.sendPrompt).mockReset().mockResolvedValue(1);
	vi.mocked(IpcService.cancelTurn).mockReset().mockResolvedValue(undefined);
	vi.mocked(IpcService.newChat).mockReset().mockResolvedValue(2);
	vi.mocked(IpcService.warmAgent).mockReset().mockResolvedValue(undefined);
});

describe("chatstore: sending a prompt", () => {
	it("pushes the user message and a streaming agent placeholder, then records the turn id", async () => {
		const store = useChatStore();

		await store.send("what is 2+2?");

		expect(store.messages).toHaveLength(2);
		expect(store.messages[0]).toMatchObject({ role: MessageRole.User, text: "what is 2+2?" });
		expect(store.messages[1]).toMatchObject({ role: MessageRole.Agent, streaming: true, text: "" });
		expect(store.activeTurn).toBe(1);
		expect(store.isStreaming).toBe(true);
	});

	it("ignores a blank prompt without touching the transcript", async () => {
		const store = useChatStore();
		await store.send("   ");
		expect(store.messages).toHaveLength(0);
		expect(IpcService.sendPrompt).not.toHaveBeenCalled();
	});

	it("closes the placeholder with an error when the core rejects the prompt", async () => {
		vi.mocked(IpcService.sendPrompt).mockRejectedValueOnce(new Error("no provider configured"));
		const store = useChatStore();

		await store.send("hello");

		expect(store.activeTurn).toBeNull();
		expect(store.error).toBe("no provider configured");
		expect(store.currentMessage).toMatchObject({ streaming: false, error: "no provider configured" });
	});
});

describe("chatstore: streaming events", () => {
	async function seedTurn(store: ReturnType<typeof useChatStore>) {
		await store.send("hello");
	}

	it("accumulates chunk and thought text onto the current message", async () => {
		const store = useChatStore();
		await seedTurn(store);

		store.handleEvent({ type: "chunk", turn: 1, text: "Hel" });
		store.handleEvent({ type: "chunk", turn: 1, text: "lo" });
		store.handleEvent({ type: "thought", turn: 1, text: "thinking…" });

		expect(store.currentMessage?.text).toBe("Hello");
		expect(store.currentMessage?.thoughts).toBe("thinking…");
	});

	it("adds a tool chip on first sight and updates it in place by id thereafter", async () => {
		const store = useChatStore();
		await seedTurn(store);

		store.handleEvent({ type: "tool", turn: 1, id: "t1", title: "Read file", kind: "read", status: "pending" });
		store.handleEvent({ type: "tool", turn: 1, id: "t1", title: "Read file", kind: "read", status: "completed" });

		expect(store.currentMessage?.tools).toHaveLength(1);
		expect(store.currentMessage?.tools[0].status).toBe("completed");
	});

	it("finishes the turn on done, substituting placeholder text for a genuinely empty answer", async () => {
		const store = useChatStore();
		await seedTurn(store);

		store.handleEvent({ type: "done", turn: 1 });

		expect(store.isStreaming).toBe(false);
		expect(store.currentMessage?.streaming).toBe(false);
		expect(store.currentMessage?.text).toBe("_No response._");
	});

	it("does not stamp placeholder text over a real answer", async () => {
		const store = useChatStore();
		await seedTurn(store);
		store.handleEvent({ type: "chunk", turn: 1, text: "42" });
		store.handleEvent({ type: "done", turn: 1 });
		expect(store.currentMessage?.text).toBe("42");
	});

	it("finishes the turn with an error message on an error event", async () => {
		const store = useChatStore();
		await seedTurn(store);

		store.handleEvent({ type: "error", turn: 1, message: "the agent crashed" });

		expect(store.isStreaming).toBe(false);
		expect(store.error).toBe("the agent crashed");
		expect(store.currentMessage?.error).toBe("the agent crashed");
	});

	it("ignores a late chunk for a turn that has already finished", async () => {
		// A cancelled or superseded turn's agent process can still have output
		// in flight; once the bubble is closed, further chunks must not
		// reopen or corrupt it.
		const store = useChatStore();
		await seedTurn(store);
		store.handleEvent({ type: "done", turn: 1 });
		const textAfterDone = store.currentMessage?.text;

		store.handleEvent({ type: "chunk", turn: 1, text: "stray output" });

		expect(store.currentMessage?.text).toBe(textAfterDone);
	});

	it("treats a Stopped status while streaming as an unexpected failure", async () => {
		const store = useChatStore();
		await seedTurn(store);

		store.handleEvent({ type: "status", provider: "claude", state: AgentState.Stopped });

		expect(store.isStreaming).toBe(false);
		expect(store.currentMessage?.error).toBe("The agent stopped unexpectedly");
		// The bubble already shows it; a duplicate standalone banner would be
		// redundant and confusing.
		expect(store.idleError).toBeNull();
	});

	it("records discovered commands without touching the transcript", async () => {
		const store = useChatStore();
		await seedTurn(store);

		store.handleEvent({
			type: "capabilities",
			provider: "claude",
			commands: [{ name: "review", description: "Review a diff" }],
		});

		expect(store.commands).toEqual([{ name: "review", description: "Review a diff" }]);
		expect(store.currentMessage?.text).toBe("");
	});

	it("records a refusal as a notice naming the tool kind", async () => {
		const store = useChatStore();
		await seedTurn(store);

		store.handleEvent({ type: "permissionDenied", turn: 1, title: "Write config.json", kind: "edit" });

		expect(store.currentMessage?.notices).toEqual([
			"Refused Write config.json — Klets does not allow edit operations.",
		]);
	});
});

describe("chatstore: idle failures (warm-up, most commonly)", () => {
	it("surfaces a Stopped failure with no active turn as idleError", () => {
		const store = useChatStore();

		store.handleEvent({
			type: "status",
			provider: "claude",
			state: AgentState.Stopped,
			detail: "claude-agent-acp is not on PATH",
		});

		expect(store.idleError).toBe("claude-agent-acp is not on PATH");
		// There is no bubble to attach it to, since nothing was streaming.
		expect(store.messages).toHaveLength(0);
	});

	it("does not surface a clean stop (no detail) as an error", () => {
		const store = useChatStore();

		store.handleEvent({ type: "status", provider: "claude", state: AgentState.Stopped });

		expect(store.idleError).toBeNull();
	});

	it("clears a stale idleError once a fresh connection attempt starts", () => {
		const store = useChatStore();
		store.handleEvent({
			type: "status",
			provider: "claude",
			state: AgentState.Stopped,
			detail: "boom",
		});
		expect(store.idleError).toBe("boom");

		store.handleEvent({ type: "status", provider: "claude", state: AgentState.Starting });

		expect(store.idleError).toBeNull();
	});

	it("warm() succeeds silently when the core accepts it", async () => {
		const store = useChatStore();
		await store.warm();
		expect(IpcService.warmAgent).toHaveBeenCalledOnce();
		expect(store.idleError).toBeNull();
	});

	it("warm() records a rejection as an idleError", async () => {
		vi.mocked(IpcService.warmAgent).mockRejectedValueOnce(new Error("no provider configured"));
		const store = useChatStore();

		await store.warm();

		expect(store.agentState).toBe(AgentState.Stopped);
		expect(store.idleError).toBe("no provider configured");
	});
});

describe("chatstore: hasPendingPermission", () => {
	async function seedTurn(store: ReturnType<typeof useChatStore>) {
		await store.send("run the tests");
	}

	it("is false with no messages", () => {
		const store = useChatStore();
		expect(store.hasPendingPermission).toBe(false);
	});

	it("is true while a permission request has no outcome yet", async () => {
		const store = useChatStore();
		await seedTurn(store);
		store.handleEvent({
			type: "permissionRequest",
			turn: 1,
			request: { id: 1, title: "Run: rm -rf build", kind: "execute", detail: null },
		});

		expect(store.hasPendingPermission).toBe(true);
	});

	it("is false once the permission has been resolved", async () => {
		const store = useChatStore();
		await seedTurn(store);
		store.handleEvent({
			type: "permissionRequest",
			turn: 1,
			request: { id: 1, title: "Run: rm -rf build", kind: "execute", detail: null },
		});
		store.handleEvent({ type: "permissionResolved", turn: 1, id: 1, allowed: true, timedOut: false });

		expect(store.hasPendingPermission).toBe(false);
	});
});

describe("chatstore: permission requests and notices survive a closed turn", () => {
	async function seedTurn(store: ReturnType<typeof useChatStore>) {
		await store.send("run the tests");
	}

	it("attaches a notice even after the turn has finished", async () => {
		// Model-selection notices arrive from session setup, which can
		// complete after the turn that triggered the spawn has already
		// finished (or failed).
		const store = useChatStore();
		await seedTurn(store);
		store.handleEvent({ type: "done", turn: 1 });

		store.handleEvent({ type: "notice", turn: 1, message: "requested model unavailable" });

		expect(store.currentMessage?.notices).toContain("requested model unavailable");
	});

	it("resolves a pending permission even when it arrives after the turn has closed", async () => {
		// Regression: cancelling a turn (or the agent crashing) resolves any
		// outstanding permission through the broker's `clear()`, which races
		// against the `done`/`error` that closes the message. If that race is
		// lost, the outcome must still land — otherwise the row is stuck
		// showing live Allow/Deny buttons for a decision already made.
		const store = useChatStore();
		await seedTurn(store);

		store.handleEvent({
			type: "permissionRequest",
			turn: 1,
			request: { id: 7, title: "Run: rm -rf build", kind: "execute", detail: "rm -rf build" },
		});

		// The turn closes (e.g. the agent crashed) before the decision comes back.
		store.handleEvent({ type: "error", turn: 1, message: "the agent crashed" });
		expect(store.currentMessage?.streaming).toBe(false);

		store.handleEvent({ type: "permissionResolved", turn: 1, id: 7, allowed: false, timedOut: false });

		expect(store.currentMessage?.permissions[0].outcome).toBe("denied");
	});

	it("still allows a genuinely new request to render after the turn closes", async () => {
		const store = useChatStore();
		await seedTurn(store);
		store.handleEvent({ type: "done", turn: 1 });

		store.handleEvent({
			type: "permissionRequest",
			turn: 1,
			request: { id: 9, title: "Run: echo hi", kind: "execute", detail: "echo hi" },
		});

		expect(store.currentMessage?.permissions).toHaveLength(1);
	});
});

describe("chatstore: cancel and reset", () => {
	it("does nothing when there is no active turn", async () => {
		const store = useChatStore();
		await store.cancel();
		expect(IpcService.cancelTurn).not.toHaveBeenCalled();
	});

	it("reset clears the transcript, the active turn, and discovered commands", async () => {
		const store = useChatStore();
		await store.send("hello");
		store.handleEvent({
			type: "capabilities",
			provider: "claude",
			commands: [{ name: "review", description: "" }],
		});
		store.handleEvent({ type: "done", turn: 1 });

		await store.reset();

		expect(store.messages).toHaveLength(0);
		expect(store.activeTurn).toBeNull();
		expect(store.commands).toHaveLength(0);
		expect(IpcService.newChat).toHaveBeenCalledOnce();
	});

	it("cancels an in-flight turn before clearing it", async () => {
		const store = useChatStore();
		await store.send("hello");

		await store.reset();

		expect(IpcService.cancelTurn).toHaveBeenCalledOnce();
	});
});
