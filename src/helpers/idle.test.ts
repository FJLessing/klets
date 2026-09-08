import { describe, expect, it } from "vitest";
import { RESET_AFTER_HIDDEN_MS, shouldResetAfterHide } from "@/helpers/idle";

describe("shouldResetAfterHide", () => {
	const hiddenAt = 1_000_000;

	it("is false before the threshold has elapsed", () => {
		const now = hiddenAt + RESET_AFTER_HIDDEN_MS - 1;
		expect(shouldResetAfterHide(true, hiddenAt, now, false)).toBe(false);
	});

	it("is true once the threshold has elapsed", () => {
		const now = hiddenAt + RESET_AFTER_HIDDEN_MS;
		expect(shouldResetAfterHide(true, hiddenAt, now, false)).toBe(true);
	});

	it("is false when the setting is off, even long after the threshold", () => {
		const now = hiddenAt + RESET_AFTER_HIDDEN_MS * 10;
		expect(shouldResetAfterHide(false, hiddenAt, now, false)).toBe(false);
	});

	it("is false when the launcher was never hidden", () => {
		expect(shouldResetAfterHide(true, null, hiddenAt + RESET_AFTER_HIDDEN_MS, false)).toBe(false);
	});

	it("is false with a pending permission, even long after the threshold", () => {
		// That reappearance means the agent is waiting on an Allow/Deny
		// decision, not that the user forgot about the conversation.
		const now = hiddenAt + RESET_AFTER_HIDDEN_MS * 10;
		expect(shouldResetAfterHide(true, hiddenAt, now, true)).toBe(false);
	});
});
