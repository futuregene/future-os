// @vitest-environment jsdom
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../../test/renderHook";
import { useAgentConnection } from "./useAgentConnection";

const mocks = vi.hoisted(() => ({
  loadModels: vi.fn(),
  listProviders: vi.fn(),
}));

vi.mock("../../../integrations/agent/agentClient", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../../integrations/agent/agentClient")>();
  return { ...actual, loadAgentModelOptions: (...args: unknown[]) => mocks.loadModels(...args) };
});
vi.mock("../../../integrations/agent/providers", () => ({
  listAgentProviders: (...args: unknown[]) => mocks.listProviders(...args),
}));
// Drive the poll by hand so overlapping ticks are deterministic.
vi.mock("../../../lib/usePolling", () => ({ usePolling: () => {} }));

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason?: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, reject, resolve };
}

const model = (id: string) => ({ id, label: id, provider: "future" });

beforeEach(() => {
  mocks.loadModels.mockReset().mockResolvedValue([]);
  mocks.listProviders.mockReset().mockResolvedValue({ builtin: [], custom: [] });
});

describe("useAgentConnection race guards", () => {
  it("does not let a slow older tick overwrite a newer catalog", async () => {
    const slow = deferred<ReturnType<typeof model>[]>();
    const fast = deferred<ReturnType<typeof model>[]>();
    mocks.loadModels.mockReturnValueOnce(slow.promise).mockReturnValueOnce(fast.promise);

    const harness = renderHook(() => useAgentConnection([]));
    let first!: Promise<void>;
    act(() => {
      first = harness.current.refreshAgentModels();
    });
    let second!: Promise<void>;
    act(() => {
      second = harness.current.refreshAgentModels();
    });

    await act(async () => {
      fast.resolve([model("new")]);
      await second;
    });
    expect(harness.current.modelOptions.map(option => option.id)).toEqual(["new"]);
    expect(harness.current.agentConnection.status).toBe("connected");

    await act(async () => {
      slow.resolve([model("stale")]);
      await first;
    });
    // The older response resolved last but must be discarded by the generation
    // guard, leaving the newer catalog in place.
    expect(harness.current.modelOptions.map(option => option.id)).toEqual(["new"]);
    harness.unmount();
  });

  it("does not apply the empty-catalog readiness probe from a superseded tick", async () => {
    const providers = deferred<{ builtin: { hasApiKey: boolean }[]; custom: unknown[] }>();
    mocks.loadModels.mockReturnValueOnce(Promise.resolve([])).mockReturnValueOnce(Promise.resolve([model("m1")]));
    mocks.listProviders.mockReturnValueOnce(providers.promise);

    const harness = renderHook(() => useAgentConnection([]));
    let first!: Promise<void>;
    act(() => {
      first = harness.current.refreshAgentModels();
    });
    await act(async () => {
      await flushAsync();
    });
    let second!: Promise<void>;
    act(() => {
      second = harness.current.refreshAgentModels();
    });
    await act(async () => {
      await second;
    });
    expect(harness.current.agentConnection.readiness).toBe("ready");

    await act(async () => {
      providers.resolve({ builtin: [], custom: [] });
      await first;
    });
    // The superseded tick would have said "needs_login"; the guard drops it.
    expect(harness.current.agentConnection.readiness).toBe("ready");
    expect(harness.current.modelOptions.map(option => option.id)).toEqual(["m1"]);
    harness.unmount();
  });

  it("keeps the last good catalog when a newer tick fails", async () => {
    mocks.loadModels.mockResolvedValueOnce([model("m1")]);
    const harness = renderHook(() => useAgentConnection([]));
    await act(async () => {
      await harness.current.refreshAgentModels();
    });
    expect(harness.current.agentConnection.status).toBe("connected");

    mocks.loadModels.mockRejectedValueOnce(new Error("Unable to connect to Future Agent"));
    await act(async () => {
      await harness.current.refreshAgentModels();
    });
    expect(harness.current.agentConnection.status).toBe("disconnected");
    expect(harness.current.agentConnection.kind).toBe("agent_unavailable");
    expect(harness.current.modelOptions.map(option => option.id)).toEqual(["m1"]);
    harness.unmount();
  });

  it("classifies a model-data failure separately from an unreachable agent", async () => {
    mocks.loadModels.mockRejectedValueOnce(new Error("agent rejected the model list (list_models)"));
    const harness = renderHook(() => useAgentConnection([]));
    await act(async () => {
      await harness.current.refreshAgentModels();
    });
    expect(harness.current.agentConnection.kind).toBe("model_error");
    harness.unmount();
  });

  it("keeps the previous array identity when the catalog is unchanged", async () => {
    mocks.loadModels.mockResolvedValue([model("m1")]);
    const harness = renderHook(() => useAgentConnection([]));
    await act(async () => {
      await harness.current.refreshAgentModels();
    });
    const first = harness.current.modelOptions;
    await act(async () => {
      await harness.current.refreshAgentModels();
    });
    // Same array reference: a fresh identity every poll would defeat the
    // composer's memo.
    expect(harness.current.modelOptions).toBe(first);
    harness.unmount();
  });

  it("unhides and reconciles the selection against the visible catalog", async () => {
    mocks.loadModels.mockResolvedValue([model("m1"), model("m2")]);
    const harness = renderHook(() => useAgentConnection([]));
    await act(async () => {
      await harness.current.refreshAgentModels();
    });
    act(() => harness.current.setSelectedModelId("future/m2"));
    expect(harness.current.selectedModelId).toBe("future/m2");

    harness.unmount();

    // Hiding the selected model falls the selection back to the catalog default.
    const hidden = renderHook(() => useAgentConnection(["future/m1"]));
    await act(async () => {
      await hidden.current.refreshAgentModels();
    });
    act(() => hidden.current.setSelectedModelId("future/m1"));
    await act(async () => {
      await Promise.resolve();
    });
    expect(hidden.current.visibleModelOptions.map(option => option.id)).toEqual(["m2"]);
    expect(hidden.current.selectedModelId).toBe("future/m2");
    hidden.unmount();
  });
});
