import { useEffect, useRef } from "react";

/**
 * How often a catalogue re-reads itself while the app is in front.
 *
 * The revision heartbeat already heals a reader that *missed* a packet: the
 * presence packet carries the desktop's catalogue revisions, and a client whose
 * applied revision is behind pulls. What it cannot heal is a reader that is
 * wrong while its revision is right — a row whose content disagrees with the
 * desktop for a reason the catalogue's signatures do not see (the workspace
 * heading that flickered between the real name and its fallback was exactly
 * that, and no revision was ever out of step).
 *
 * Ten minutes: short enough to bound how long such a mistake can be looked at,
 * long enough that a phone left open all day costs six pulls an hour — and each
 * of those is answered from the desktop's snapshot cache (~0.75 ms server-side,
 * measured) rather than from the store.
 */
export const CATALOG_SELF_HEAL_MS = 10 * 60 * 1000;

/**
 * Read the catalogue again on a slow clock, and once immediately on returning to
 * the front.
 *
 * `foreground` is the app's own lifecycle fact (`AppState`, via
 * `useAppLifecycle`), not a guess: a backgrounded phone holds no socket open and
 * must run no timer, so leaving the foreground disposes the interval here rather
 * than letting it tick and decide not to act. Coming back reads at once — the
 * time spent in the background is exactly the time a self-heal was skipped.
 *
 * Mounting does *not* read: a runtime that has just appeared is connecting, and
 * its own recovery reads the catalogue on the way up. This adds a read only for
 * a phone that was in front, left, and came back.
 */
export function useCatalogSelfHeal(
  refresh: () => unknown,
  { foreground, intervalMs = CATALOG_SELF_HEAL_MS }: { foreground: boolean; intervalMs?: number },
): void {
  const refreshRef = useRef(refresh);
  // Updated in an effect, not during render: a render that never commits must
  // not change which callback the timer would call.
  useEffect(() => {
    refreshRef.current = refresh;
  }, [refresh]);
  // Whether this runtime has been in front before: the difference between a
  // mount (the connection reads for itself) and a return.
  const seen = useRef(false);
  useEffect(() => {
    if (!foreground) return;
    if (seen.current) void refreshRef.current();
    seen.current = true;
    const timer = setInterval(() => {
      void refreshRef.current();
    }, intervalMs);
    return () => clearInterval(timer);
  }, [foreground, intervalMs]);
}
