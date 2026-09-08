// @vitest-environment jsdom
import type { StoredThread } from "../../../integrations/storage/threadStore";
import { act } from "react";
import { expect, it, vi } from "vitest";
import { rememberLastUsedThinkingLevel, resolveInitialThinkingLevel } from "../../../integrations/agent/agentClient";
import { updateCachedAgentState } from "../../../integrations/agent/agentStateCache";
import { renderHook } from "../../../test/renderHook";
import { useModelSelection } from "./useModelSelection";

it("keeps drafts and existing high-level sessions off while model thinking is disabled", () => {
  const model = { id: "deepseek-v4-pro", label: "Model", provider: "custom", reasoning: false, thinkingLevel: "high" };
  const modelId = "custom/deepseek-v4-pro";
  let activeThread: StoredThread | null = null;
  rememberLastUsedThinkingLevel("high");
  expect(resolveInitialThinkingLevel(modelId, [model])).toBe("off");
  const h = renderHook(() => useModelSelection({
    activeThread,
    selectedModelId: modelId,
    setSelectedModelId: vi.fn(),
    modelOptions: [{ ...model }],
    visibleModelOptions: [{ ...model }],
    refreshStore: vi.fn().mockResolvedValue(undefined),
  }));
  try {
    expect(h.current.selectedThinkingLevel).toBe("off");
    expect(h.current.activeThinkingLevel).toBe("off");
    act(() => h.current.syncSelection(modelId, "high"));
    expect(h.current.selectedThinkingLevel).toBe("off");
    activeThread = { id: "reasoning-selection-test" } as StoredThread;
    act(() => updateCachedAgentState(activeThread!.id, { model: modelId, thinkingLevel: "high" }));
    h.rerender();
    expect(h.current.activeThinkingLevel).toBe("off");
    model.reasoning = true;
    h.rerender();
    expect(h.current.activeThinkingLevel).toBe("high");
  }
  finally {
    h.unmount();
    localStorage.clear();
  }
});
