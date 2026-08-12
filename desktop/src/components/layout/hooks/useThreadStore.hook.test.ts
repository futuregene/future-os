import type { StoredThread, StoredWorkspace } from "../../../integrations/storage/threadStore";
import { act } from "react";
// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../../test/renderHook";

import { useThreadStore } from "./useThreadStore";

const listThreads = vi.fn();
const listWorkspaces = vi.fn();
const initializeAppStore = vi.fn();
const getRecentOrCreateDefaultThread = vi.fn();
const listLatestRunInfos = vi.fn();
const listStreamingThreadIds = vi.fn();
const prefetchAgentState = vi.fn();

vi.mock("../../../integrations/storage/threadStore", () => ({
  listThreads: (...args: unknown[]) => listThreads(...args),
  listWorkspaces: (...args: unknown[]) => listWorkspaces(...args),
  initializeAppStore: (...args: unknown[]) => initializeAppStore(...args),
  getRecentOrCreateDefaultThread: (...args: unknown[]) => getRecentOrCreateDefaultThread(...args),
  listLatestRunInfos: (...args: unknown[]) => listLatestRunInfos(...args),
}));

vi.mock("../../../integrations/agent/agentStateCache", () => ({
  listStreamingThreadIds: (...args: unknown[]) => listStreamingThreadIds(...args),
  prefetchAgentState: (...args: unknown[]) => prefetchAgentState(...args),
}));

type Listener = (event: { payload: Record<string, unknown> }) => void;
const listeners = new Map<string, Listener>();

vi.mock("@tauri-apps/api/event", () => ({
  listen: (name: string, handler: Listener) => {
    listeners.set(name, handler);
    return Promise.resolve(() => {});
  },
}));

function thread(id: string, workspaceId = "w1", status = "active") {
  return { id, workspaceId, status } as unknown as StoredThread;
}

const workspace: StoredWorkspace = { id: "w1", kind: "user" } as unknown as StoredWorkspace;

beforeEach(() => {
  for (const mock of [listThreads, listWorkspaces, initializeAppStore, getRecentOrCreateDefaultThread, listLatestRunInfos, listStreamingThreadIds, prefetchAgentState]) {
    mock.mockReset();
  }
  listeners.clear();
  initializeAppStore.mockResolvedValue(undefined);
  listThreads.mockResolvedValue([thread("t1"), thread("t2")]);
  listWorkspaces.mockResolvedValue([workspace]);
  getRecentOrCreateDefaultThread.mockResolvedValue(thread("t1"));
  listLatestRunInfos.mockResolvedValue([]);
  listStreamingThreadIds.mockResolvedValue([]);
});

async function mountStore() {
  const h = renderHook(() => useThreadStore());
  await flushAsync();
  await flushAsync();
  return h;
}

