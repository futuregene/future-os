import { beforeEach, describe, expect, it, vi } from "vitest";

const invokeCommand = vi.fn();

vi.mock("../tauri/invoke", () => ({
  invokeCommand: (...args: unknown[]) => invokeCommand(...args),
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
});
