import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl } from "@tauri-apps/plugin-opener";
import type { AgentEvent, AppSnapshot, Settings } from "@/helpers/types";

/**
 * Single entry point to the Rust core.
 *
 * Mirrors the ApiService pattern: every call goes through here so error
 * shapes stay consistent and components never touch `invoke` directly.
 */
export class IpcService {
	static async getSnapshot(): Promise<AppSnapshot> {
		return invoke<AppSnapshot>("get_snapshot");
	}

	static async saveSettings(settings: Settings): Promise<AppSnapshot> {
		return invoke<AppSnapshot>("save_settings", { settings });
	}

	static async setApiKey(keyId: string, value: string): Promise<AppSnapshot> {
		return invoke<AppSnapshot>("set_api_key", { keyId, value });
	}

	static async setActiveProvider(providerId: string): Promise<void> {
		return invoke("set_active_provider", { providerId });
	}

	static async sendPrompt(text: string): Promise<number> {
		return invoke<number>("send_prompt", { text });
	}

	static async cancelTurn(): Promise<void> {
		return invoke("cancel_turn");
	}

	static async newChat(): Promise<number> {
		return invoke<number>("new_chat");
	}

	static async hideLauncher(): Promise<void> {
		return invoke("hide_launcher");
	}

	static async openSettings(): Promise<void> {
		return invoke("open_settings");
	}

	static async closeSettings(): Promise<void> {
		return invoke("close_settings");
	}

	static async resizeLauncher(height: number): Promise<void> {
		return invoke("resize_launcher", { height });
	}

	static async quit(): Promise<void> {
		return invoke("quit");
	}

	static onAgentEvent(handler: (event: AgentEvent) => void): Promise<UnlistenFn> {
		return listen<AgentEvent>("klets://agent", (event) => handler(event.payload));
	}

	static onFocus(handler: () => void): Promise<UnlistenFn> {
		return listen("klets://focus", () => handler());
	}

	/** Open a link in the user's browser rather than inside the launcher. */
	static async openExternal(url: string): Promise<void> {
		if (!/^https?:\/\//i.test(url)) return;
		await openUrl(url);
	}

	static errorMessage(error: unknown): string {
		if (typeof error === "string") return error;
		if (error instanceof Error) return error.message;
		return "Something went wrong";
	}
}
