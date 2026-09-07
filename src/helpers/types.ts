export enum AgentState {
	Starting = "starting",
	Connecting = "connecting",
	Ready = "ready",
	Stopped = "stopped",
}

export enum MessageRole {
	User = "user",
	Agent = "agent",
}

export interface ProviderStatus {
	id: string;
	name: string;
	command: string;
	keyId: string;
	keyLabel: string;
	installHint: string;
	keyUrl: string;
	loginHint: string;
	defaultModel: string | null;
	binaryPath: string | null;
	hasApiKey: boolean;
	authenticated: boolean;
	model: string | null;
}

export interface ProviderSettings {
	enabled: boolean;
	model: string | null;
}

export interface Settings {
	hotkey: string;
	activeProvider: string;
	hideOnBlur: boolean;
	launchAtLogin: boolean;
	providers: Record<string, ProviderSettings>;
}

export interface AppSnapshot {
	settings: Settings;
	providers: ProviderStatus[];
	agentEvent: string;
	activeProvider: string | null;
}

export interface ToolActivity {
	id: string;
	title: string;
	kind: string;
	status: string;
}

export interface ChatMessage {
	id: number;
	role: MessageRole;
	text: string;
	thoughts: string;
	tools: ToolActivity[];
	notices: string[];
	error: string | null;
	streaming: boolean;
}

/** Streamed from the Rust core over the `klets://agent` event. */
export type AgentEvent =
	| { type: "status"; provider: string; state: AgentState; detail?: string }
	| { type: "chunk"; turn: number; text: string }
	| { type: "thought"; turn: number; text: string }
	| { type: "tool"; turn: number; id: string; title: string; kind: string; status: string }
	| { type: "permissionDenied"; turn: number; title: string }
	| { type: "done"; turn: number; stopReason?: string }
	| { type: "error"; turn: number; message: string };
