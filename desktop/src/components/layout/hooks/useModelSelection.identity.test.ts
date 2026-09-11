// @vitest-environment jsdom
import { expect, it, vi } from "vitest";
import { renderHook } from "../../../test/renderHook";
import { useModelSelection } from "./useModelSelection";

it("qualifies a unique legacy selection before exposing it to the send pipeline", () => {
  const models = [{ id: "family/model", provider: "gateway", label: "Gateway" }];
  const hook = renderHook(() => useModelSelection({
    activeThread: null,
    selectedModelId: "family/model",
    setSelectedModelId: vi.fn(),
    modelOptions: models,
    visibleModelOptions: models,
    refreshStore: vi.fn().mockResolvedValue(undefined),
  }));
  try {
    expect(hook.current.activeThreadModelId).toBe("gateway/family/model");
  }
  finally {
    hook.unmount();
  }
});
