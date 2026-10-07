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
	/**
	 * A `Stopped` failure that happened with no turn to attach it to — most
	 * commonly a pre-connect warm-up. There is no bubble for this, so unlike
	 * an ordinary turn error it needs its own place to render.
	 */
	idleError: string | null;
	activeTurn: number | null;
	/** Highest turn id the core has handed out, prompts and new chats alike. */
	lastTurn: number;
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
		idleError: null,
		activeTurn: null,
		lastTurn: 0,
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
		/** Whether the current bubble has an Allow/Deny decision still open. */
		hasPendingPermission(): boolean {
			return this.currentMessage?.permissions.some((p) => !p.outcome) ?? false;
		},
	},
	actions: {
		async send(text: string) {
			const trimmed = text.trim();
			if (!trimmed || this.isSending || this.isStreaming) return;

			this.isSending = true;
			this.error = null;
			this.idleError = null;

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
				const turn = await IpcService.sendPrompt(trimmed);
				this.activeTurn = turn;
				this.recordTurn(turn);
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

		/**
		 * Best-effort: start the agent ahead of the first prompt. A failure
		 * (a missing binary, most commonly) lands in `idleError` — the same
		 * place a handshake failure during a real turn would put it, since
		 * from the frontend's perspective both are "the agent tried to start
		 * and couldn't", just with no turn number attached.
		 */
		async warm() {
			try {
				await IpcService.warmAgent();
			} catch (err) {
				this.agentState = AgentState.Stopped;
				this.idleError = IpcService.errorMessage(err);
			}
		},

		async reset() {
			if (this.isStreaming) await this.cancel();
			this.messages = [];
			this.activeTurn = null;
			this.error = null;
			this.idleError = null;
			// A new chat may run on a different agent, so its list of tools
			// from the last one is no longer accurate.
			this.commands = [];
			try {
				this.recordTurn(await IpcService.newChat());
			} catch (err) {
				this.error = IpcService.errorMessage(err);
			}
		},

		/** Fold one streamed event into the transcript. */
		handleEvent(event: AgentEvent) {
			if (event.type === "status") {
				const wasStreaming = this.isStreaming;
				this.agentState = event.state;
				this.agentDetail = event.detail ?? null;

				if (event.state !== AgentState.Stopped) {
					// A fresh attempt is underway; an earlier idle failure no
					// longer describes what's happening.
					this.idleError = null;
				} else if (wasStreaming) {
					this.finishTurn(event.detail ?? "The agent stopped unexpectedly");
				} else if (event.detail) {
					this.idleError = event.detail;
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

			// A new chat's `done` can land after the next question was asked
			// (gemini's session/new takes seconds) and would close its bubble.
			if (!this.isCurrentTurn(event.turn)) return;

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

		recordTurn(turn: number) {
			this.lastTurn = Math.max(this.lastTurn, turn);
		},

		/**
		 * Turn 0 is connection-level and applies to whatever is in flight.
		 * While `sendPrompt` is still pending the new id is unknown, but any
		 * id not yet handed out can only be it.
		 */
		isCurrentTurn(turn: number): boolean {
			if (turn === 0) return true;
			if (this.activeTurn !== null) return turn === this.activeTurn;
			return turn > this.lastTurn;
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
