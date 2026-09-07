import { defineStore, acceptHMRUpdate } from "pinia";
import { IpcService } from "@/services/ipc";
import type { AppSnapshot, AuthMode, ProviderStatus, Settings } from "@/helpers/types";

interface SettingsState {
	settings: Settings | null;
	providers: ProviderStatus[];
	defaultSystemPrompt: string;
	isLoading: boolean;
	isSaving: boolean;
	error: string | null;
	notice: string | null;
}

export const useSettingsStore = defineStore("settings", {
	state: (): SettingsState => ({
		settings: null,
		providers: [],
		defaultSystemPrompt: "",
		isLoading: false,
		isSaving: false,
		error: null,
		notice: null,
	}),
	getters: {
		activeProvider(): ProviderStatus | null {
			if (!this.settings) return null;
			return this.providers.find((p) => p.id === this.settings?.activeProvider) ?? null;
		},
		/**
		 * Providers offered in the launcher: installed and able to sign in,
		 * either with a saved key or the agent's own login. Anything else is
		 * only shown in settings, where it can be fixed.
		 */
		readyProviders(): ProviderStatus[] {
			return this.providers.filter((p) => p.binaryPath !== null && p.authenticated);
		},
		hasAnyProvider(): boolean {
			return this.readyProviders.length > 0;
		},
		hotkey(): string {
			return this.settings?.hotkey ?? "Ctrl+Space";
		},
	},
	actions: {
		apply(snapshot: AppSnapshot) {
			this.settings = snapshot.settings;
			this.providers = snapshot.providers;
			this.defaultSystemPrompt = snapshot.defaultSystemPrompt;
		},

		async load() {
			this.isLoading = true;
			this.error = null;
			try {
				this.apply(await IpcService.getSnapshot());
			} catch (err) {
				this.error = IpcService.errorMessage(err);
			} finally {
				this.isLoading = false;
			}
		},

		async save(patch: Partial<Settings>) {
			if (!this.settings) return;
			this.isSaving = true;
			this.error = null;
			try {
				this.apply(await IpcService.saveSettings({ ...this.settings, ...patch }));
				this.notice = "Saved";
			} catch (err) {
				const message = IpcService.errorMessage(err);
				// Re-read so the UI never shows a value the core rejected —
				// but `load()` itself clears `error` at its start (correctly,
				// for a fresh standalone load), which would otherwise wipe
				// this message before it's ever seen. Restore it after.
				await this.load();
				this.error = message;
			} finally {
				this.isSaving = false;
			}
		},

		async saveApiKey(keyId: string, value: string) {
			this.isSaving = true;
			this.error = null;
			try {
				this.apply(await IpcService.setApiKey(keyId, value));
				this.notice = "API key saved";
			} catch (err) {
				this.error = IpcService.errorMessage(err);
			} finally {
				this.isSaving = false;
			}
		},

		async setAuthMode(providerId: string, mode: AuthMode) {
			this.isSaving = true;
			this.error = null;
			try {
				this.apply(await IpcService.setAuthMode(providerId, mode));
				this.notice = "Sign-in method updated";
			} catch (err) {
				this.error = IpcService.errorMessage(err);
			} finally {
				this.isSaving = false;
			}
		},

		async setModel(providerId: string, model: string) {
			if (!this.settings) return;
			const providers = { ...this.settings.providers };
			const current = providers[providerId] ?? { enabled: true, model: null, authMode: null };
			providers[providerId] = { ...current, model: model.trim() || null };
			await this.save({ providers });
		},

		async selectProvider(providerId: string) {
			if (!this.settings) return;
			this.settings.activeProvider = providerId;
			try {
				await IpcService.setActiveProvider(providerId);
			} catch (err) {
				this.error = IpcService.errorMessage(err);
			}
		},

		/** Pass `null` to restore the managed scratch directory. */
		async setWorkingDir(path: string | null) {
			this.isSaving = true;
			this.error = null;
			try {
				this.apply(await IpcService.setWorkingDir(path));
				this.notice = path ? "Working directory updated" : "Restored the scratch directory";
			} catch (err) {
				this.error = IpcService.errorMessage(err);
			} finally {
				this.isSaving = false;
			}
		},
	},
});

if (import.meta.hot) {
	import.meta.hot.accept(acceptHMRUpdate(useSettingsStore, import.meta.hot));
}
