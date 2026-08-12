import type { Presence } from "./types";

export const PRESENCE_STALE_AFTER_MS = 60_000;

/**
 * Desktop presence, judged by the local clock's *offset* from the desktop's
 * heartbeats rather than an absolute wall-clock comparison (L7). The desktop
 * timestamps are seconds on ITS clock; comparing `now - lastHeartbeatTs*1000`
 * directly is wrong whenever the two devices disagree by more than the stale
 * window — a phone running 61s slow would show the desktop permanently
 * offline. Track the running delta and judge STALENESS by how far the delta
 * drifts from its own baseline, not by an absolute age.
 */
export function isDesktopOnline(
  presence: Presence | null,
  nowMs: number,
  state: { deltaMs: number; count: number } = { deltaMs: 0, count: 0 },
): { online: boolean; deltaMs: number; count: number } {
  if (!presence?.online || !presence.lastHeartbeatTs) {
    return { online: false, deltaMs: state.deltaMs, count: state.count };
  }
  const observedDeltaMs = nowMs - presence.lastHeartbeatTs * 1000;
  // Warm up on the first observation — a single sample can't judge drift.
  if (state.count === 0) {
    return { online: true, deltaMs: observedDeltaMs, count: 1 };
  }
  // If the baseline delta suddenly jumps by more than the stale window, the
  // desktop stopped beating (or our clock jumped); treat it as offline.
  const driftMs = Math.abs(observedDeltaMs - state.deltaMs);
  if (driftMs > PRESENCE_STALE_AFTER_MS) {
    return { online: false, deltaMs: state.deltaMs, count: state.count };
  }
  // Keep the baseline drifting gently with each healthy sample.
  const blended = state.deltaMs + (observedDeltaMs - state.deltaMs) * 0.1;
  return { online: true, deltaMs: blended, count: state.count + 1 };
}
