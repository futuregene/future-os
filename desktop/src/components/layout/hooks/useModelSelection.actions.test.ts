// @vitest-environment jsdom
import type { AgentModelOption } from "../../../integrations/agent/agentClient";
import type { StoredThread } from "../../../integrations/storage/threadStore";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { renderHook } from "../../../test/renderHook";
import { useModelSelection } from "./useModelSelection";

const mocks = vi.hoisted(() => ({
  updateCachedAgentState: vi.fn(),
  updateThreadModel: vi.fn(),
  updateThreadThinkingLevel: vi.fn(),
}));

vi.mock("../../../integrations/agent/agentStateCache", () => ({
  updateCachedAgentState: (...args: unknown[]) => mocks.updateCachedAgentState(...args),
  useCachedAgentState: () => undefined,
}));
vi.mock("../../../integrations/storage/threadStore", () => ({
  updateThreadModel: (...args: unknown[]) => mocks.updateThreadModel(...args),
  updateThreadThinkingLevel: (...args: unknown[]) => mocks.updateThreadThinkingLevel(...args),
}));

// A current agent's catalogue: the model's effective list always starts with
// the implicit `default`, and a model without thinking controls lists it alone.
const SMART: AgentModelOption = {
  id: "smart",
  label: "Smart",
  provider: "future",
  thinkingLevels: ["default", "off", "low", "medium", "high", "xhigh"],
  defaultThinkingLevel: "default",
};
const PLAIN: AgentModelOption = {
  id: "plain",
  label: "Plain",
  provider: "future",
  thinkingLevels: ["default"],
  defaultThinkingLevel: "default",
};
const MODELS = [SMART, PLAIN];

function thread(id = "t1"): StoredThread {
  return {
    id,
    agentSessionId: id,
    title: id,
    mode: "chat",
    workspaceId: "w1",
    status: "active",
    pinned: false,
    readonly: false,
    createdAt: 0,
    updatedAt: 0,
  };
}

function captureToasts() {
  const toasts: { message: string; tone?: string }[] = [];
  const handler = (event: Event) => toasts.push((event as CustomEvent<{ message: string; tone?: string }>).detail);
  window.addEventListener("futureos:toast", handler);
  return { stop: () => window.removeEventListener("futureos:toast", handler), toasts };
}

function mount(activeThread: StoredThread | null, selectedModelId = "future/smart") {
  const setSelectedModelId = vi.fn();
  const refreshStore = vi.fn().mockResolvedValue(undefined);
  const harness = renderHook(() => useModelSelection({
    activeThread,
    selectedModelId,
    setSelectedModelId,
    modelOptions: MODELS,
    visibleModelOptions: MODELS,
    refreshStore,
  }));
  return { harness, refreshStore, setSelectedModelId };
}

beforeEach(() => {
  localStorage.clear();
  mocks.updateCachedAgentState.mockReset();
  mocks.updateThreadModel.mockReset().mockResolvedValue(undefined);
  mocks.updateThreadThinkingLevel.mockReset().mockResolvedValue(undefined);
});

