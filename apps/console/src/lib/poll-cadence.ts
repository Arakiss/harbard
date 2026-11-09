/**
 * When the faces' slice (`GET /api/v1/workspace?only=activity`) is pulled.
 * It carries what the living logo and the desktop's Dock show: the faces,
 * open "Needs you" items (`needs`) and unread Direct (`directs`).
 *
 * A browser tab stops pulling while hidden. The desktop shell's window is
 * hidden whenever the operator closes it (close hides; the app keeps running), and
 * that is exactly when the Dock badge and bounce matter, so it keeps a
 * slower pull of the same light slice instead. The full workspace view is
 * pulled only while visible.
 */

/** Slice cadence while the shell is visible. */
export const VISIBLE_ACTIVITY_MS = 2_000;
/** Slice cadence while the desktop shell's window is hidden. */
export const HIDDEN_DESKTOP_MS = 10_000;

export function activityPoll(visible: boolean, desktop: boolean): { fetch: boolean; delay: number } {
  if (visible) return { fetch: true, delay: VISIBLE_ACTIVITY_MS };
  if (desktop) return { fetch: true, delay: HIDDEN_DESKTOP_MS };
  // Nothing to fetch; look again soon, and `visibilitychange` wakes it.
  return { fetch: false, delay: VISIBLE_ACTIVITY_MS };
}
