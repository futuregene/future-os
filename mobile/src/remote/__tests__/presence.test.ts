import { isDesktopOnline, PRESENCE_STALE_AFTER_MS } from "../presence";
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

describe("isDesktopOnline (relative heartbeat, L7)", () => {
  it("accepts the first observation as online (establishes the clock baseline)", () => {
    const result = isDesktopOnline(presence(), now);
    expect(result.online).toBe(true);
    expect(result.count).toBe(1);
  });

  it("rejects an explicit offline update", () => {
    expect(isDesktopOnline(presence({ online: false }), now).online).toBe(false);
  });

  it("rejects null presence", () => {
    expect(isDesktopOnline(null, now).online).toBe(false);
  });

  it("stays online when the clock is skewed but the delta is stable", () => {
    // Phone clock runs 61s slow vs the desktop — an absolute age check would
    // mark this permanently offline (L7). The baseline absorbs the offset.
    let state: ReturnType<typeof isDesktopOnline> = { deltaMs: 0, count: 0, online: false };
    const skewed = 61_000;
    state = isDesktopOnline(presence({ lastHeartbeatTs: (now - skewed) / 1000 }), now, state);
    expect(state.online).toBe(true);
    // A healthy second heartbeat at the same skew keeps it online.
    const next = state;
    const result = isDesktopOnline(
      presence({ lastHeartbeatTs: (now - skewed + 1_000) / 1000 }),
      now + 1_000,
      next,
    );
    expect(result.online).toBe(true);
  });

  it("marks offline when the delta jumps by the stale window (heartbeat stopped)", () => {
    // Desktop beats every second for a while…
    let state: ReturnType<typeof isDesktopOnline> = { deltaMs: 0, count: 0, online: false };
    for (let i = 0; i < 3; i += 1) {
      state = isDesktopOnline(
        presence({ lastHeartbeatTs: (now - i * 1_000) / 1000 }),
        now,
        state,
      );
      expect(state.online).toBe(true);
    }
    // …then stops. The observed delta grows past the stale window.
    const result = isDesktopOnline(
      presence({ lastHeartbeatTs: (now - (PRESENCE_STALE_AFTER_MS + 2_000)) / 1000 }),
      now,
      state,
    );
    expect(result.online).toBe(false);
  });

  it("recovers when the desktop resumes beating", () => {
    let state: ReturnType<typeof isDesktopOnline> = { deltaMs: 0, count: 0, online: false };
    state = isDesktopOnline(presence(), now, state);
    // Offline for a minute.
    const stale = isDesktopOnline(
      presence({ lastHeartbeatTs: (now - (PRESENCE_STALE_AFTER_MS + 2_000)) / 1000 }),
      now,
      state,
    );
    expect(stale.online).toBe(false);
    // Fresh heartbeat re-establishes online.
    const recovered = isDesktopOnline(
      presence({ lastHeartbeatTs: now / 1000 }),
      now,
      stale,
    );
    expect(recovered.online).toBe(true);
  });
});
