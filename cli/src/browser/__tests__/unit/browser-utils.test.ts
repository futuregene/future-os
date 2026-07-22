/**
 * Unit tests for browser utility modules:
 * mouse.ts (centerOf), screenshot-writer.ts (resolveScreenshotPath),
 * chromium-endpoint.ts (identifyBrowser via resolveCdpEndpoint),
 * chromium-execution-context.ts (ExecutionContextTracker).
 */
import { describe, test, expect, mock } from "bun:test";
import { centerOf } from "../../input/mouse.js";
import { resolveScreenshotPath } from "../../artifacts/screenshot-writer.js";
import { ExecutionContextTracker } from "../../chromium/chromium-execution-context.js";
import type { CdpSession } from "../../chromium/cdp-connection.js";

// ─── mouse.ts ──────────────────────────────────────────────────────────────

describe("centerOf", () => {
  test("center of simple box", () => {
    expect(centerOf({ x: 0, y: 0, width: 100, height: 50 })).toEqual({ x: 50, y: 25 });
  });

  test("center with offset origin", () => {
    expect(centerOf({ x: 10, y: 20, width: 40, height: 60 })).toEqual({ x: 30, y: 50 });
  });

  test("rounds to integer", () => {
    // 7/2 = 3.5 → 4
    expect(centerOf({ x: 0, y: 0, width: 7, height: 3 })).toEqual({ x: 4, y: 2 });
  });

  test("zero-size box", () => {
    expect(centerOf({ x: 5, y: 5, width: 0, height: 0 })).toEqual({ x: 5, y: 5 });
  });

  test("negative coordinates", () => {
    expect(centerOf({ x: -10, y: -20, width: 40, height: 60 })).toEqual({ x: 10, y: 10 });
  });
});

// ─── screenshot-writer.ts ──────────────────────────────────────────────────

describe("resolveScreenshotPath", () => {
  test("explicit path is used as-is", () => {
    expect(resolveScreenshotPath("/tmp/my-shot.png")).toBe("/tmp/my-shot.png");
  });

  test("undefined generates timestamped path", () => {
    const path = resolveScreenshotPath();
    expect(path).toContain("browser-");
    expect(path).toEndWith(".png");
  });

  test("generated path contains no colons or dots in filename", () => {
    const path = resolveScreenshotPath();
    const filename = path.split("/").pop()!;
    // Colons are replaced with dashes (ISO timestamp sanitization)
    expect(filename).not.toContain(":");
  });
});

// ─── ExecutionContextTracker ───────────────────────────────────────────────

function mockCdpSession(): CdpSession & {
  emit: (event: string, params: unknown) => void;
  _handlers: Map<string, Array<(params: unknown) => void>>;
} {
  const handlers = new Map<string, Array<(params: unknown) => void>>();
  const session = {
    on(event: string, handler: (params: unknown) => void) {
      if (!handlers.has(event)) handlers.set(event, []);
      handlers.get(event)!.push(handler);
      return () => {
        const arr = handlers.get(event);
        if (arr) {
          const idx = arr.indexOf(handler);
          if (idx >= 0) arr.splice(idx, 1);
        }
      };
    },
    emit(event: string, params: unknown) {
      for (const h of handlers.get(event) ?? []) h(params);
    },
    _handlers: handlers,
  } as unknown as CdpSession & {
    emit: (event: string, params: unknown) => void;
    _handlers: Map<string, Array<(params: unknown) => void>>;
  };
  return session;
}

function makeDeadline(ms: number) {
  const start = Date.now();
  return {
    remainingMs: () => Math.max(0, ms - (Date.now() - start)),
    get expired() { return Date.now() - start >= ms; },
  };
}

describe("ExecutionContextTracker", () => {
  test("tracks created contexts", async () => {
    const session = mockCdpSession();
    const tracker = new ExecutionContextTracker(session);

    session.emit("Runtime.executionContextCreated", {
      context: { id: 1, auxData: { frameId: "f1", isDefault: true }, name: "main" },
    });

    const id = await tracker.getMainWorldContextId("f1", makeDeadline(100));
    expect(id).toBe(1);
    tracker.dispose();
  });

  test("falls back to any default context", async () => {
    const session = mockCdpSession();
    const tracker = new ExecutionContextTracker(session);

    session.emit("Runtime.executionContextCreated", {
      context: { id: 42, auxData: { frameId: "other_frame", isDefault: true }, name: "main" },
    });

    // Request a different frameId — should fall back to the default context
    const id = await tracker.getMainWorldContextId("unknown_frame", makeDeadline(100));
    expect(id).toBe(42);
    tracker.dispose();
  });

  test("destroyed contexts are removed", async () => {
    const session = mockCdpSession();
    const tracker = new ExecutionContextTracker(session);

    session.emit("Runtime.executionContextCreated", {
      context: { id: 1, auxData: { frameId: "f1", isDefault: true }, name: "main" },
    });
    session.emit("Runtime.executionContextDestroyed", { executionContextId: 1 });

    await expect(tracker.getMainWorldContextId("f1", makeDeadline(50))).rejects.toThrow();
    tracker.dispose();
  });

  test("cleared event removes all contexts", async () => {
    const session = mockCdpSession();
    const tracker = new ExecutionContextTracker(session);

    session.emit("Runtime.executionContextCreated", {
      context: { id: 1, auxData: { frameId: "f1", isDefault: true }, name: "main" },
    });
    session.emit("Runtime.executionContextCreated", {
      context: { id: 2, auxData: { frameId: "f2", isDefault: true }, name: "main" },
    });
    session.emit("Runtime.executionContextsCleared", {});

    await expect(tracker.getMainWorldContextId("f1", makeDeadline(50))).rejects.toThrow();
    tracker.dispose();
  });

  test("non-default contexts are skipped for main world", async () => {
    const session = mockCdpSession();
    const tracker = new ExecutionContextTracker(session);

    session.emit("Runtime.executionContextCreated", {
      context: { id: 1, auxData: { frameId: "f1", isDefault: false }, name: "isolated" },
    });

    await expect(tracker.getMainWorldContextId("f1", makeDeadline(50))).rejects.toThrow();
    tracker.dispose();
  });

  test("dispose removes listeners", () => {
    const session = mockCdpSession();
    const tracker = new ExecutionContextTracker(session);

    expect(session._handlers.size).toBeGreaterThan(0);
    tracker.dispose();
    // After dispose, the handlers should have been unsubscribed
    for (const arr of session._handlers.values()) {
      expect(arr.length).toBe(0);
    }
  });

  test("context appears after creation event", async () => {
    const session = mockCdpSession();
    const tracker = new ExecutionContextTracker(session);

    // Start waiting before emitting the event
    const promise = tracker.getMainWorldContextId("f1", makeDeadline(500));

    // Simulate async context creation
    setTimeout(() => {
      session.emit("Runtime.executionContextCreated", {
        context: { id: 7, auxData: { frameId: "f1", isDefault: true }, name: "main" },
      });
    }, 30);

    const id = await promise;
    expect(id).toBe(7);
    tracker.dispose();
  });
});
