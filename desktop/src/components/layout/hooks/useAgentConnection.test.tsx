// @vitest-environment jsdom
import type { AgentModelOption } from "../../../integrations/agent/agentClient";
import type { ProvidersView } from "../../../integrations/agent/providers";
import { act } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { emitFutureEvent } from "../../../lib/futureEvents";
import { flushAsync, renderHook } from "../../../test/renderHook";
import { useAgentConnection } from "./useAgentConnection";

const mocks = vi.hoisted(() => ({
  loadModels: vi.fn(),
  listProviders: vi.fn<() => Promise<ProvidersView>>(),
}));

vi.mock("../../../integrations/agent/agentClient", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../integrations/agent/agentClient")>();
  return { ...actual, loadAgentModelOptions: mocks.loadModels };
});

vi.mock("../../../integrations/agent/providers", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../integrations/agent/providers")>();
  return { ...actual, listAgentProviders: mocks.listProviders };
});

function model(id: string, provider = "future"): AgentModelOption {
  return { id, label: id, provider };
}

const CATALOG = [model("deepseek-v4-pro"), model("claude", "anthropic")];

function providers(builtinKeys: boolean, customCount = 0): ProvidersView {
  return {
    builtin: [{ id: "future", name: "FutureOS", baseUrl: "https://x", hasApiKey: builtinKeys, modelCount: 1, requiresBaseUrl: false }],
    custom: Array.from({ length: customCount }, (_, index) => ({
      id: `c${index}`,
      name: `c${index}`,
      api: "openai",
      baseUrl: "https://y",
      hasApiKey: true,
      models: [],
    })),
  };
}

