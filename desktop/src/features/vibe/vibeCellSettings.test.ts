import type { AgentModelOption } from "../../integrations/agent/agentClient";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { getCachedAgentState, updateCachedAgentState } from "../../integrations/agent/agentStateCache";
import { updateThreadModel, updateThreadThinkingLevel } from "../../integrations/storage/threads";
import { setVibeCellModel, setVibeCellThinkingLevel } from "./vibeCellSettings";

/**
 * Changing one cell's model, from the panel's menu.
 *
 * Two promises are being kept here, and both are easy to break silently. The write
 * goes **through the conversation's own id** — never through the shell's "active
 * thread", which in the panel is whichever conversation the main window happens to
 * have open — and the cached agent state is updated in the same call, because that
 * cache is what the cell renders from.
 */
vi.mock("../../integrations/storage/threads", () => ({
  updateThreadModel: vi.fn(async () => {}),
  updateThreadThinkingLevel: vi.fn(async () => {}),
}));

vi.mock("../../integrations/agent/agentStateCache", () => ({
  getCachedAgentState: vi.fn(() => null),
  updateCachedAgentState: vi.fn(),
  useCachedAgentState: vi.fn(() => null),
}));

const modelOptions: AgentModelOption[] = [
  { id: "model-fast", label: "Fast", thinkingLevels: ["off", "on"] },
  { id: "model-deep", label: "Deep", thinkingLevels: ["default", "low", "high"] },
] as unknown as AgentModelOption[];

beforeEach(() => {
  vi.mocked(updateThreadModel).mockClear();
  vi.mocked(updateThreadThinkingLevel).mockClear();
  vi.mocked(updateCachedAgentState).mockClear();
});

describe("setVibeCellModel", () => {
  it("writes the model, then the level that model defaults to, then the cache", async () => {
    await setVibeCellModel("thread-1", "model-deep", modelOptions);

    // Order matters: the level is written *after* the model, because the level is a
    // property of the model that is now in force.
    const modelCall = vi.mocked(updateThreadModel).mock.invocationCallOrder[0]!;
    const levelCall = vi.mocked(updateThreadThinkingLevel).mock.invocationCallOrder[0]!;
    const cacheCall = vi.mocked(updateCachedAgentState).mock.invocationCallOrder[0]!;
    expect(modelCall).toBeLessThan(levelCall);
    expect(levelCall).toBeLessThan(cacheCall);

    expect(updateThreadModel).toHaveBeenCalledWith({ modelId: "model-deep", threadId: "thread-1" });
    expect(updateThreadThinkingLevel).toHaveBeenCalledWith({ threadId: "thread-1", thinkingLevel: "default" });
    expect(updateCachedAgentState).toHaveBeenCalledWith("thread-1", { model: "model-deep", thinkingLevel: "default" });
  });

  it("updates the cache only after both writes have landed", async () => {
    // The cache is what the cell renders from, so it may not claim a change the store
    // has not accepted: a rejected level write must leave the cell showing the old one.
    vi.mocked(updateThreadThinkingLevel).mockRejectedValueOnce(new Error("agent offline"));

    await expect(setVibeCellModel("thread-2", "model-deep", modelOptions)).rejects.toThrow("agent offline");
    expect(updateCachedAgentState).not.toHaveBeenCalled();
  });
});

describe("setVibeCellThinkingLevel", () => {
  it("writes one conversation's level and mirrors it into the cache", async () => {
    await setVibeCellThinkingLevel("thread-3", "high");

    expect(updateThreadThinkingLevel).toHaveBeenCalledWith({ threadId: "thread-3", thinkingLevel: "high" });
    expect(updateCachedAgentState).toHaveBeenCalledWith("thread-3", { thinkingLevel: "high" });
    // Nothing else about the conversation is touched.
    expect(updateThreadModel).not.toHaveBeenCalled();
    expect(getCachedAgentState).not.toHaveBeenCalled();
  });
});
