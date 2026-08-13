import {
  INITIAL_PRESENCE_STATE,
  isDesktopOnline,
  PRESENCE_STALE_AFTER_MS,
  type PresenceState,
} from "../presence";
import type { Presence } from "../types";

const now = 2_000_000_000_000;

function presence(overrides: Partial<Presence> = {}): Presence {
  return {
    online: true,
    pairId: "pair-1",
    bridgeInstanceId: "bridge-1",
    lastHeartbeatTs: now / 1000,
    sessions: [],
    ...overrides,
  };
}

function freshState(): PresenceState {
  return { ...INITIAL_PRESENCE_STATE };
}

type Result = PresenceState & { online: boolean };

describe("isDesktopOnline (relative heartbeat, L7)", () => {
  it("accepts the first observation as online (establishes the clock baseline)", () => {
    const result = isDesktopOnline(presence(), now, freshState());
    expect(result.online).toBe(true);
    expect(result.count).toBe(1);
  });

  it("rejects an explicit offline update", () => {
    expect(isDesktopOnline(presence({ online: false }), now, freshState()).online).toBe(false);
  });

  it("rejects null presence", () => {
    expect(isDesktopOnline(null, now, freshState()).online).toBe(false);
  });

  it("stays online when the clock is skewed but the delta is stable", () => {
    // Phone clock runs 61s slow vs the desktop — an absolute age check would
    // mark this permanently offline (L7). The baseline absorbs the offset.
    const skewed = 61_000;
    let state = isDesktopOnline(
      presence({ lastHeartbeatTs: (now - skewed) / 1000 }),
      now,
      freshState(),
    );
    expect(state.online).toBe(true);
    const result = isDesktopOnline(
      presence({ lastHeartbeatTs: (now - skewed + 1_000) / 1000 }),
      now + 1_000,
      state,
    );
    expect(result.online).toBe(true);
  });

  it("marks offline when the delta jumps past the stale window (heartbeat stopped)", () => {
    let state: Result = { ...freshState(), online: false };
    for (let i = 0; i < 3; i += 1) {
      state = isDesktopOnline(
        presence({ lastHeartbeatTs: (now - i * 1_000) / 1000 }),
        now,
        state,
      );
      expect(state.online).toBe(true);
    }
    const result = isDesktopOnline(
      presence({ lastHeartbeatTs: (now - (PRESENCE_STALE_AFTER_MS + 2_000)) / 1000 }),
      now,
      state,
    );
    expect(result.online).toBe(false);
  });

  it("recovers when the desktop resumes beating", () => {
    let state = isDesktopOnline(presence(), now, freshState());
    const stale = isDesktopOnline(
      presence({ lastHeartbeatTs: (now - (PRESENCE_STALE_AFTER_MS + 2_000)) / 1000 }),
      now,
      state,
    );
    expect(stale.online).toBe(false);
    const recovered = isDesktopOnline(presence({ lastHeartbeatTs: now / 1000 }), now, stale);
    expect(recovered.online).toBe(true);
  });

  // The review's flagged gap: a phone NTP re-sync after an outage jumps the
  // local clock by >60s. Heartbeats keep arriving at the new, steady offset —
  // that must be adopted as the new baseline, not wedged offline forever.
  it("adopts a new baseline after a sustained clock jump (heartbeats still arriving)", () => {
    // Healthy connection, phone clock in sync.
    let state: Result = { ...freshState(), online: false };
    state = isDesktopOnline(presence({ lastHeartbeatTs: now / 1000 }), now, state);
    state = isDesktopOnline(presence({ lastHeartbeatTs: (now + 1_000) / 1000 }), now + 1_000, state);
    expect(state.online).toBe(true);

    // NTP jumps the phone clock +90s. Each new heartbeat now arrives with a
    // delta ~90s larger than the baseline — out of the 60s window, but STABLE
    // at the new offset (the desktop keeps beating every second).
    const jump = 90_000;
    let t = now + 2_000;
    let jumpedState: typeof state = state;
    // Feed several heartbeats at the jumped offset; the first two read offline
    // (could be a dead desktop), but a sustained stable new offset re-baselines.
    const outcomes: boolean[] = [];
    for (let i = 0; i < 4; i += 1) {
      jumpedState = isDesktopOnline(
        presence({ lastHeartbeatTs: (t + jump) / 1000 }),
        t,
        jumpedState,
      );
      outcomes.push(jumpedState.online);
      t += 1_000;
    }
    // Early samples may be offline; by the end it must have recovered.
    expect(jumpedState.online).toBe(true);
    expect(outcomes[outcomes.length - 1]).toBe(true);
  });

  it("does NOT re-baseline when the desktop truly dies (delta keeps growing)", () => {
    let state = isDesktopOnline(presence({ lastHeartbeatTs: now / 1000 }), now, freshState());
    expect(state.online).toBe(true);
    // Desktop froze: lastHeartbeatTs stops advancing while now advances. Each
    // successive observation grows the delta — never a stable new offset.
    let t = now;
    const frozenTs = now / 1000;
    for (let i = 0; i < 5; i += 1) {
      t += PRESENCE_STALE_AFTER_MS + 5_000; // jump forward past the window each time
      state = isDesktopOnline(presence({ lastHeartbeatTs: frozenTs }), t, state);
      expect(state.online).toBe(false);
    }
  });

  // Regression for the residual edge the review flagged: the OLD discriminator
  // leaned on sampling-timer jitter — two ticks that happen to land ≤10ms apart
  // would read a frozen (dead) desktop as a "stable new offset" and adopt it.
  // The structural fix requires lastHeartbeatTs to ADVANCE between
  // confirmations, so a frozen heartbeat can never be adopted no matter how the
  // timer aligns.
  it("never adopts a dead desktop even when consecutive ticks align within tolerance", () => {
    const frozenTs = now / 1000;
    // Establish a healthy baseline first.
    let state = isDesktopOnline(presence({ lastHeartbeatTs: frozenTs }), now, freshState());
    expect(state.online).toBe(true);
    // The desktop dies: lastHeartbeatTs freezes at frozenTs. Push the clock far
    // enough forward that the first sample lands OUT of the stale window — this
    // seeds the jump detector (staleCount 1).
    let t = now + PRESENCE_STALE_AFTER_MS + 5_000;
    state = isDesktopOnline(presence({ lastHeartbeatTs: frozenTs }), t, state);
    expect(state.online).toBe(false);
    expect(state.staleCount).toBe(1);
    // Now walk forward in tiny steps (≤ tolerance) so each successive observed
    // delta sits within the window of the previous — the pathological alignment
    // that, under the old rule, would accumulate confirmations and adopt the
    // frozen offset. The advancing-heartbeat requirement blocks it: a frozen
    // lastHeartbeatTs can never be re-baselined.
    for (let i = 0; i < 6; i += 1) {
      t += 9;
      state = isDesktopOnline(presence({ lastHeartbeatTs: frozenTs }), t, state);
      expect(state.online).toBe(false);
      // staleCount must never climb to the confirm threshold while frozen.
      expect(state.staleCount).toBeLessThan(3);
    }
  });
});
