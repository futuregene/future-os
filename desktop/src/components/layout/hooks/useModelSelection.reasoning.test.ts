// @vitest-environment jsdom
import type { AgentModelOption } from "../../../integrations/agent/agentClient";
import type { StoredThread } from "../../../integrations/storage/threadStore";
import { act } from "react";
import { expect, it, vi } from "vitest";
import { rememberLastUsedThinkingLevel, resolveInitialThinkingLevel } from "../../../integrations/agent/agentClient";
import { updateCachedAgentState } from "../../../integrations/agent/agentStateCache";
import { renderHook } from "../../../test/renderHook";
import { useModelSelection } from "./useModelSelection";

it("keeps a single-option model on its only level, and follows the list once it widens", () => {
  // A current agent lists the implicit `default` alone for a model with no
  // thinking controls, and the desktop defers to that list — `reasoning` has no
  // behaviour left.
  const model: AgentModelOption = {
    id: "deepseek-v4-pro",
    label: "Model",
    provider: "custom",
    thinkingLevels: ["default"],
    defaultThinkingLevel: "default",
  };
  const modelId = "custom/deepseek-v4-pro";
  let activeThread: StoredThread | null = null;
  rememberLastUsedThinkingLevel("high");
  expect(resolveInitialThinkingLevel(modelId, [model])).toBe("default");
  const h = renderHook(() => useModelSelection({
    activeThread,
    selectedModelId: modelId,
    setSelectedModelId: vi.fn(),
    modelOptions: [{ ...model }],
    visibleModelOptions: [{ ...model }],
    refreshStore: vi.fn().mockResolvedValue(undefined),
  }));
  try {
    expect(h.current.selectedThinkingLevel).toBe("default");
    expect(h.current.activeThinkingLevel).toBe("default");
    act(() => h.current.syncSelection(modelId, "high"));
    expect(h.current.selectedThinkingLevel).toBe("default");
    activeThread = { id: "reasoning-selection-test" } as StoredThread;
    act(() => updateCachedAgentState(activeThread!.id, { model: modelId, thinkingLevel: "high" }));
    h.rerender();
    // The agent's stored level is not one this model offers, so it is shown as
    // the model's own starting level (the agent clamps it there too).
    expect(h.current.activeThinkingLevel).toBe("default");
    model.thinkingLevels = ["default", "off", "low", "high"];
    h.rerender();
    expect(h.current.activeThinkingLevel).toBe("high");
  }
  finally {
    h.unmount();
    localStorage.clear();
  }
});
