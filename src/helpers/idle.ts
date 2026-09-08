/** How long the launcher must sit hidden before reappearing clears the conversation. */
export const RESET_AFTER_HIDDEN_MS = 20 * 60 * 1000;

/**
 * Whether a launcher that was hidden should clear its conversation now that
 * it's shown again.
 *
 * A pure decision so it's testable without a DOM: given when the launcher
 * went dark, the time it's showing again, and whether the setting is even
 * on, has it been hidden long enough to count as "came back to a stale
 * conversation" rather than "just glanced away"?
 *
 * Skipped while a permission request is pending: that reappearance means the
 * agent is waiting on an Allow/Deny decision, not that the user forgot about
 * the conversation, and clearing it out from under a live decision would be
 * wrong.
 */
export function shouldResetAfterHide(
	enabled: boolean,
	hiddenAt: number | null,
	now: number,
	hasPendingPermission: boolean,
): boolean {
	if (!enabled || hiddenAt === null || hasPendingPermission) return false;
	return now - hiddenAt >= RESET_AFTER_HIDDEN_MS;
}