describe("useThreadStore", () => {
  it("bootstraps the store with the recent thread active", async () => {
    const h = await mountStore();
    expect(initializeAppStore).toHaveBeenCalled();
    expect(h.current.loadingStore).toBe(false);
    expect(h.current.storeError).toBeNull();
    expect(h.current.threads.map(t => t.id)).toEqual(["t1", "t2"]);
    expect(h.current.activeThreadId).toBe("t1");
    expect(h.current.activeThread?.id).toBe("t1");
    expect(h.current.activeWorkspace?.id).toBe("w1");
    expect(h.current.activeThreads.map(t => t.id)).toEqual(["t1", "t2"]);
    // Active thread prefetches agent state.
    expect(prefetchAgentState).toHaveBeenCalledWith("t1");
    h.unmount();
  });

  it("surfaces a bootstrap failure as storeError", async () => {
    initializeAppStore.mockRejectedValue(new Error("db corrupt"));
    const h = await mountStore();
    expect(h.current.storeError).toBe("db corrupt");
    expect(h.current.loadingStore).toBe(false);
    h.unmount();
  });

  it("refreshStore prefers the requested id, then the current, then the first", async () => {
    const h = await mountStore();
    // Prefer the requested id.
    await act(async () => {
      await h.current.refreshStore("t2");
    });
    expect(h.current.activeThreadId).toBe("t2");
    // An unknown requested id keeps the current selection.
    await act(async () => {
      await h.current.refreshStore("nope");
    });
    expect(h.current.activeThreadId).toBe("t2");
    // When the current thread vanishes, fall back to the first selectable.
    listThreads.mockResolvedValue([thread("t9")]);
    await act(async () => {
      await h.current.refreshStore();
    });
    expect(h.current.activeThreadId).toBe("t9");
    h.unmount();
  });

  it("refreshStore with no selectable threads clears the active id", async () => {
    const h = await mountStore();
    listThreads.mockResolvedValue([thread("t1", "w1", "archived")]);
    await act(async () => {
      await h.current.refreshStore();
    });
    expect(h.current.activeThreadId).toBeNull();
    expect(h.current.activeThread).toBeNull();
    h.unmount();
  });

  it("drops a stale refreshStore response when a newer one is in flight", async () => {
    const h = await mountStore();
    let resolveSlow!: (value: unknown) => void;
    listThreads.mockImplementationOnce(() => new Promise((resolve) => {
      resolveSlow = resolve;
    }));
    listWorkspaces.mockResolvedValue([workspace]);
    const slow = h.current.refreshStore("t2");
    // A newer refresh supersedes the slow one.
    listThreads.mockResolvedValue([thread("t1")]);
    await act(async () => {
      await h.current.refreshStore("t1");
    });
    expect(h.current.activeThreadId).toBe("t1");
    // Now resolve the stale one — it must not overwrite the newer state.
    await act(async () => {
      resolveSlow([thread("t2")]);
      await slow;
    });
    expect(h.current.activeThreadId).toBe("t1");
    h.unmount();
  });

  it("reduces thread-runtime-updated pushes into run statuses", async () => {
    const h = await mountStore();
    act(() => {
      listeners.get("thread-runtime-updated")?.({
        payload: { threadId: "t1", runId: "r1", revision: 5, status: "running", resetProjection: false },
      });
    });
    expect(h.current.threadRunStatuses.t1).toMatchObject({ runId: "r1", status: "running" });
    h.unmount();
  });

  it("applies the initial streaming snapshot and later pushes", async () => {
    listStreamingThreadIds.mockResolvedValue(["t1"]);
    const h = await mountStore();
    await flushAsync();
    expect(h.current.threadStreamingStatuses).toEqual({ t1: true });

    // A push with the streaming set changes it; a stale revision is ignored;
    // an identical set keeps object identity.
    act(() => {
      listeners.get("thread-streaming-updated")?.({ payload: { revision: 2, threadIds: ["t1", "t2"] } });
    });
    expect(h.current.threadStreamingStatuses).toEqual({ t1: true, t2: true });
    const same = h.current.threadStreamingStatuses;
    act(() => {
      listeners.get("thread-streaming-updated")?.({ payload: { revision: 1, threadIds: ["t9"] } });
    });
    expect(h.current.threadStreamingStatuses).toEqual({ t1: true, t2: true });
    act(() => {
      listeners.get("thread-streaming-updated")?.({ payload: { revision: 3, threadIds: ["t1", "t2"] } });
    });
    expect(h.current.threadStreamingStatuses).toBe(same);
    h.unmount();
  });

  it("discards the streaming snapshot when a push beat it", async () => {
    let resolveSnapshot!: (ids: string[]) => void;
    listStreamingThreadIds.mockImplementation(
      () => new Promise<string[]>((resolve) => {
        resolveSnapshot = resolve;
      }),
    );
    const h = renderHook(() => useThreadStore());
    await flushAsync();
    await flushAsync();
    act(() => {
      listeners.get("thread-streaming-updated")?.({ payload: { revision: 1, threadIds: ["t2"] } });
    });
    await act(async () => {
      resolveSnapshot(["t1"]);
      await Promise.resolve();
    });
    await flushAsync();
    expect(h.current.threadStreamingStatuses).toEqual({ t2: true });
    h.unmount();
  });

  it("reconciles run statuses on the polling tick, keeping entries on failure", async () => {
    vi.useFakeTimers();
    const h = renderHook(() => useThreadStore());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    // First tick: infos for t1 only.
    listLatestRunInfos.mockResolvedValue([
      { threadId: "t1", runId: "r1", status: "running", endedAt: null },
    ]);
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(h.current.threadRunStatuses.t1).toMatchObject({ runId: "r1", status: "running" });

    // A failing batch keeps the previous statuses.
    listLatestRunInfos.mockRejectedValue(new Error("ipc"));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(h.current.threadRunStatuses.t1).toMatchObject({ runId: "r1" });

    // Unchanged snapshot keeps object identity (no re-render).
    listLatestRunInfos.mockResolvedValue([
      { threadId: "t1", runId: "r1", status: "running", endedAt: null },
    ]);
    const before = h.current.threadRunStatuses;
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(h.current.threadRunStatuses).toBe(before);
    h.unmount();
    vi.useRealTimers();
  });

  it("drops a stale reconciliation response when a push invalidated it", async () => {
    vi.useFakeTimers();
    const h = renderHook(() => useThreadStore());
    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    let resolveInfos!: (value: unknown) => void;
    listLatestRunInfos.mockImplementation(() => new Promise((resolve) => {
      resolveInfos = resolve;
    }));
    // Kick a reconciliation tick (timer fires, fetch hangs).
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    // A push arrives while the fetch is in flight.
    act(() => {
      listeners.get("thread-runtime-updated")?.({
        payload: { threadId: "t1", runId: "r9", revision: 9, status: "running", resetProjection: false },
      });
    });
    expect(h.current.threadRunStatuses.t1).toMatchObject({ runId: "r9" });
    // The stale fetch resolves — its snapshot must not revert the push.
    await act(async () => {
      resolveInfos([{ threadId: "t1", runId: "r1", status: "completed", endedAt: 1 }]);
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(h.current.threadRunStatuses.t1).toMatchObject({ runId: "r9" });
    h.unmount();
    vi.useRealTimers();
  });

  it("clears statuses when no active threads remain", async () => {
    const h = await mountStore();
    act(() => {
      listeners.get("thread-runtime-updated")?.({
        payload: { threadId: "t1", runId: "r1", revision: 5, status: "running", resetProjection: false },
      });
    });
    expect(h.current.threadRunStatuses.t1).toBeDefined();
    listThreads.mockResolvedValue([thread("t1", "w1", "archived")]);
    await act(async () => {
      await h.current.refreshStore();
    });
    expect(h.current.threadRunStatuses).toEqual({});
    expect(h.current.threadStreamingStatuses).toEqual({});
    h.unmount();
  });

  it("activeWorkspace falls back to the user-kind workspace", async () => {
    const h = await mountStore();
    // Point the active thread at an unknown workspace: falls back to kind user.
    listThreads.mockResolvedValue([thread("t1", "unknown-ws")]);
    listWorkspaces.mockResolvedValue([workspace]);
    await act(async () => {
      await h.current.refreshStore("t1");
    });
    expect(h.current.activeWorkspace?.id).toBe("w1");
    h.unmount();
  });

  it("ignores a bootstrap that resolves after unmount", async () => {
    let resolveThreads!: (value: unknown) => void;
    listThreads.mockImplementation(() => new Promise((resolve) => {
      resolveThreads = resolve;
    }));
    const h = renderHook(() => useThreadStore());
    // Let the bootstrap reach the hanging listThreads call.
    await flushAsync();
    await flushAsync();
    h.unmount();
    await act(async () => {
      resolveThreads([thread("t1")]);
      await Promise.resolve();
    });
    // No crash / no post-unmount setState.
  });
});
