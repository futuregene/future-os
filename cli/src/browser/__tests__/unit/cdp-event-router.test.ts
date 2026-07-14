/**
 * Unit tests for CdpEventRouter — no browser needed.
 */
import { describe, test, expect } from "bun:test";
import { CdpEventRouter } from "../../chromium/cdp-event-router.js";

describe("CdpEventRouter", () => {
  test("dispatches events to matching sessionId+method", () => {
    const router = new CdpEventRouter();
    const received: unknown[] = [];

    router.add("session-1", "Page.loadEventFired", (params) => {
      received.push({ session: "session-1", params });
    });

    router.dispatch("session-1", "Page.loadEventFired", { timestamp: 123 });
    expect(received.length).toBe(1);
  });

  test("does NOT dispatch to wrong sessionId", () => {
    const router = new CdpEventRouter();
    const received: unknown[] = [];

    router.add("session-1", "Page.loadEventFired", (params) => {
      received.push({ session: "session-1", params });
    });

    // Dispatch to session-2 — should NOT trigger session-1 handler
    router.dispatch("session-2", "Page.loadEventFired", { timestamp: 456 });
    expect(received.length).toBe(0);
  });

  test("does NOT dispatch to wrong method", () => {
    const router = new CdpEventRouter();
    const received: unknown[] = [];

    router.add("session-1", "Page.loadEventFired", () => {
      received.push("wrong");
    });

    router.dispatch("session-1", "Page.domContentEventFired", {});
    expect(received.length).toBe(0);
  });

  test("wildcard handlers (sessionId=undefined) receive all sessions", () => {
    const router = new CdpEventRouter();
    const received: unknown[] = [];

    router.add(undefined, "Target.targetCreated", (params) => {
      received.push(params);
    });

    router.dispatch("session-1", "Target.targetCreated", { targetId: "1" });
    router.dispatch("session-2", "Target.targetCreated", { targetId: "2" });
    expect(received.length).toBe(2);
  });

  test("unsubscribe stops delivery", () => {
    const router = new CdpEventRouter();
    const received: unknown[] = [];

    const unsub = router.add("session-1", "Page.loadEventFired", () => {
      received.push("event");
    });

    unsub();
    router.dispatch("session-1", "Page.loadEventFired", {});
    expect(received.length).toBe(0);
  });

  test("clearSession removes only that session", () => {
    const router = new CdpEventRouter();
    const s1: unknown[] = [];
    const s2: unknown[] = [];

    router.add("session-1", "Page.loadEventFired", () => s1.push("s1"));
    router.add("session-2", "Page.loadEventFired", () => s2.push("s2"));

    router.clearSession("session-1");

    router.dispatch("session-1", "Page.loadEventFired", {});
    router.dispatch("session-2", "Page.loadEventFired", {});

    expect(s1.length).toBe(0);
    expect(s2.length).toBe(1);
  });

  test("clear removes all handlers", () => {
    const router = new CdpEventRouter();
    const received: unknown[] = [];

    router.add("session-1", "Page.loadEventFired", () => received.push("s1"));
    router.add("session-2", "Page.loadEventFired", () => received.push("s2"));
    router.add(undefined, "Target.targetCreated", () => received.push("browser"));

    router.clear();

    router.dispatch("session-1", "Page.loadEventFired", {});
    router.dispatch("session-2", "Page.loadEventFired", {});
    router.dispatch("session-3", "Target.targetCreated", {});

    expect(received.length).toBe(0);
  });

  test("one handler throwing does not break others", () => {
    const router = new CdpEventRouter();
    const ok: unknown[] = [];

    router.add("s", "test", () => { throw new Error("boom"); });
    router.add("s", "test", () => ok.push("still called"));

    router.dispatch("s", "test", {});
    expect(ok).toEqual(["still called"]);
  });
});
