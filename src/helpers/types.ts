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

/** How a provider authenticates. Chosen explicitly, never inferred. */
export enum AuthMode {
	Subscription = "subscription",
	ApiKey = "apiKey",
}

/** What an agent is allowed to do during a session. */
export enum ToolPolicy {
	Off = "off",
	ReadOnly = "readOnly",
	AskToRun = "askToRun",
}

export interface PendingPermission {
	id: number;
	title: string;
	kind: string;
	detail: string | null;
	/** Set once answered, so the row can show the outcome. */
	outcome?: "allowed" | "denied" | "timedOut";
}

export interface AgentCommandInfo {
	name: string;
	description: string;
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
	hasSubscriptionToken: boolean;
	authenticated: boolean;
	authMode: AuthMode;
	authModes: AuthMode[];
	subscriptionKeyId: string | null;
	/** Set when this provider actually runs on another agent. */
	runsVia: string | null;
	model: string | null;
}

export interface ProviderSettings {
	enabled: boolean;
	model: string | null;
	authMode: AuthMode | null;
}

export interface Settings {
	hotkey: string;
	activeProvider: string;
	hideOnBlur: boolean;
	launchAtLogin: boolean;
	providers: Record<string, ProviderSettings>;
	toolPolicy: ToolPolicy;
	systemPrompt: string;
	workingDir: string | null;
	resetWhenHidden: boolean;
}

export interface AppSnapshot {
	settings: Settings;
	providers: ProviderStatus[];
	agentEvent: string;
	activeProvider: string | null;
	/** The seed prompt, so "reset to default" never has to duplicate it. */
	defaultSystemPrompt: string;
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
	permissions: PendingPermission[];
	error: string | null;
	streaming: boolean;
}

/** Streamed from the Rust core over the `klets://agent` event. */
export type AgentEvent =
	| { type: "status"; provider: string; state: AgentState; detail?: string }
	| { type: "chunk"; turn: number; text: string }
	| { type: "thought"; turn: number; text: string }
	| { type: "tool"; turn: number; id: string; title: string; kind: string; status: string }
	| { type: "permissionDenied"; turn: number; title: string; kind: string }
	| { type: "permissionRequest"; turn: number; request: PendingPermission }
	| {
			type: "permissionResolved";
			turn: number;
			id: number;
			allowed: boolean;
			timedOut: boolean;
	  }
	| { type: "capabilities"; provider: string; commands: AgentCommandInfo[] }
	| { type: "notice"; turn: number; message: string }
	| { type: "done"; turn: number; stopReason?: string }
	| { type: "error"; turn: number; message: string };
