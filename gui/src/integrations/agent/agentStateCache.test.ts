import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeCommand = vi.fn();
const { agentEventHandlers } = vi.hoisted(() => ({
  agentEventHandlers: [] as Array<(event: { payload: Record<string, unknown> }) => void>,
}));

vi.mock("../tauri/invoke", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
  listen: vi.fn(async (_name: string, handler: (event: { payload: Record<string, unknown> }) => void) => {
    agentEventHandlers.push(handler);
    return () => {};
  }),
}));

describe("agentStateCache", () => {
  beforeEach(() => {
    invokeCommand.mockReset();
    vi.resetModules();
  });

  it("deduplicates concurrent loads for one thread", async () => {
    invokeCommand.mockResolvedValue({ model: "future/m1", thinkingLevel: "low" });
    const { getAgentState } = await import("./agentStateCache");

    const [first, second] = await Promise.all([
      getAgentState("thread-dedup"),
      getAgentState("thread-dedup"),
    ]);

    expect(invokeCommand).toHaveBeenCalledTimes(1);
    expect(first).toEqual(second);
  });

  it("keeps serving stale entries while revalidating (no silent snapshot drop)", async () => {
    vi.useFakeTimers();
    try {
      invokeCommand.mockResolvedValue({ model: "future/m1", thinkingLevel: "low" });
      const { getAgentState, getCachedAgentState } = await import("./agentStateCache");

      await getAgentState("thread-swr");
      expect(getCachedAgentState("thread-swr")).toMatchObject({ model: "future/m1" });

      // Past the TTL: the sync read must STILL return the last-known state —
      // dropping it made the composer fall back to the global draft model.
      vi.setSystemTime(Date.now() + 60_000);
      expect(getCachedAgentState("thread-swr")).toMatchObject({ model: "future/m1" });

      // ...while an awaited fetch still revalidates against the agent.
      invokeCommand.mockResolvedValue({ model: "future/m2", thinkingLevel: "high" });
      await getAgentState("thread-swr");
      expect(getCachedAgentState("thread-swr")).toMatchObject({ model: "future/m2" });
    }
    finally {
      vi.useRealTimers();
    }
  });

  it("does not let a stale load overwrite an optimistic update", async () => {
    let resolveLoad: ((value: Record<string, unknown>) => void) | undefined;
    invokeCommand.mockReturnValue(new Promise((resolve) => {
      resolveLoad = resolve;
    }));
    const {
      getAgentState,
      getCachedAgentState,
      updateCachedAgentState,
    } = await import("./agentStateCache");

    const pending = getAgentState("thread-race");
    updateCachedAgentState("thread-race", { model: "future/new" });
    resolveLoad?.({ model: "future/old", thinkingLevel: "high" });

    await expect(pending).resolves.toMatchObject({ model: "future/new" });
    expect(getCachedAgentState("thread-race")).toMatchObject({ model: "future/new" });
  });

  it("revalidateAgentState bypasses the TTL throttle", async () => {
    invokeCommand.mockResolvedValue({ model: "future/m1", sessionId: "sess_1" });
    const { getAgentState, getCachedAgentState, revalidateAgentState } = await import("./agentStateCache");

    await getAgentState("thread-force");
    expect(invokeCommand).toHaveBeenCalledTimes(1);

    // The entry is fresh (well inside the TTL): a plain prefetch would
    // short-circuit, but an agent restart within that window must still
    // revalidate — that's the gap force semantics close.
    invokeCommand.mockResolvedValue({ model: "future/m2", sessionId: "sess_2" });
    revalidateAgentState("thread-force");

    await vi.waitFor(() => {
      expect(invokeCommand).toHaveBeenCalledTimes(2);
      expect(getCachedAgentState("thread-force")).toMatchObject({ model: "future/m2" });
    });
  });

  it("config_reloaded drops the stale entry and revalidates instead of re-inserting it", async () => {
    invokeCommand.mockResolvedValue({ model: "future/m1", thinkingLevel: "low", sessionId: "sess_1" });
    const { getAgentState, getCachedAgentState, installAgentEventListener } = await import("./agentStateCache");

    await getAgentState("thread-cfg");
    expect(getCachedAgentState("thread-cfg")).toMatchObject({ model: "future/m1" });

    installAgentEventListener();
    const handler = agentEventHandlers[agentEventHandlers.length - 1];
    expect(handler).toBeDefined();

    invokeCommand.mockResolvedValue({ model: "future/m2", thinkingLevel: "high", sessionId: "sess_1" });
    handler?.({ payload: { _eventType: "config_reloaded", sessionId: "sess_1" } });

    // The pre-reload snapshot must be gone immediately — it used to be
    // re-inserted with a fresh fetchedAt and linger indefinitely.
    expect(getCachedAgentState("thread-cfg")).toBeUndefined();

    // ...and the cache revalidates against the agent right away.
    await vi.waitFor(() => {
      expect(getCachedAgentState("thread-cfg")).toMatchObject({ model: "future/m2" });
    });
  });
});
