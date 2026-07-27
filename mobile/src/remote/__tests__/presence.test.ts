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

describe("isDesktopOnline", () => {
  it("accepts a fresh online heartbeat", () => {
    expect(isDesktopOnline(presence(), now)).toBe(true);
  });

  it("rejects an explicit offline update", () => {
    expect(isDesktopOnline(presence({ online: false }), now)).toBe(false);
  });

  it("rejects a stale heartbeat at the same threshold as web", () => {
    const lastHeartbeatTs = (now - PRESENCE_STALE_AFTER_MS - 1_000) / 1000;
    expect(isDesktopOnline(presence({ lastHeartbeatTs }), now)).toBe(false);
  });

  it("does not confuse a NATS connection with desktop presence", () => {
    expect(isDesktopOnline(null, now)).toBe(false);
  });
});