describe("useModelSelection persistence", () => {
  it("persists a model change with its default thinking level and refreshes the thread", async () => {
    const { harness, refreshStore, setSelectedModelId } = mount(thread());
    await act(async () => {
      await harness.current.changeModel("future/plain");
    });

    expect(setSelectedModelId).toHaveBeenCalledWith("future/plain");
    expect(mocks.updateThreadModel).toHaveBeenCalledWith({ modelId: "future/plain", threadId: "t1" });
    // A single-level model pins the thread to its only level.
    expect(mocks.updateThreadThinkingLevel).toHaveBeenCalledWith({ thinkingLevel: "default", threadId: "t1" });
    expect(mocks.updateCachedAgentState).toHaveBeenCalledWith("t1", { model: "future/plain", thinkingLevel: "default" });
    expect(refreshStore).toHaveBeenCalledWith("t1");
    harness.unmount();
  });

  it("only updates the draft when no thread is active", async () => {
    const { harness, refreshStore, setSelectedModelId } = mount(null);
    await act(async () => {
      await harness.current.changeModel("future/plain");
    });
    expect(setSelectedModelId).toHaveBeenCalledWith("future/plain");
    expect(mocks.updateThreadModel).not.toHaveBeenCalled();
    expect(refreshStore).not.toHaveBeenCalled();
    expect(localStorage.getItem("futureos:last-used-model")).toBe("future/plain");
    harness.unmount();
  });

  it("toasts and reloads the store when a model write fails", async () => {
    mocks.updateThreadModel.mockRejectedValue(new Error("db busy"));
    const toasts = captureToasts();
    const { harness, refreshStore } = mount(thread());
    await act(async () => {
      await harness.current.changeModel("future/plain");
    });
    expect(toasts.toasts[0]!.message).toContain("db busy");
    expect(toasts.toasts[0]!.tone).toBe("error");
    expect(refreshStore).toHaveBeenCalledWith("t1");
    toasts.stop();
    harness.unmount();
  });

  it("persists an explicit thinking-level change on the active thread", async () => {
    const { harness, refreshStore } = mount(thread());
    await act(async () => {
      await harness.current.changeThinkingLevel("high");
    });
    expect(mocks.updateCachedAgentState).toHaveBeenCalledWith("t1", { thinkingLevel: "high" });
    expect(mocks.updateThreadThinkingLevel).toHaveBeenCalledWith({ thinkingLevel: "high", threadId: "t1" });
    expect(refreshStore).toHaveBeenCalledWith("t1");
    harness.unmount();
  });

  it("keeps a thinking-level change local for a draft, and any non-empty token is a level", async () => {
    const { harness, refreshStore } = mount(null);
    await act(async () => {
      await harness.current.changeDraftThinkingLevel("high");
    });
    expect(harness.current.selectedThinkingLevel).toBe("high");
    expect(localStorage.getItem("futureos:last-used-thinking-level")).toBe("high");
    expect(refreshStore).not.toHaveBeenCalled();

    await act(async () => {
      await harness.current.changeThinkingLevel("nonsense");
    });
    // Arbitrary tokens are legal levels now; only emptiness falls back to the
    // implicit default (whether the chosen model offers it is a separate rule).
    expect(localStorage.getItem("futureos:last-used-thinking-level")).toBe("nonsense");
    expect(mocks.updateThreadThinkingLevel).not.toHaveBeenCalled();
    harness.unmount();
  });

  it("toasts and reloads when a thinking-level write fails", async () => {
    mocks.updateThreadThinkingLevel.mockRejectedValue(new Error("locked"));
    const toasts = captureToasts();
    const { harness, refreshStore } = mount(thread());
    await act(async () => {
      await harness.current.changeThinkingLevel("xhigh");
    });
    expect(toasts.toasts[0]!.message).toContain("locked");
    expect(refreshStore).toHaveBeenCalledWith("t1");
    toasts.stop();
    harness.unmount();
  });

  it("switches the draft model and follows that model's starting level", () => {
    const { harness } = mount(null);
    act(() => harness.current.changeDraftModel("future/plain"));
    expect(harness.current.selectedThinkingLevel).toBe("default");
    act(() => harness.current.changeDraftModel("future/smart"));
    expect(harness.current.selectedThinkingLevel).toBe("default");
    harness.unmount();
  });

  it("primes the selection for a just-created thread", () => {
    const { harness, setSelectedModelId } = mount(null);
    act(() => harness.current.syncSelection("future/plain", "high"));
    expect(setSelectedModelId).toHaveBeenCalledWith("future/plain");
    // The raw level is primed as given (the harness's model id prop does not
    // change); the display then resolves it against whichever model is active.
    expect(harness.current.selectedThinkingLevel).toBe("high");
    harness.unmount();
  });
});

describe("useModelSelection derived state", () => {
  it("explains an empty picker as no models vs all disabled", () => {
    const none = renderHook(() => useModelSelection({
      activeThread: null,
      selectedModelId: "future/smart",
      setSelectedModelId: vi.fn(),
      modelOptions: [],
      visibleModelOptions: [],
      refreshStore: vi.fn().mockResolvedValue(undefined),
    }));
    expect(none.current.modelsEmptyReason).toBe("no_models");
    none.unmount();

    const allDisabled = renderHook(() => useModelSelection({
      activeThread: null,
      selectedModelId: "future/smart",
      setSelectedModelId: vi.fn(),
      modelOptions: MODELS,
      visibleModelOptions: [],
      refreshStore: vi.fn().mockResolvedValue(undefined),
    }));
    expect(allDisabled.current.modelsEmptyReason).toBe("all_disabled");
    allDisabled.unmount();

    const some = mount(null);
    expect(some.harness.current.modelsEmptyReason).toBeUndefined();
    some.harness.unmount();
  });

  it("keeps a single-option model on its only level", () => {
    const { harness } = mount(null, "future/plain");
    expect(harness.current.activeThinkingLevel).toBe("default");
    expect(harness.current.selectedThinkingLevel).toBe("default");
    harness.unmount();
  });

  it("falls back to the catalog default for an unknown thread model", () => {
    const { harness } = mount(thread(), "future/does-not-exist");
    expect(harness.current.activeThreadModelId).toBe("future/smart");
    harness.unmount();
  });
});