describe("useAgentConnection", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    mocks.loadModels.mockReset().mockResolvedValue(CATALOG);
    mocks.listProviders.mockReset().mockResolvedValue(providers(true));
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("starts checking, then reports connected with a resolved default model", async () => {
    const hook = renderHook(() => useAgentConnection([]));
    expect(hook.current.agentConnection.status).toBe("checking");

    await flushAsync();

    expect(hook.current.agentConnection.status).toBe("connected");
    expect(hook.current.agentConnection.readiness).toBe("ready");
    expect(hook.current.agentConnection.error).toBeNull();
    expect(hook.current.modelOptions).toEqual(CATALOG);
    hook.unmount();
  });

  it("classifies an unreachable agent and keeps the last good catalog", async () => {
    const hook = renderHook(() => useAgentConnection([]));
    await flushAsync();
    const goodCatalog = hook.current.modelOptions;

    mocks.loadModels.mockRejectedValue(new Error("Unable to connect to Future Agent"));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });

    expect(hook.current.agentConnection.status).toBe("disconnected");
    expect(hook.current.agentConnection.kind).toBe("agent_unavailable");
    // Blanking the catalog made every model id unresolvable and reset the
    // composer's model — the last good list must survive a failed poll.
    expect(hook.current.modelOptions).toBe(goodCatalog);
    hook.unmount();
  });

  it("classifies model-data failures separately from an unreachable agent", async () => {
    const cases: Array<[string, string]> = [
      ["Unable to load Future Agent models", "agent_unavailable"],
      ["agent returned invalid model data", "model_error"],
      ["list_models rejected the model list", "model_error"],
      ["something else entirely", "unknown"],
    ];

    for (const [message, kind] of cases) {
      mocks.loadModels.mockRejectedValue(new Error(message));
      const hook = renderHook(() => useAgentConnection([]));
      await flushAsync();
      expect(hook.current.agentConnection.kind, message).toBe(kind);
      expect(hook.current.agentConnection.error).toBe(message);
      hook.unmount();
    }
  });

  it("reports needs_login vs no_models when the catalog is empty", async () => {
    mocks.loadModels.mockResolvedValue([]);
    mocks.listProviders.mockResolvedValue(providers(false));
    const unconfigured = renderHook(() => useAgentConnection([]));
    await flushAsync();
    expect(unconfigured.current.agentConnection.readiness).toBe("needs_login");
    unconfigured.unmount();

    // Credentials exist (a custom provider counts) but expose no models.
    mocks.listProviders.mockResolvedValue(providers(false, 1));
    const empty = renderHook(() => useAgentConnection([]));
    await flushAsync();
    expect(empty.current.agentConnection.readiness).toBe("no_models");
    empty.unmount();

    // A failing provider probe degrades to the generic "no models" instead of
    // guessing "needs login".
    mocks.listProviders.mockRejectedValue(new Error("providers unreachable"));
    const unknown = renderHook(() => useAgentConnection([]));
    await flushAsync();
    expect(unknown.current.agentConnection.readiness).toBe("no_models");
    unknown.unmount();
  });

  it("reports all_disabled when every loaded model is hidden", async () => {
    const hook = renderHook(() => useAgentConnection(["future/deepseek-v4-pro", "anthropic/claude"]));
    await flushAsync();

    expect(hook.current.modelOptions).toHaveLength(2);
    expect(hook.current.visibleModelOptions).toEqual([]);
    expect(hook.current.agentConnection.readiness).toBe("all_disabled");
    hook.unmount();
  });

  it("filters hidden models out of the visible set and re-derives readiness immediately", async () => {
    const hidden: string[] = [];
    const ref = { current: hidden };
    const hook = renderHook(() => useAgentConnection(ref.current));
    await flushAsync();
    expect(hook.current.visibleModelOptions).toHaveLength(2);
    expect(hook.current.agentConnection.readiness).toBe("ready");

    ref.current = ["anthropic/claude"];
    hook.rerender();
    expect(hook.current.visibleModelOptions.map(entry => entry.id)).toEqual(["deepseek-v4-pro"]);
    expect(hook.current.agentConnection.readiness).toBe("ready");
    hook.unmount();
  });

  it("drops a selected model that leaves the visible set and falls back to the default", async () => {
    const hook = renderHook(() => useAgentConnection([]));
    await flushAsync();
    const initial = hook.current.selectedModelId;

    act(() => {
      hook.current.setSelectedModelId("anthropic/claude");
    });
    expect(hook.current.selectedModelId).toBe("anthropic/claude");

    // Hiding the selected model must move the picker to a usable default.
    const hidden = renderHook(() => useAgentConnection(["anthropic/claude"]));
    await flushAsync();
    act(() => {
      hidden.current.setSelectedModelId("anthropic/claude");
    });
    expect(hidden.current.selectedModelId).not.toBe("anthropic/claude");

    // An empty visible set clears the selection so pickers show their empty state.
    const allHidden = renderHook(() => useAgentConnection(["future/deepseek-v4-pro", "anthropic/claude"]));
    await flushAsync();
    expect(allHidden.current.visibleModelOptions).toEqual([]);
    expect(allHidden.current.selectedModelId).toBe("");

    expect(initial).toBeTruthy();
    hook.unmount();
    hidden.unmount();
    allHidden.unmount();
  });

  it("keeps the previous model array identity when the poll returns an equal catalog", async () => {
    const hook = renderHook(() => useAgentConnection([]));
    await flushAsync();
    const first = hook.current.modelOptions;
    const firstConnection = hook.current.agentConnection;

    // A fresh array with the same contents would flow AppShell → AgentThread →
    // Composer and defeat the composer's memo once per 10s poll.
    mocks.loadModels.mockResolvedValue(CATALOG.map(entry => ({ ...entry })));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });

    expect(hook.current.modelOptions).toBe(first);
    expect(hook.current.agentConnection).toBe(firstConnection);
    hook.unmount();
  });

  it("ignores a stale failure that lands after a newer tick already succeeded", async () => {
    let rejectSlow!: (reason?: unknown) => void;
    mocks.loadModels.mockReturnValueOnce(new Promise((_resolve, reject) => {
      rejectSlow = reject;
    }));

    const hook = renderHook(() => useAgentConnection([]));
    await flushAsync();

    // A second tick succeeds while the first is still in flight.
    mocks.loadModels.mockResolvedValue(CATALOG);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10_000);
    });
    expect(hook.current.agentConnection.status).toBe("connected");
    expect(hook.current.modelOptions).toEqual(CATALOG);

    // The abandoned slow call now fails — it must not flip the UI to
    // disconnected nor blank the freshly loaded catalog.
    await act(async () => {
      rejectSlow(new Error("Unable to connect to Future Agent"));
      await Promise.resolve();
    });

    expect(hook.current.agentConnection.status).toBe("connected");
    expect(hook.current.modelOptions).toEqual(CATALOG);
    hook.unmount();
  });

  it("never flaps through checking on later polls", async () => {
    const hook = renderHook(() => useAgentConnection([]));
    await flushAsync();
    const seen: string[] = [];
    for (let tick = 0; tick < 3; tick++) {
      mocks.loadModels.mockRejectedValue(new Error("Unable to connect to Future Agent"));
      await act(async () => {
        await vi.advanceTimersByTimeAsync(10_000);
      });
      seen.push(hook.current.agentConnection.status);
    }
    // The initial state owns "checking"; polls only report real results.
    expect(seen).toEqual(["disconnected", "disconnected", "disconnected"]);
    hook.unmount();
  });

  it("refreshes immediately on future-models-synced and providers-changed", async () => {
    const hook = renderHook(() => useAgentConnection([]));
    await flushAsync();
    expect(mocks.loadModels).toHaveBeenCalledTimes(1);

    await act(async () => {
      emitFutureEvent("future-models-synced", undefined);
      await Promise.resolve();
    });
    expect(mocks.loadModels).toHaveBeenCalledTimes(2);

    await act(async () => {
      emitFutureEvent("providers-changed", {
        revision: 1,
        providerId: "future",
        operation: "set_key",
        authChanged: true,
        modelsChanged: true,
      });
      await Promise.resolve();
    });
    expect(mocks.loadModels).toHaveBeenCalledTimes(3);
    hook.unmount();
  });

  it("exposes refreshAgentModels for a manual reload and stops polling on unmount", async () => {
    const hook = renderHook(() => useAgentConnection([]));
    await flushAsync();

    mocks.loadModels.mockResolvedValue([model("only")]);
    await act(async () => {
      await hook.current.refreshAgentModels();
    });
    expect(hook.current.modelOptions.map(entry => entry.id)).toEqual(["only"]);

    hook.unmount();
    const calls = mocks.loadModels.mock.calls.length;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(60_000);
    });
    expect(mocks.loadModels).toHaveBeenCalledTimes(calls);
  });
});
