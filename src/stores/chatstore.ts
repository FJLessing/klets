import { defineStore, acceptHMRUpdate } from "pinia";
import { IpcService } from "@/services/ipc";
import {
	AgentState,
	MessageRole,
	type AgentCommandInfo,
	type AgentEvent,
	type ChatMessage,
} from "@/helpers/types";

interface ChatState {
	messages: ChatMessage[];
	/** Commands and skills the connected agent advertises. */
	commands: AgentCommandInfo[];
	agentState: AgentState;
	agentDetail: string | null;
	activeTurn: number | null;
	isSending: boolean;
	error: string | null;
	nextId: number;
}

function blankAgentMessage(id: number): ChatMessage {
	return {
		id,
		role: MessageRole.Agent,
		text: "",
		thoughts: "",
		tools: [],
		notices: [],
		permissions: [],
		error: null,
		streaming: true,
	};
}

export const useChatStore = defineStore("chat", {
	state: (): ChatState => ({
		messages: [],
		commands: [],
		agentState: AgentState.Stopped,
		agentDetail: null,
		activeTurn: null,
		isSending: false,
		error: null,
		nextId: 1,
	}),
	getters: {
		isEmpty(): boolean {
			return this.messages.length === 0;
		},
		isStreaming(): boolean {
			return this.activeTurn !== null;
		},
		/** The agent bubble currently being written into. */
		currentMessage(): ChatMessage | null {
			for (let i = this.messages.length - 1; i >= 0; i -= 1) {
				if (this.messages[i].role === MessageRole.Agent) return this.messages[i];
			}
			return null;
		},
		/** Shown while the agent is starting up or connecting. */
		statusLabel(): string | null {
			if (this.agentState === AgentState.Starting) return "Starting agent…";
			if (this.agentState === AgentState.Connecting) return "Connecting…";
			return null;
		},
	},
	actions: {
		async send(text: string) {
			const trimmed = text.trim();
			if (!trimmed || this.isSending || this.isStreaming) return;

			this.isSending = true;
			this.error = null;

			this.messages.push({
				id: this.nextId++,
				role: MessageRole.User,
				text: trimmed,
				thoughts: "",
				tools: [],
				notices: [],
				permissions: [],
				error: null,
				streaming: false,
			});
			const pending = blankAgentMessage(this.nextId++);
			this.messages.push(pending);

			try {
				this.activeTurn = await IpcService.sendPrompt(trimmed);
			} catch (err) {
				const message = IpcService.errorMessage(err);
				pending.error = message;
				pending.streaming = false;
				this.error = message;
				this.activeTurn = null;
			} finally {
				this.isSending = false;
			}
		},

		async cancel() {
			if (!this.isStreaming) return;
			try {
				await IpcService.cancelTurn();
			} catch (err) {
				this.error = IpcService.errorMessage(err);
			}
		},

		async reset() {
			if (this.isStreaming) await this.cancel();
			this.messages = [];
			this.activeTurn = null;
			this.error = null;
			// A new chat may run on a different agent, so its list of tools
			// from the last one is no longer accurate.
			this.commands = [];
			try {
				await IpcService.newChat();
			} catch (err) {
				this.error = IpcService.errorMessage(err);
			}
		},

		/** Fold one streamed event into the transcript. */
		handleEvent(event: AgentEvent) {
			if (event.type === "status") {
				this.agentState = event.state;
				this.agentDetail = event.detail ?? null;
				if (event.state === AgentState.Stopped && this.isStreaming) {
					this.finishTurn(event.detail ?? "The agent stopped unexpectedly");
				}
				return;
			}

			// Whatever the agent reports it can do, which is the only reliable
			// view of the MCP servers and skills it loaded from its own config.
			if (event.type === "capabilities") {
				this.commands = event.commands;
				return;
			}

			const message = this.currentMessage;
			if (!message) return;

			// Notices describe the session rather than the turn (an unavailable
			// model, for example), so they attach even after a turn closes.
			if (event.type === "notice") {
				message.notices.push(event.message);
				return;
			}

			// A pending decision must render — and resolve — even if the turn
			// is being torn down. Cancelling a turn (or the agent crashing)
			// resolves any outstanding permission via the broker's `clear()`,
			// which races against the `done`/`error` that closes the message;
			// if that race is lost, the resolution must still land, or the
			// row is left showing live Allow/Deny buttons for a decision
			// that's already been made.
			if (event.type === "permissionRequest") {
				message.permissions.push(event.request);
				return;
			}

			if (event.type === "permissionResolved") {
				const pending = message.permissions.find((p) => p.id === event.id);
				if (pending) {
					pending.outcome = event.timedOut
						? "timedOut"
						: event.allowed
							? "allowed"
							: "denied";
				}
				return;
			}

			// Late events from a cancelled turn must not append to a bubble
			// that has already been closed off.
			if (!message.streaming) return;

			switch (event.type) {
				case "chunk":
					message.text += event.text;
					break;
				case "thought":
					message.thoughts += event.text;
					break;
				case "tool": {
					const existing = message.tools.find((t) => t.id === event.id);
					if (existing) {
						if (event.title) existing.title = event.title;
						if (event.kind) existing.kind = event.kind;
						if (event.status) existing.status = event.status;
					} else {
						message.tools.push({
							id: event.id,
							title: event.title || "Tool call",
							kind: event.kind,
							status: event.status,
						});
					}
					break;
				}
				case "permissionDenied":
					message.notices.push(
						`Refused ${event.title} — Klets does not allow ${event.kind} operations.`,
					);
					break;
				case "done":
					this.finishTurn(null);
					break;
				case "error":
					this.finishTurn(event.message);
					break;
			}
		},

		finishTurn(error: string | null) {
			const message = this.currentMessage;
			if (message) {
				message.streaming = false;
				if (error) message.error = error;
				// An empty answer with no error still needs to say something.
				if (!error && !message.text.trim()) {
					message.text = "_No response._";
				}
			}
			if (error) this.error = error;
			this.activeTurn = null;
		},
	},
});

if (import.meta.hot) {
	import.meta.hot.accept(acceptHMRUpdate(useChatStore, import.meta.hot));
}
