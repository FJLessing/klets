import { beforeEach, describe, expect, it, vi } from "vitest";
import { createPinia, setActivePinia } from "pinia";
import { useSettingsStore } from "@/stores/settingsstore";
import { AuthMode, ToolPolicy, type AppSnapshot, type Settings } from "@/helpers/types";

vi.mock("@/services/ipc", () => ({
	IpcService: {
		getSnapshot: vi.fn(),
		saveSettings: vi.fn(),
		errorMessage: (error: unknown) => (error instanceof Error ? error.message : "Something went wrong"),
	},
}));

import { IpcService } from "@/services/ipc";

function makeSettings(overrides: Partial<Settings> = {}): Settings {
	return {
		hotkey: "Ctrl+Space",
		activeProvider: "claude",
		hideOnBlur: true,
		launchAtLogin: false,
		providers: {},
		toolPolicy: ToolPolicy.AskToRun,
		systemPrompt: "default prompt",
		workingDir: null,
		resetWhenHidden: true,
		...overrides,
	};
}

function makeSnapshot(settings: Settings): AppSnapshot {
	return {
		settings,
		providers: [],
		agentEvent: "klets://agent",
		activeProvider: null,
		defaultSystemPrompt: "default prompt",
	};
}

beforeEach(() => {
	setActivePinia(createPinia());
	vi.mocked(IpcService.getSnapshot).mockReset();
	vi.mocked(IpcService.saveSettings).mockReset();
});

describe("settingsstore: loading a snapshot", () => {
	it("exposes the default prompt separately from the current one, for the Reset button", async () => {
		vi.mocked(IpcService.getSnapshot).mockResolvedValue(
			makeSnapshot(makeSettings({ systemPrompt: "a prompt the user edited" })),
		);
		const store = useSettingsStore();

		await store.load();

		expect(store.settings?.systemPrompt).toBe("a prompt the user edited");
		expect(store.defaultSystemPrompt).toBe("default prompt");
	});
});

describe("settingsstore: setModel", () => {
	it("creates a provider entry with a safe default shape when none exists yet", async () => {
		const settings = makeSettings();
		vi.mocked(IpcService.getSnapshot).mockResolvedValue(makeSnapshot(settings));
		vi.mocked(IpcService.saveSettings).mockImplementation(async (next) => makeSnapshot(next));
		const store = useSettingsStore();
		await store.load();

		await store.setModel("gemini", "gemini-3.6-flash");

		expect(IpcService.saveSettings).toHaveBeenCalledWith(
			expect.objectContaining({
				providers: { gemini: { enabled: true, model: "gemini-3.6-flash", authMode: null } },
			}),
		);
	});

	it("preserves an existing provider's authMode when only the model changes", async () => {
		const settings = makeSettings({
			providers: { claude: { enabled: true, model: null, authMode: AuthMode.ApiKey } },
		});
		vi.mocked(IpcService.getSnapshot).mockResolvedValue(makeSnapshot(settings));
		vi.mocked(IpcService.saveSettings).mockImplementation(async (next) => makeSnapshot(next));
		const store = useSettingsStore();
		await store.load();

		await store.setModel("claude", "claude-opus");

		expect(IpcService.saveSettings).toHaveBeenCalledWith(
			expect.objectContaining({
				providers: { claude: { enabled: true, model: "claude-opus", authMode: AuthMode.ApiKey } },
			}),
		);
	});

	it("clears the override with a blank model rather than saving whitespace", async () => {
		const settings = makeSettings({
			providers: { claude: { enabled: true, model: "claude-opus", authMode: null } },
		});
		vi.mocked(IpcService.getSnapshot).mockResolvedValue(makeSnapshot(settings));
		vi.mocked(IpcService.saveSettings).mockImplementation(async (next) => makeSnapshot(next));
		const store = useSettingsStore();
		await store.load();

		await store.setModel("claude", "   ");

		expect(IpcService.saveSettings).toHaveBeenCalledWith(
			expect.objectContaining({
				providers: { claude: { enabled: true, model: null, authMode: null } },
			}),
		);
	});
});

describe("settingsstore: save failure", () => {
	it("re-loads from the core so the UI never shows a value the core rejected", async () => {
		const original = makeSettings({ hotkey: "Ctrl+Space" });
		vi.mocked(IpcService.getSnapshot).mockResolvedValue(makeSnapshot(original));
		const store = useSettingsStore();
		await store.load();

		vi.mocked(IpcService.saveSettings).mockRejectedValueOnce(new Error("'Ctrl+Z' is not a valid shortcut"));
		vi.mocked(IpcService.getSnapshot).mockResolvedValueOnce(makeSnapshot(original));

		await store.save({ hotkey: "Ctrl+Z" });

		expect(store.error).toBe("'Ctrl+Z' is not a valid shortcut");
		expect(store.settings?.hotkey).toBe("Ctrl+Space");
	});
});
