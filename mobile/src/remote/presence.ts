import type { Presence } from "./types";

export const PRESENCE_STALE_AFTER_MS = 60_000;
/**
 * The desktop bridge beats once per second, so a *receipt gap* — no presence
 * packet arriving for this long — is a much faster death signal than the
 * 60s wall-clock staleness window (which exists to absorb clock skew, not to
 * detect death). Purely local timing, immune to inter-device clock offset.
 */
export const PRESENCE_RECEIPT_STALE_MS = 15_000;
/**
 * Consecutive out-of-window samples at a *stable, advancing* new offset before
 * we adopt it as the new baseline. The structural discriminator is the
 * desktop's own `lastHeartbeatTs`:
 *   - clock jump: heartbeats keep arriving, so `lastHeartbeatTs` advances each
 *     sample and the observed delta holds steady at the new (shifted) value.
 *   - dead desktop: heartbeats stop, `lastHeartbeatTs` freezes, and the delta
 *     just keeps growing — never advancing, never settling.
 * Requiring `lastHeartbeatTs` to advance between confirmations means a frozen
 * (dead) desktop can never be adopted as a new baseline, no matter how the
 * sampling timer happens to align.
 */
const CLOCK_JUMP_CONFIRM_SAMPLES = 3;
/** Two out-of-window samples within this of each other count as "same offset". */
const CLOCK_JUMP_TOLERANCE_MS = 10_000;

export interface PresenceState {
  deltaMs: number;
  count: number;
  /** Last out-of-window observation, to detect a stable (jumped) offset. */
  staleDeltaMs: number;
  staleCount: number;
  /** `lastHeartbeatTs` of the previous out-of-window sample (0 = none yet). */
  staleHeartbeatTs: number;
}

export const INITIAL_PRESENCE_STATE: PresenceState = {
  deltaMs: 0,
  count: 0,
  staleDeltaMs: 0,
  staleCount: 0,
  staleHeartbeatTs: 0,
};

/**
 * Desktop presence, judged by the local clock's *offset* from the desktop's
 * heartbeats rather than an absolute wall-clock comparison (L7). The desktop
 * timestamps are seconds on ITS clock; comparing `now - lastHeartbeatTs*1000`
 * directly is wrong whenever the two devices disagree by more than the stale
 * window — a phone running 61s slow would show the desktop permanently
 * offline.
 *
 * The baseline delta drifts gently with each healthy sample. A single sample
 * outside the window reads as "offline" (the desktop may have died). A clock
 * jump is confirmed structurally: several consecutive out-of-window samples
 * whose `lastHeartbeatTs` keeps ADVANCING while sitting at the same new offset
 * — proof the desktop is still beating, just shifted — and only then is the
 * new baseline adopted. A dead desktop's frozen `lastHeartbeatTs` never
 * satisfies that, so it can never wedge into a false "online".
 */
export function isDesktopOnline(
  presence: Presence | null,
  nowMs: number,
  state: PresenceState = INITIAL_PRESENCE_STATE,
): PresenceState & { online: boolean } {
  if (!presence?.online || !presence.lastHeartbeatTs) {
    return { ...state, online: false };
  }
  const heartbeatTs = presence.lastHeartbeatTs;
  const observedDeltaMs = nowMs - heartbeatTs * 1000;
  // Warm up on the first observation — a single sample can't judge drift.
  if (state.count === 0) {
    return {
      online: true,
      deltaMs: observedDeltaMs,
      count: 1,
      staleDeltaMs: 0,
      staleCount: 0,
      staleHeartbeatTs: 0,
    };
  }
  const driftMs = Math.abs(observedDeltaMs - state.deltaMs);
  if (driftMs <= PRESENCE_STALE_AFTER_MS) {
    // Healthy sample — keep the baseline drifting gently and clear the jump
    // detector.
    const blended = state.deltaMs + (observedDeltaMs - state.deltaMs) * 0.1;
    return {
      online: true,
      deltaMs: blended,
      count: state.count + 1,
      staleDeltaMs: 0,
      staleCount: 0,
      staleHeartbeatTs: 0,
    };
  }
  // Out of window. A clock-jump confirmation requires BOTH a stable new offset
  // AND the desktop's heartbeat actually advancing since the last out-of-window
  // sample — the structural proof that beats are still arriving (a jump), not
  // frozen (a death). The advancing-heartbeat condition removes any dependence
  // on sampling-timer alignment. The first out-of-window sample only seeds the
  // detector (staleCount 1); subsequent confirmations compare against it.
  const heartbeatAdvanced =
    state.staleCount > 0 && heartbeatTs > state.staleHeartbeatTs;
  const nearLastStale =
    Math.abs(observedDeltaMs - state.staleDeltaMs) <= CLOCK_JUMP_TOLERANCE_MS;
  const confirmed = heartbeatAdvanced && nearLastStale;
  const staleCount = confirmed ? state.staleCount + 1 : 1;
  if (staleCount >= CLOCK_JUMP_CONFIRM_SAMPLES) {
    return {
      online: true,
      deltaMs: observedDeltaMs,
      count: state.count + 1,
      staleDeltaMs: 0,
      staleCount: 0,
      staleHeartbeatTs: 0,
    };
  }
  // Record this sample as the reference for the next comparison. On a fresh
  // (unconfirmed) sample the streak restarts here.
  return {
    online: false,
    deltaMs: state.deltaMs,
    count: state.count,
    staleDeltaMs: observedDeltaMs,
    staleCount,
    staleHeartbeatTs: heartbeatTs,
  };
}
