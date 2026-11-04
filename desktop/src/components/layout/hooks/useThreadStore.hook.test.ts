import type { StoredThread, StoredWorkspace } from "../../../integrations/storage/threadStore";
import { act } from "react";
// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../../test/renderHook";

import { RECENT_PAGE_SIZE } from "../recentOrder";
import { useThreadStore } from "./useThreadStore";

const listThreadsPage = vi.fn();
const listPinnedThreads = vi.fn();
const listActiveThreads = vi.fn();
const listWorkspacesPage = vi.fn();
const listPinnedWorkspaces = vi.fn();
const initializeAppStore = vi.fn();
const getRecentOrCreateDefaultThread = vi.fn();
const listLatestRunInfos = vi.fn();
const listStreamingThreadIds = vi.fn();
const prefetchAgentState = vi.fn();

vi.mock("../../../integrations/storage/threadStore", () => ({
  listThreadsPage: (...args: unknown[]) => listThreadsPage(...args),
  listPinnedThreads: (...args: unknown[]) => listPinnedThreads(...args),
  listActiveThreads: (...args: unknown[]) => listActiveThreads(...args),
  listWorkspacesPage: (...args: unknown[]) => listWorkspacesPage(...args),
  listPinnedWorkspaces: (...args: unknown[]) => listPinnedWorkspaces(...args),
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

function thread(id: string, workspaceId = "w1", status = "active", agentSessionId: string | null = `s-${id}`) {
  return { id, workspaceId, status, agentSessionId } as unknown as StoredThread;
}

const workspace: StoredWorkspace = { id: "w1", kind: "user" } as unknown as StoredWorkspace;

/** The chat scope's first page: what bootstrap asks for. */
function pageOf(
  threads: StoredThread[],
  nextCursor: string | null = null,
  totalRoots = threads.length,
) {
  return { threads, nextCursor, totalRoots };
}

function workspacesPageOf(workspaces: StoredWorkspace[], nextCursor: string | null = null) {
  return { workspaces, nextCursor };
}

beforeEach(() => {
  for (const mock of [
    listThreadsPage,
    listPinnedThreads,
    listActiveThreads,
    listWorkspacesPage,
    listPinnedWorkspaces,
    initializeAppStore,
    getRecentOrCreateDefaultThread,
    listLatestRunInfos,
    listStreamingThreadIds,
    prefetchAgentState,
  ]) {
    mock.mockReset();
  }
  listeners.clear();
  initializeAppStore.mockResolvedValue(undefined);
  listThreadsPage.mockResolvedValue(pageOf([thread("t1"), thread("t2")]));
  listPinnedThreads.mockResolvedValue([]);
  listActiveThreads.mockResolvedValue([]);
  listWorkspacesPage.mockResolvedValue(workspacesPageOf([workspace]));
  listPinnedWorkspaces.mockResolvedValue([]);
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
  it("bootstraps from the first pages, not the whole store", async () => {
    const h = await mountStore();
    expect(initializeAppStore).toHaveBeenCalled();
    expect(h.current.loadingStore).toBe(false);
    expect(h.current.storeError).toBeNull();
    // Only the chat scope's first page was asked for; the other scopes load on
    // demand (a workspace when it is expanded, archived when it is opened).
    expect(listThreadsPage).toHaveBeenCalledTimes(1);
    expect(listThreadsPage.mock.calls[0]![0]).toBe("chat");
    expect(h.current.threads.map(t => t.id).sort()).toEqual(["t1", "t2"]);
    expect(h.current.activeThreadId).toBe("t1");
    expect(h.current.activeThread?.id).toBe("t1");
    expect(h.current.activeWorkspace?.id).toBe("w1");
    expect(h.current.activeThreads.map(t => t.id).sort()).toEqual(["t1", "t2"]);
    // Active thread prefetches agent state.
    expect(prefetchAgentState).toHaveBeenCalledWith("t1");
    h.unmount();
  });

  it("asks for the pinned shortcut rows separately and whole", async () => {
    listPinnedThreads.mockResolvedValue([thread("pin")]);
    listPinnedWorkspaces.mockResolvedValue([{ ...workspace, pinned: true } as StoredWorkspace]);
    const h = await mountStore();
    expect(listPinnedThreads).toHaveBeenCalledWith("recent");
    expect(h.current.threads.map(t => t.id)).toContain("pin");
    expect(h.current.workspaces.map(w => w.id)).toContain("w1");
    h.unmount();
  });

  it("does not overwrite a fresh thread's draft selection with null agent state", async () => {
    const fresh = thread("fresh", "w1", "active", null);
    listThreadsPage.mockResolvedValue(pageOf([fresh]));
    getRecentOrCreateDefaultThread.mockResolvedValue(fresh);

    const h = await mountStore();

    expect(h.current.activeThread?.id).toBe("fresh");
    expect(prefetchAgentState).not.toHaveBeenCalled();
    h.unmount();
  });

  it("surfaces a bootstrap failure as storeError", async () => {
    initializeAppStore.mockRejectedValue(new Error("db corrupt"));
    const h = await mountStore();
    expect(h.current.storeError).toBe("db corrupt");
    expect(h.current.loadingStore).toBe(false);
    h.unmount();
  });

  it("stays silent when the bootstrap fails after the store was torn down", async () => {
    // A failure that arrives after unmount must not be written into the dead
    // hook's state (nor replace a newer mount's error with a stale one). The
    // bootstrap is parked mid-flight, the hook is unmounted, and only then does
    // the store reject.
    let failBootstrap!: (error: Error) => void;
    getRecentOrCreateDefaultThread.mockReturnValue(new Promise((_resolve, reject) => {
      failBootstrap = reject;
    }));

    const h = renderHook(() => useThreadStore());
    await act(async () => {
      await Promise.resolve();
    });
    expect(h.current.loadingStore).toBe(true);

    h.unmount();

    await act(async () => {
      failBootstrap(new Error("db corrupt"));
      await Promise.resolve();
      await Promise.resolve();
    });

    // The hook is gone; nothing threw and no state was written back into it.
    expect(h.current.storeError).toBeNull();
  });

  it("refreshStore prefers the requested id, then the current, then keeps it", async () => {
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
    h.unmount();
  });

  it("refreshStore drops a conversation its pages no longer return", async () => {
    const h = await mountStore();
    // The chat page now returns only t1 (t2 was archived elsewhere); the open
    // conversation is t2, which the refreshed rows do not hold — the store
    // keeps it addressable rather than selecting a stranger, but the archived
    // row itself leaves the visible set.
    h.current.setActiveThreadId("t1");
    listThreadsPage.mockResolvedValue(pageOf([thread("t1")]));
    await act(async () => {
      await h.current.refreshStore();
    });
    expect(h.current.threads.map(t => t.id)).toEqual(["t1"]);
    expect(h.current.activeThreadId).toBe("t1");
    h.unmount();
  });

  /**
   * A row a page stops returning is not the same evidence as a deleted row.
   *
   * Archiving leaves the open conversation addressable (the detail stays on
   * screen, read-only), which is why the store holds a row its scopes no longer
   * return. The delete path is the one caller that says otherwise — without
   * that distinction the two cases are indistinguishable, and the next two
   * tests are the bug that distinction fixes.
   */
  it("keeps the open conversation addressable when a refresh only lost its page", async () => {
    const h = await mountStore();
    expect(h.current.activeThreadId).toBe("t1");
    listThreadsPage.mockResolvedValue(pageOf([thread("t2")]));
    await act(async () => {
      await h.current.refreshStore();
    });
    expect(h.current.threads.map(t => t.id).sort()).toEqual(["t1", "t2"]);
    expect(h.current.activeThreadId).toBe("t1");
    h.unmount();
  });

  /**
   * Deleting the conversation that is open.
   *
   * Reported as "deleting the conversation I am reading leaves its row in the
   * sidebar and its history on screen": the delete removed the row in the
   * database, and the refresh that followed put it back — the store read "the
   * page does not return the open conversation" as "still open, just off this
   * page" and revived the row it was told was deleted.
   */
  it("deleting the open conversation drops its row and opens the nearest remaining one", async () => {
    const h = await mountStore();
    expect(h.current.activeThreadId).toBe("t1");
    // The page after the delete: t1 is gone. t9 is more recent but lives in
    // another workspace, so the fallback stays where the reader was working.
    listThreadsPage.mockResolvedValue(pageOf([
      { ...thread("t2", "w1"), lastMessageAt: 10 } as StoredThread,
      { ...thread("t9", "w2"), lastMessageAt: 99 } as StoredThread,
    ]));
    await act(async () => {
      await h.current.refreshStore(undefined, { removedThreadIds: ["t1"] });
    });
    expect(h.current.threads.map(t => t.id).sort()).toEqual(["t2", "t9"]);
    expect(h.current.activeThreadId).toBe("t2");
    expect(h.current.activeThread?.id).toBe("t2");
    h.unmount();
  });

  it("deleting the open conversation with nothing left opens nothing", async () => {
    const h = await mountStore();
    listThreadsPage.mockResolvedValue(pageOf([]));
    await act(async () => {
      await h.current.refreshStore(undefined, { removedThreadIds: ["t1", "t2"] });
    });
    expect(h.current.threads).toEqual([]);
    expect(h.current.activeThreadId).toBeNull();
    expect(h.current.activeThread).toBeNull();
    h.unmount();
  });

  it("drops a stale refreshStore response when a newer one is in flight", async () => {
    const h = await mountStore();
    let resolveSlow: ((value: unknown) => void) | null = null;
    let slowParked = false;
    listThreadsPage.mockImplementationOnce(() => new Promise((resolve) => {
      resolveSlow = resolve;
      slowParked = true;
    }));
    const slow = h.current.refreshStore("t2");
    // Let the slow refresh reach its page request (it awaits the pinned rows,
    // the workspace page and the active stretch first).
    for (let i = 0; i < 20; i += 1)
      await flushAsync();
    expect(slowParked).toBe(true);

    // A newer refresh supersedes the slow one.
    listThreadsPage.mockResolvedValue(pageOf([thread("t1")]));
    await act(async () => {
      await h.current.refreshStore("t1");
    });
    expect(h.current.activeThreadId).toBe("t1");
    // Now resolve the stale one — it must not overwrite the newer state.
    await act(async () => {
      resolveSlow!(pageOf([thread("t2")]));
      await slow;
    });
    expect(h.current.activeThreadId).toBe("t1");
    h.unmount();
  });

  /**
   * A re-list brings the workspace list with it, pin included.
   *
   * The rail orders its workspace groups by `pinned` and draws the marker from
   * the same flag, and a pin written on *another* machine — a phone pinning one
   * of this desktop's workspaces — reaches this window through the backend's
   * invalidation and this read. One that refreshed only the threads would leave
   * the group in its old place, unmarked, on the machine that owns it.
   */
  it("re-reads the workspace list, pin and all", async () => {
    const h = await mountStore();
    expect(h.current.workspaces).toEqual([workspace]);
    listWorkspacesPage.mockResolvedValue(workspacesPageOf([]));
    listPinnedWorkspaces.mockResolvedValue([{ ...workspace, pinned: true } as StoredWorkspace]);
    await act(async () => {
      await h.current.refreshStore();
    });
    expect(h.current.workspaces).toEqual([{ ...workspace, pinned: true }]);
    h.unmount();
  });

  /**
   * The recent view's own stream.
   *
   * Reported as "after a restart the recent list is missing most of my
   * conversations": the view ranked only the rows some *other* scope had
   * loaded (`chat`'s first page of ten roots, plus whatever workspace group was
   * expanded), and never asked for the one stream that spans the store. These
   * cases pin both halves of the fix down — the stream is requested, and it is
   * paged by the size its own control promises.
   */
  describe("the recent view's stream", () => {
    /**
     * `count` conversations from `offset`, as one page of roots.
     *
     * `total` is the whole stream's size, which the host reports with every page
     * (it describes the scope, not the page).
     */
    function recentPage(count: number, nextCursor: string | null, total: number, offset = 0) {
      return pageOf(
        Array.from({ length: count }, (_, index) => thread(`r${offset + index}`)),
        nextCursor,
        total,
      );
    }

    it("asks for the recent scope, thirty roots at a time", async () => {
      const h = await mountStore();
      listThreadsPage.mockClear();
      listThreadsPage.mockResolvedValue(recentPage(RECENT_PAGE_SIZE, null, RECENT_PAGE_SIZE));

      act(() => {
        h.current.ensureScope("recent");
      });
      await flushAsync();
      await flushAsync();

      expect(listThreadsPage).toHaveBeenCalledTimes(1);
      expect(listThreadsPage.mock.calls[0]![0]).toBe("recent");
      expect(listThreadsPage.mock.calls[0]![1]).toBeNull();
      // Thirty, not the tree's ten: the page is what the view's control names.
      expect(listThreadsPage.mock.calls[0]![2]).toBe(RECENT_PAGE_SIZE);
      // The stream holds its own rows, and only its own: the chat page's rows
      // are in the store but are not what this view draws.
      expect(h.current.recentThreadIds?.size).toBe(RECENT_PAGE_SIZE);
      expect(h.current.recentThreadIds?.has("t1")).toBe(false);
      h.unmount();
    });

    it("keeps the stream's own count and reveals all of it on one request", async () => {
      const h = await mountStore();
      listThreadsPage.mockReset();
      listThreadsPage
        .mockResolvedValueOnce(recentPage(RECENT_PAGE_SIZE, "cursor-1", 62))
        .mockResolvedValueOnce(recentPage(RECENT_PAGE_SIZE, "cursor-2", 62, RECENT_PAGE_SIZE))
        .mockResolvedValueOnce(recentPage(2, null, 62, 60));

      act(() => {
        h.current.ensureScope("recent");
      });
      await flushAsync();
      await flushAsync();

      // One page is in; the scope says where the stream ends, so the control can
      // name the real remainder instead of the loaded part.
      expect(h.current.scopeRemaining("recent")).toBe(32);

      await act(async () => {
        await h.current.loadRemaining("recent");
      });

      // Every page behind that one control, and the count is now zero.
      expect(listThreadsPage.mock.calls.map(call => call[1])).toEqual([null, "cursor-1", "cursor-2"]);
      expect(h.current.scopeRemaining("recent")).toBe(0);
      expect(h.current.scopeHasMore("recent")).toBe(false);
      expect(h.current.recentThreadIds?.size).toBe(62);
      h.unmount();
    });

    it("reports no stream at all until it is started", async () => {
      // The list must not read "empty" because nobody asked for it yet: absent
      // means "draw what you were handed", not "there is nothing".
      const h = await mountStore();
      expect(h.current.recentThreadIds).toBeUndefined();
      expect(h.current.scopeRemaining("recent")).toBe(0);
      h.unmount();
    });

    it("keeps a stream the reader opened while bootstrap was still running", async () => {
      // A window restored on the recent view asks for its stream at mount, which
      // is while bootstrap's own first pages are still in flight. The request
      // must not be thrown away — the view asks once per mount, so losing it left
      // the list on the chat page's rows until the reader switched views.
      let finishBootstrap: () => void = () => {};
      initializeAppStore.mockImplementation(() => new Promise<void>((resolve) => {
        finishBootstrap = resolve;
      }));
      listThreadsPage.mockImplementation((scope: unknown) => Promise.resolve(
        scope === "recent"
          ? recentPage(RECENT_PAGE_SIZE, "cursor-1", 62)
          : pageOf([thread("t1"), thread("t2")]),
      ));

      const h = renderHook(() => useThreadStore());
      act(() => {
        h.current.ensureScope("recent");
      });
      await flushAsync();
      // Held, not asked: the database this page is read from is not open yet.
      expect(listThreadsPage.mock.calls.filter(call => call[0] === "recent")).toHaveLength(0);

      finishBootstrap();
      await flushAsync();
      await flushAsync();
      await flushAsync();

      // The view draws its own page, still knows how much it holds back, and the
      // rows it read are in the store behind it.
      expect(h.current.recentThreadIds?.size).toBe(RECENT_PAGE_SIZE);
      expect(h.current.scopeRemaining("recent")).toBe(62 - RECENT_PAGE_SIZE);
      expect(h.current.threads.filter(thread => thread.id.startsWith("r"))).toHaveLength(RECENT_PAGE_SIZE);
      h.unmount();
    });

    it("keeps the rows a push brought in while bootstrap was still running", async () => {
      // Bootstrap contributes its first pages; it does not get to drop what a
      // refresh already merged (a `threads-updated` push lands during startup).
      let finishBootstrap: (recent: StoredThread) => void = () => {};
      getRecentOrCreateDefaultThread.mockImplementation(() => new Promise<StoredThread>((resolve) => {
        finishBootstrap = resolve;
      }));
      // Consumed by the push's refresh, not by bootstrap's own read.
      listPinnedThreads.mockResolvedValueOnce([thread("imported")]).mockResolvedValue([]);

      const h = renderHook(() => useThreadStore());
      await act(async () => {
        await h.current.refreshStore();
      });
      expect(h.current.threads.map(thread => thread.id)).toContain("imported");

      finishBootstrap(thread("t1"));
      await flushAsync();
      await flushAsync();

      expect(h.current.threads.map(thread => thread.id)).toContain("imported");
      h.unmount();
    });
  });

  it("ensureScope starts a scope exactly once", async () => {
    const h = await mountStore();
    listThreadsPage.mockResolvedValue(pageOf([]));
    act(() => {
      h.current.ensureScope("workspace:w1");
    });
    await flushAsync();
    act(() => {
      h.current.ensureScope("workspace:w1");
    });
    await flushAsync();
    const calls = listThreadsPage.mock.calls.filter(call => call[0] === "workspace:w1");
    expect(calls).toHaveLength(1);
    h.unmount();
  });

  it("reduces a thread-runtime-updated batch into run statuses", async () => {
    const h = await mountStore();
    act(() => {
      listeners.get("thread-runtime-updated")?.({
        payload: { updates: [
          { threadId: "t1", runId: "r1", revision: 5, status: "running", resetProjection: false },
          { threadId: "t2", runId: "r2", revision: 6, status: "completed", resetProjection: false },
        ] },
      });
    });
    expect(h.current.threadRunStatuses.t1).toMatchObject({ runId: "r1", status: "running" });
    expect(h.current.threadRunStatuses.t2).toMatchObject({ runId: "r2", status: "completed" });
    h.unmount();
  });

  it("patches one row's activity stamp from the backend push", async () => {
    listThreadsPage.mockResolvedValue(pageOf([
      { ...thread("t1"), lastMessageAt: 100 } as unknown as StoredThread,
      { ...thread("t2"), lastMessageAt: 200 } as unknown as StoredThread,
    ]));
    const h = await mountStore();

    act(() => {
      listeners.get("thread-activity-updated")?.({ payload: { threadId: "t1", lastMessageAt: 900 } });
    });
    // The row is patched (its age label and place both read this), and the
    // entries this app did not name keep their values.
    const stamps = h.current.threads
      .filter(t => t.id === "t1" || t.id === "t2")
      .map(t => [t.id, t.lastMessageAt]);
    expect(stamps).toEqual([["t1", 900], ["t2", 200]]);

    // A push never reloads the store: the list is patched in place, so the
    // reader's list is not rebuilt (and SQLite is not re-read) per second.
    expect(listThreadsPage).toHaveBeenCalledTimes(1);

    // An equal or older stamp, and a thread this list does not hold, keep the
    // array's identity so React skips the render.
    const unchanged = h.current.threads;
    act(() => {
      listeners.get("thread-activity-updated")?.({ payload: { threadId: "t1", lastMessageAt: 900 } });
      listeners.get("thread-activity-updated")?.({ payload: { threadId: "t1", lastMessageAt: 500 } });
      listeners.get("thread-activity-updated")?.({ payload: { threadId: "gone", lastMessageAt: 900 } });
    });
    expect(h.current.threads).toBe(unchanged);
    expect(listThreadsPage).toHaveBeenCalledTimes(1);
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
    // A tick starts, and then a push for the same thread arrives.
    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    act(() => {
      listeners.get("thread-runtime-updated")?.({
        payload: { updates: [{ threadId: "t1", runId: "r9", revision: 9, status: "running", resetProjection: false }] },
      });
    });
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
        payload: { updates: [{ threadId: "t1", runId: "r1", revision: 5, status: "running", resetProjection: false }] },
      });
    });
    expect(h.current.threadRunStatuses.t1).toBeDefined();
    listThreadsPage.mockResolvedValue(pageOf([thread("t1", "w1", "archived")]));
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
    listThreadsPage.mockResolvedValue(pageOf([thread("t1", "unknown-ws")]));
    listWorkspacesPage.mockResolvedValue(workspacesPageOf([workspace]));
    await act(async () => {
      await h.current.refreshStore("t1");
    });
    expect(h.current.activeWorkspace?.id).toBe("w1");
    h.unmount();
  });

  it("ignores a bootstrap that resolves after unmount", async () => {
    let resolveThreads!: (value: unknown) => void;
    listThreadsPage.mockImplementationOnce(() => new Promise((resolve) => {
      resolveThreads = resolve;
    }));

    const h = renderHook(() => useThreadStore());
    // Let initializeAppStore and the recent-thread read settle, so the first
    // page request is the one still parked.
    await flushAsync();
    await flushAsync();

    h.unmount();

    await act(async () => {
      resolveThreads(pageOf([thread("t1")]));
      await Promise.resolve();
      await Promise.resolve();
    });
    expect(h.current.threads).toEqual([]);
  });

  it("selectThread makes a row the open conversation even when no page holds it", async () => {
    const h = await mountStore();
    const archived = thread("old", "w1", "archived");
    act(() => {
      h.current.selectThread(archived);
    });
    expect(h.current.activeThreadId).toBe("old");
    expect(h.current.activeThread?.id).toBe("old");
    h.unmount();
  });
});
