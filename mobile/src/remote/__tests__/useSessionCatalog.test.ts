import { createElement, type MutableRefObject } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { useSessionCatalog } from "../useSessionCatalog";
import type { RemoteClient } from "../client";
import type { RemoteSession } from "../types";

type Catalog = ReturnType<typeof useSessionCatalog>;

function session(id: string, status?: string): RemoteSession {
  return {
    sessionId: id,
    threadId: `thread-${id}`,
    title: `Title ${id}`,
    streaming: false,
    status,
  };
}

describe("useSessionCatalog", () => {
  let clientRef: { current: RemoteClient | null };
  let selectedRef: { current: string };
  let request: jest.Mock;
  let onFinished: jest.Mock;
  let result: { current: Catalog };
  let renderer: ReactTestRenderer | null;
  let consoleError: jest.SpyInstance;

  function TestComponent(): null {
    result.current = useSessionCatalog(
      clientRef as MutableRefObject<RemoteClient | null>,
      selectedRef as MutableRefObject<string>,
      onFinished,
    );
    return null;
  }

  function render(): void {
    act(() => {
      renderer = create(createElement(TestComponent));
    });
  }

  beforeEach(() => {
    consoleError = jest.spyOn(console, "error");
    request = jest.fn();
    onFinished = jest.fn();
    clientRef = {
      current: { request, requestRetry: request } as unknown as RemoteClient,
    };
    selectedRef = { current: "s1" };
    result = { current: undefined as unknown as Catalog };
    renderer = null;
  });

  afterEach(() => {
    if (renderer) {
      act(() => renderer!.unmount());
      renderer = null;
    }
    try { expect(consoleError).not.toHaveBeenCalled(); }
    finally { consoleError.mockRestore(); }
  });

  test("100 concurrent refreshes share one request and one trailing freshness read", async () => {
    let finish!: (value: unknown) => void;
    request.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }))
      .mockResolvedValueOnce({ data: { sessions: [session("latest")] } });
    render();
    let pending!: Promise<void>[];
    act(() => { pending = Array.from({ length: 100 }, () => result.current.refreshSessions()); });
    expect(request).toHaveBeenCalledTimes(1);
    expect(new Set(pending).size).toBe(1);
    await act(async () => { finish({ data: { sessions: [session("older")] } }); await Promise.all(pending); });
    expect(request).toHaveBeenCalledTimes(2);
    expect(result.current.sessions.map(item => item.sessionId)).toEqual(["latest"]);
  });

  test("a replaced client starts a fresh refresh without waiting for the old flight", async () => {
    let finish!: (value: unknown) => void;
    request.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    render();
    let old!: Promise<void>;
    act(() => { old = result.current.refreshSessions(); });
    const nextRequest = jest.fn().mockResolvedValue({ data: { sessions: [session("new")] } });
    clientRef.current = { requestRetry: nextRequest } as unknown as RemoteClient;
    await act(async () => { await result.current.refreshSessions(); });
    await act(async () => { finish({ data: { sessions: [session("old")] } }); await old; });
    expect(result.current.sessions.map(item => item.sessionId)).toEqual(["new"]);
  });

  test("late pull cannot overwrite a more recent pushed sessions snapshot", async () => {
    render();
    let resolve!: (value: unknown) => void;
    request.mockReturnValueOnce(
      new Promise((done) => {
        resolve = done;
      }),
    );
    let pulling!: Promise<void>;
    act(() => { pulling = result.current.refreshSessions(); });
    act(() => void result.current.applySessionSnapshot([session("new")]));
    await act(async () => {
      resolve({ data: { sessions: [session("old")] } });
      await pulling;
    });
    expect(result.current.sessions.map((item) => item.sessionId)).toEqual(["new"]);
  });

  test("reset fences outstanding directory and settings requests", async () => {
    render();
    let resolve!: (value: unknown) => void;
    request.mockReturnValue(
      new Promise((done) => {
        resolve = done;
      }),
    );
    let pulling!: Promise<void[]>;
    act(() => {
      pulling = Promise.all([
        result.current.refreshSessions(),
        result.current.refreshWorkspaces(),
        result.current.refreshSettings(),
      ]);
    });
    act(() => result.current.reset());
    await act(async () => {
      resolve({
        data: {
          sessions: [session("old")],
          workspaces: [{ id: "old" }],
          approvalTier: "manual",
          sandboxAvailable: true,
        },
      });
      await pulling;
    });
    expect(result.current.sessions).toEqual([]);
    expect(result.current.workspaces).toEqual([]);
    expect(result.current.approvalTier).toBe("off");
    expect(result.current.sandboxAvailable).toBe(false);
  });

  test("late delete completion cannot close or remove a new pairing's conversation", async () => {
    render();
    let resolve!: (value: unknown) => void;
    request.mockReturnValueOnce(
      new Promise((done) => {
        resolve = done;
      }),
    );
    const deleting = result.current.deleteSession("s1", "thread-s1");
    act(() => {
      result.current.reset();
      result.current.applySessionSnapshot([session("s1")]);
    });
    let close = true;
    await act(async () => {
      resolve({ data: {} });
      close = await deleting;
    });
    expect(close).toBe(false);
    expect(result.current.sessions.map((item) => item.sessionId)).toEqual(["s1"]);
  });

  test("completion callbacks include the open session, deduplicate snapshots, and skip initial history/cancellation", () => {
    render();
    act(() => { result.current.applySessionSnapshot([session("s1", "completed")]); });
    expect(onFinished).not.toHaveBeenCalled();
    act(() => { result.current.applySessionSnapshot([session("s1", "running")]); });
    act(() => { result.current.applySessionSnapshot([session("s1", "completed")]); });
    act(() => { result.current.applySessionSnapshot([session("s1", "completed")]); });
    expect(onFinished).toHaveBeenCalledTimes(1);
    expect(onFinished).toHaveBeenCalledWith(expect.objectContaining({ sessionId: "s1", status: "completed" }));
    expect(result.current.unreadSessions.size).toBe(0);
    act(() => { result.current.applySessionSnapshot([session("s1", "running")]); });
    act(() => { result.current.applySessionSnapshot([session("s1", "cancelled")]); });
    expect(onFinished).toHaveBeenCalledTimes(1);
    act(() => { result.current.reset(); result.current.applySessionSnapshot([session("s1", "failed")]); });
    expect(onFinished).toHaveBeenCalledTimes(1);
  });

  test("live completion covers fast runs and deduplicates the later snapshot and repeated terminal event", () => {
    render();
    const event = { type: "agent_end", data: "{}", runId: "r1", idx: 2 };
    act(() => { result.current.observeRunEvent({ ...event, type: "agent_start" }, "s1"); });
    act(() => { result.current.observeRunEvent(event, "s1"); });
    expect(onFinished).toHaveBeenCalledTimes(1);
    act(() => {
      result.current.applySessionSnapshot([session("s1", "running")]);
      result.current.applySessionSnapshot([session("s1", "completed")]);
      result.current.observeRunEvent(event, "s1");
    });
    expect(onFinished).toHaveBeenCalledTimes(1);
    act(() => { result.current.observeRunEvent({ ...event, runId: "r2", data: '{"state":"cancelled"}' }, "s1"); });
    expect(onFinished).toHaveBeenCalledTimes(1);
  });

  test("a snapshot arriving before the live terminal only reminds once; the next run still reminds", () => {
    render();
    const event = { type: "agent_start", data: "{}", runId: "r1" };
    act(() => {
      result.current.observeRunEvent(event, "s1");
      result.current.applySessionSnapshot([session("s1", "running")]);
      result.current.applySessionSnapshot([session("s1", "failed")]);
      result.current.observeRunEvent({ ...event, type: "agent_end", data: '{"state":"failed"}' }, "s1");
    });
    expect(onFinished).toHaveBeenCalledTimes(1);
    act(() => {
      result.current.applySessionSnapshot([session("s1", "running")]);
      result.current.applySessionSnapshot([session("s1", "completed")]);
    });
    expect(onFinished).toHaveBeenCalledTimes(2);
  });

  test.each(['{"state":"error"}', '{"state":"incomplete"}', '{"error":"failed to spawn"}', '{"reason":"incomplete"}'])("live failure payload %s is never announced as success", data => {
    render();
    act(() => { result.current.observeRunEvent({ type: "agent_end", data, runId: "r1" }, "s1"); });
    expect(onFinished).toHaveBeenCalledWith(expect.objectContaining({ status: "failed" }));
  });

  test("streaming-only running state is detected and reset prevents cross-desktop history alerts", () => {
    render();
    act(() => { result.current.applySessionSnapshot([{ ...session("s1"), streaming: true }]); });
    act(() => { result.current.applySessionSnapshot([session("s1", "completed")]); });
    expect(onFinished).toHaveBeenCalledTimes(1);
    act(() => {
      result.current.observeRunEvent({ type: "agent_end", data: "{}", runId: "r1" }, "s1");
      result.current.reset();
      result.current.applySessionSnapshot([session("s1", "completed")]);
    });
    const count = onFinished.mock.calls.length;
    act(() => { result.current.observeRunEvent({ type: "agent_end", data: "{}", runId: "r1" }, "s1"); });
    expect(onFinished).toHaveBeenCalledTimes(count + 1);
  });

  test("a confirming push retires a title overlay so later Desktop renames are visible", () => {
    render();
    act(() => {
      result.current.setCatalogEpoch("epoch");
      result.current.applySessionSnapshot([session("s1")], { epoch: "epoch", revision: 1 });
      result.current.setTitleOverrides({ s1: "Phone name" });
      result.current.applySessionSnapshot([{ ...session("s1"), title: "Phone name" }], { epoch: "epoch", revision: 2 });
    });
    expect(result.current.titleOverrides).toEqual({});
    act(() => { result.current.applySessionSnapshot([{ ...session("s1"), title: "Desktop name" }], { epoch: "epoch", revision: 3 }); });
    expect(result.current.sessions[0]?.title).toBe("Desktop name");
  });

  test.each([1, 2])("a fresh read retires overlays even when confirmation was missed (revision %s)", async revision => {
    render();
    act(() => {
      result.current.setCatalogEpoch("epoch");
      result.current.applySessionSnapshot([session("s1")], { epoch: "epoch", revision: 1 });
      result.current.setTitleOverrides({ s1: "Phone name" });
    });
    request.mockResolvedValue({ data: { sessions: [session("s1")], version: { epoch: "epoch", revision } } });
    await act(async () => { await result.current.refreshSessions(); });
    expect(result.current.titleOverrides).toEqual({});
    expect(result.current.sessions[0]?.title).toBe("Title s1");
  });

  test("a pre-rename read cannot retire the new overlay; the trailing post-ack read can", async () => {
    render();
    act(() => {
      result.current.setCatalogEpoch("epoch");
      result.current.applySessionSnapshot([session("s1")], { epoch: "epoch", revision: 1 });
    });
    let oldReply!: (value: unknown) => void;
    let newReply!: (value: unknown) => void;
    request.mockReturnValueOnce(new Promise(resolve => { oldReply = resolve; }))
      .mockResolvedValueOnce({ data: {} })
      .mockReturnValueOnce(new Promise(resolve => { newReply = resolve; }));
    let pending!: Promise<void>;
    act(() => { pending = result.current.refreshSessions(); });
    await act(async () => { await result.current.rename("s1", "Phone name"); });
    await act(async () => {
      oldReply({ data: { sessions: [session("s1")], version: { epoch: "epoch", revision: 2 } } });
      for (let i = 0; i < 10; i++) await Promise.resolve();
    });
    expect(result.current.titleOverrides.s1).toBe("Phone name");
    expect(result.current.sessions[0]?.title).toBe("Phone name");
    await act(async () => {
      newReply({ data: { sessions: [{ ...session("s1"), title: "Desktop newer" }], version: { epoch: "epoch", revision: 3 } } });
      await pending;
    });
    expect(result.current.titleOverrides).toEqual({});
    expect(result.current.sessions[0]?.title).toBe("Desktop newer");
  });

  test("an older version cannot retire overlays even when returned to a fresh request", async () => {
    render();
    act(() => {
      result.current.setCatalogEpoch("epoch");
      result.current.applySessionSnapshot([session("s1")], { epoch: "epoch", revision: 5 });
      result.current.setTitleOverrides({ s1: "Phone name" });
    });
    request.mockResolvedValue({ data: { sessions: [session("s1")], version: { epoch: "epoch", revision: 4 } } });
    await act(async () => { await result.current.refreshSessions(); });
    expect(result.current.titleOverrides.s1).toBe("Phone name");
  });

  test("returns the full catalogue surface on mount", () => {
    render();
    expect(result.current.sessions).toEqual([]);
    expect(result.current.unreadSessions).toEqual(new Set());
    expect(result.current.workspaces).toEqual([]);
    expect(result.current.models).toEqual([]);
    expect(result.current.approvalTier).toBe("off");
    expect(result.current.sandboxAvailable).toBe(false);
    expect(result.current.titleOverrides).toEqual({});
    expect(typeof result.current.applySessionSnapshot).toBe("function");
  });

  test("applySessionSnapshot decorates titles and detects a finished transition", async () => {
    render();
    // Baseline: s2 is running (establishes lastStatusRef).
    act(() => void result.current.applySessionSnapshot([session("s2", "running")]));
    expect(result.current.sessions[0]?.title).toBe("Title s2");

    // Rename s2 to install a title override (synced to titleOverridesRef via effect).
    request.mockResolvedValueOnce({ data: {} });
    await act(async () => {
      await result.current.rename("s2", "Custom Title");
    });

    // Second snapshot: s2 completed — uses the override and flags unread.
    act(() => void result.current.applySessionSnapshot([session("s2", "completed")]));
    expect(result.current.sessions[0]).toMatchObject({
      sessionId: "s2",
      title: "Custom Title",
    });
    expect(result.current.unreadSessions.has("s2")).toBe(true);
  });

  test("applySessionSnapshot leaves the selected session out of unread", () => {
    render();
    act(() => void result.current.applySessionSnapshot([session("s1", "running")]));
    act(() => void result.current.applySessionSnapshot([session("s1", "completed")]));
    expect(result.current.unreadSessions.has("s1")).toBe(false);
  });

  test("refreshSessions applies the pushed snapshot", async () => {
    render();
    request.mockResolvedValueOnce({
      data: { sessions: [session("s2", "running")] },
    });
    await act(async () => {
      await result.current.refreshSessions();
    });
    expect(result.current.sessions).toHaveLength(1);
    expect(request).toHaveBeenCalledWith({ type: "list_sessions" }, "list");
  });

  test("refreshSessions tolerates a missing sessions array", async () => {
    render();
    request.mockResolvedValueOnce({ data: {} });
    await act(async () => {
      await result.current.refreshSessions();
    });
    expect(result.current.sessions).toEqual([]);
  });

  test("refreshSessions swallows a dropped connection", async () => {
    render();
    request.mockRejectedValueOnce(new Error("not_connected"));
    await act(async () => {
      await result.current.refreshSessions();
    });
    expect(result.current.sessions).toEqual([]);
  });

  test("refreshModels returns models on the first attempt", async () => {
    render();
    request.mockResolvedValueOnce({
      data: { models: [{ id: "m1", label: "M1" }] },
    });
    await act(async () => {
      await result.current.refreshModels();
    });
    expect(result.current.models).toEqual([{ id: "m1", label: "M1" }]);
    expect(request).toHaveBeenCalledTimes(1);
  });

  test("hiding every desktop model clears a previous catalogue without warm-up retries, then restores it", async () => {
    jest.useFakeTimers();
    try {
      render();
      request.mockResolvedValueOnce({ data: { models: [{ id: "m1" }] } });
      await act(async () => { await result.current.refreshModels(); });
      request.mockResolvedValueOnce({ data: { models: [], allModelsHidden: true } });
      await act(async () => { await result.current.refreshModels(); });
      expect(result.current.models).toEqual([]);
      expect(result.current.catalogSync.models).toBe("ready");
      await act(async () => { await jest.runAllTimersAsync(); });
      expect(request).toHaveBeenCalledTimes(2);
      request.mockResolvedValueOnce({ data: { models: [{ id: "m1" }], allModelsHidden: false } });
      await act(async () => { await result.current.refreshModels(); });
      expect(result.current.models).toEqual([{ id: "m1" }]);
    } finally {
      jest.useRealTimers();
    }
  });

  test("a stale model response cannot resurrect models after all are hidden", async () => {
    render();
    let resolve!: (value: unknown) => void;
    request.mockReturnValueOnce(new Promise(yes => { resolve = yes; }));
    let stale!: Promise<void>;
    act(() => { stale = result.current.refreshModels(); });
    request.mockResolvedValueOnce({ data: { models: [], allModelsHidden: true } });
    await act(async () => { await result.current.refreshModels(); });
    await act(async () => {
      resolve({ data: { models: [{ id: "hidden" }] } });
      await stale;
    });
    expect(result.current.models).toEqual([]);
    expect(result.current.catalogSync.models).toBe("ready");
  });

  test("refreshModels retries in the background after an empty first answer", async () => {
    jest.useFakeTimers();
    try {
      render();
      request
        .mockResolvedValueOnce({ data: { models: [] } })
        .mockResolvedValueOnce({ data: { models: [{ id: "m1" }] } });
      let pending: Promise<void> | undefined;
      act(() => {
        pending = result.current.refreshModels();
      });
      await act(async () => {
        await jest.runAllTimersAsync();
      });
      await act(async () => {
        await pending;
      });
      expect(result.current.models).toEqual([{ id: "m1" }]);
      expect(request).toHaveBeenCalledTimes(2);
    } finally {
      jest.useRealTimers();
    }
  });

  test("refreshModels clears a never-populated list after the retry budget is spent", async () => {
    jest.useFakeTimers();
    try {
      render();
      request.mockResolvedValue({ data: { models: [] } });
      let pending: Promise<void> | undefined;
      act(() => {
        pending = result.current.refreshModels();
      });
      await act(async () => {
        await jest.runAllTimersAsync();
      });
      await act(async () => {
        await pending;
      });
      // All four retry delays elapsed with an empty list; the catalogue stays
      // empty (and is explicitly cleared because no previous list exists).
      expect(result.current.models).toEqual([]);
      expect(request).toHaveBeenCalledTimes(5);
    } finally {
      jest.useRealTimers();
    }
  });

  test("refreshModels retries in the background after a failed first attempt", async () => {
    jest.useFakeTimers();
    try {
      render();
      request
        .mockRejectedValueOnce(new Error("warming up"))
        .mockResolvedValueOnce({ data: { models: [{ id: "m2" }] } });
      let pending: Promise<void> | undefined;
      act(() => {
        pending = result.current.refreshModels();
      });
      await act(async () => {
        await jest.runAllTimersAsync();
      });
      await act(async () => {
        await pending;
      });
      expect(result.current.models).toEqual([{ id: "m2" }]);
    } finally {
      jest.useRealTimers();
    }
  });

  test("refreshSettings updates approval tier and sandbox availability", async () => {
    render();
    request.mockResolvedValueOnce({
      data: { approvalTier: "high", sandboxAvailable: true },
    });
    await act(async () => {
      await result.current.refreshSettings();
    });
    expect(result.current.approvalTier).toBe("high");
    expect(result.current.sandboxAvailable).toBe(true);
  });

  test("a failed settings read is recorded as failed and keeps the previous tier", async () => {
    render();
    request.mockResolvedValueOnce({ data: { approvalTier: "sandbox", sandboxAvailable: true } });
    await act(async () => { await result.current.refreshSettings(); });
    expect(result.current.approvalTier).toBe("sandbox");
    request.mockRejectedValueOnce(new Error("desktop offline"));
    await act(async () => { await result.current.refreshSettings(); });
    // The domain reports `failed` so the panel can offer a retry, and the tier
    // the desktop last confirmed is not replaced by a guess.
    expect(result.current.catalogSync.settings).toBe("failed");
    expect(result.current.approvalTier).toBe("sandbox");
  });

  test("refreshWorkspaces updates the workspace list", async () => {
    render();
    request.mockResolvedValueOnce({
      data: { workspaces: [{ id: "w1", name: "W", path: "/w" }] },
    });
    await act(async () => {
      await result.current.refreshWorkspaces();
    });
    expect(result.current.workspaces).toEqual([{ id: "w1", name: "W", path: "/w" }]);
  });

  test("refreshWorkspaces preserves the last snapshot on error", async () => {
    render();
    request.mockResolvedValueOnce({
      data: { workspaces: [{ id: "w1", name: "W", path: "/w" }] },
    });
    await act(async () => {
      await result.current.refreshWorkspaces();
    });
    request.mockRejectedValueOnce(new Error("gone"));
    await act(async () => {
      await result.current.refreshWorkspaces();
    });
    expect(result.current.workspaces).toEqual([{ id: "w1", name: "W", path: "/w" }]);
  });

  test("reset clears catalogue state", async () => {
    render();
    request.mockResolvedValueOnce({
      data: { sessions: [session("s2", "running")] },
    });
    await act(async () => {
      await result.current.refreshSessions();
    });
    expect(result.current.sessions).toHaveLength(1);
    act(() => result.current.reset());
    expect(result.current.sessions).toEqual([]);
    expect(result.current.workspaces).toEqual([]);
    expect(result.current.titleOverrides).toEqual({});
  });

  test("title suggestion uses the chosen session and locale without renaming", async () => {
    render();
    act(() => void result.current.applySessionSnapshot([session("s1", "running")]));
    const originalTitle = result.current.sessions[0]!.title;
    request.mockResolvedValueOnce({ success: true, data: { title: "Suggested" } });
    let title = "";
    await act(async () => { title = await result.current.generateTitle("s1", "zh"); });
    expect(title).toBe("Suggested");
    expect(request).toHaveBeenCalledWith(
      { type: "generate_session_title", sessionId: "s1", mode: "zh" }, "s1", 65_000,
    );
    expect(result.current.sessions[0]!.title).toBe(originalTitle);
    expect(result.current.titleOverrides).toEqual({});
  });

  test("rename trims the name and updates both the override and the session", async () => {
    render();
    act(
      () =>
        void result.current.applySessionSnapshot([
          session("s1", "running"),
          session("s2", "running"),
        ]),
    );
    request.mockResolvedValueOnce({ data: {} });
    await act(async () => {
      await result.current.rename("s2", "  New Name  ");
    });
    expect(request).toHaveBeenCalledWith(
      { type: "set_session_name", sessionId: "s2", name: "New Name" },
      "s2",
    );
    expect(result.current.titleOverrides["s2"]).toBe("New Name");
    expect(
      result.current.sessions.map((s) => (s.sessionId === "s2" ? s.title : s.sessionId)),
    ).toEqual(["s1", "New Name"]);
  });

  test("rename ignores an empty session id or a blank name", async () => {
    render();
    await act(async () => {
      await result.current.rename("", "name");
    });
    await act(async () => {
      await result.current.rename("s2", "   ");
    });
    expect(request).not.toHaveBeenCalled();
  });

  test("deleteSession drops the session and reports the selected one", async () => {
    render();
    act(
      () =>
        void result.current.applySessionSnapshot([
          session("s1", "running"),
          session("s2", "running"),
        ]),
    );
    request.mockResolvedValueOnce({ data: {} });
    let selected = false;
    await act(async () => {
      selected = await result.current.deleteSession("s1", "thread-s1");
    });
    expect(selected).toBe(true);
    expect(result.current.sessions.map((s) => s.sessionId)).toEqual(["s2"]);
  });

  test("deleteSession returns false for a non-selected session", async () => {
    render();
    act(() => void result.current.applySessionSnapshot([session("s2", "running")]));
    request.mockResolvedValueOnce({ data: {} });
    let selected = false;
    await act(async () => {
      selected = await result.current.deleteSession("s2", "thread-s2");
    });
    expect(selected).toBe(false);
  });

  test("deleteSession rejects invalid requests rather than reporting a successful deletion", async () => {
    render();
    await expect(result.current.deleteSession("", "")).rejects.toThrow("Session unavailable");
    expect(request).not.toHaveBeenCalled();
  });

  test("deleteSession rejects a dropped connection and retains the catalogue for retry", async () => {
    render();
    act(() => void result.current.applySessionSnapshot([session("s1")]));
    clientRef.current = null;
    await expect(result.current.deleteSession("s1", "thread-s1")).rejects.toThrow(
      "Session unavailable",
    );
    expect(result.current.sessions).toHaveLength(1);
    expect(request).not.toHaveBeenCalled();
  });

  test("deleteWorkspace rejects a dropped connection and retains the catalogue for retry", async () => {
    render();
    act(() => void result.current.applySessionSnapshot([session("s1")]));
    clientRef.current = null;
    await expect(result.current.deleteWorkspace("w1")).rejects.toThrow("Workspace unavailable");
    expect(result.current.sessions).toHaveLength(1);
    expect(request).not.toHaveBeenCalled();
  });

  test("deleteWorkspace rejects an empty id rather than reporting a successful deletion", async () => {
    render();
    await expect(result.current.deleteWorkspace("")).rejects.toThrow("Workspace unavailable");
    expect(request).not.toHaveBeenCalled();
  });

  test("deleteWorkspace drops the workspace, its sessions and their unread markers", async () => {
    render();
    selectedRef.current = "";
    request.mockResolvedValueOnce({
      data: { workspaces: [{ id: "w1", name: "W", path: "/w" }] },
    });
    await act(async () => {
      await result.current.refreshWorkspaces();
    });
    const inWorkspace = (id: string, workspaceId: string): RemoteSession => ({
      ...session(id, "running"),
      mode: "workspace",
      workspaceId,
    });
    act(
      () =>
        void result.current.applySessionSnapshot([
          inWorkspace("s1", "w1"),
          inWorkspace("s2", "w2"),
        ]),
    );
    // Both finish while nothing is selected, so both are flagged unread.
    act(
      () =>
        void result.current.applySessionSnapshot([
          { ...inWorkspace("s1", "w1"), status: "completed" },
          { ...inWorkspace("s2", "w2"), status: "completed" },
        ]),
    );
    expect(result.current.unreadSessions).toEqual(new Set(["s1", "s2"]));

    request.mockResolvedValueOnce({ data: {} });
    let selected = false;
    await act(async () => {
      selected = await result.current.deleteWorkspace("w1");
    });
    expect(request).toHaveBeenLastCalledWith(
      { type: "delete_workspace", workspaceId: "w1" },
      "list",
    );
    expect(result.current.workspaces).toEqual([]);
    expect(result.current.sessions.map((s) => s.sessionId)).toEqual(["s2"]);
    expect(result.current.unreadSessions).toEqual(new Set(["s2"]));
    // The selected session (none) was not inside the deleted workspace.
    expect(selected).toBe(false);
  });

  test("deleteWorkspace reports a deleted selected session so the caller closes it", async () => {
    render();
    const inWorkspace = (id: string): RemoteSession => ({
      ...session(id),
      mode: "workspace",
      workspaceId: "w1",
    });
    act(() => void result.current.applySessionSnapshot([inWorkspace("s1")]));
    request.mockResolvedValueOnce({ data: {} });
    let selected = false;
    await act(async () => {
      selected = await result.current.deleteWorkspace("w1");
    });
    expect(selected).toBe(true);
    expect(result.current.sessions).toEqual([]);
  });

  test("deleteWorkspace keeps the local catalogue when the desktop refuses", async () => {
    render();
    act(
      () =>
        void result.current.applySessionSnapshot([
          { ...session("s1"), mode: "workspace", workspaceId: "w1" },
        ]),
    );
    request.mockRejectedValueOnce(new Error("workspace not found"));
    await expect(result.current.deleteWorkspace("w1")).rejects.toThrow("workspace not found");
    expect(result.current.sessions.map((s) => s.sessionId)).toEqual(["s1"]);
  });

  test("deleting a workspace also drops the phone-side titles of its sessions", async () => {
    // A title override is keyed by session id and outlives the session. If it
    // were kept, a later session that reuses the id would show a name the user
    // typed for an unrelated conversation.
    render();
    act(() => void result.current.applySessionSnapshot([
      { ...session("s1"), mode: "workspace", workspaceId: "w1" },
    ]));
    act(() => result.current.setTitleOverrides({ s1: "Phone name", "elsewhere": "Kept" }));
    request.mockResolvedValueOnce({ data: {} });
    await act(async () => { await result.current.deleteWorkspace("w1"); });
    expect(result.current.titleOverrides).toEqual({ elsewhere: "Kept" });
  });

  test("a live finish marks a session the user is not reading as unread", async () => {
    render();
    selectedRef.current = "other";
    act(() => {
      result.current.observeRunEvent({ type: "agent_start", data: "{}", runId: "r1" }, "s1");
      result.current.observeRunEvent({ type: "agent_end", data: "{}", runId: "r1" }, "s1");
    });
    expect(onFinished).toHaveBeenCalledWith(expect.objectContaining({ sessionId: "s1" }));
    expect(result.current.unreadSessions.has("s1")).toBe(true);
    // The one on screen is not flagged: the user is looking at it.
    act(() => {
      result.current.observeRunEvent({ type: "agent_start", data: "{}", runId: "r2" }, "other");
      result.current.observeRunEvent({ type: "agent_end", data: "{}", runId: "r2" }, "other");
    });
    expect(result.current.unreadSessions.has("other")).toBe(false);
  });

  test("setSessionPinned reorders pinned sessions to the top", async () => {
    render();
    act(
      () =>
        void result.current.applySessionSnapshot([
          session("a", "running"),
          session("b", "running"),
        ]),
    );
    request.mockResolvedValueOnce({ data: {} });
    await act(async () => {
      await result.current.setSessionPinned("b", "thread-b", true);
    });
    expect(result.current.sessions.map((s) => s.sessionId)).toEqual(["b", "a"]);
  });

  test("setSessionPinned ignores empty ids", async () => {
    render();
    await act(async () => {
      await result.current.setSessionPinned("", "", true);
    });
    expect(request).not.toHaveBeenCalled();
  });

  const workspaceSnapshot = (revision: number, pinned: boolean, epoch = "source") => ({
    version: { epoch, revision },
    workspaces: [{ id: "w", name: "Workspace", path: "/w", pinned }],
  });
  function renderWorkspaceCatalog() {
    render();
    act(() => {
      result.current.setCatalogEpoch("source");
      const initial = workspaceSnapshot(1, false);
      result.current.setWorkspaces(initial.workspaces, initial.version);
    });
  }

  test("workspace pin acknowledgements update through the versioned snapshot", async () => {
    renderWorkspaceCatalog();
    for (const [revision, pinned] of [[2, true], [3, false]] as const) {
      request.mockResolvedValueOnce({ data: workspaceSnapshot(revision, pinned) });
      await act(async () => { await result.current.setWorkspacePinned("w", pinned); });
      expect(result.current.workspaces[0]?.pinned).toBe(pinned);
      expect(request).toHaveBeenLastCalledWith(
        { type: "set_workspace_pinned", workspaceId: "w", pinned }, "list",
      );
    }
  });

  test("a late pin acknowledgement cannot undo a newer unpin or its repeated push", async () => {
    renderWorkspaceCatalog();
    let finishPin!: (response: unknown) => void;
    request.mockReturnValueOnce(new Promise(resolve => { finishPin = resolve; }));
    let pin!: Promise<void>;
    act(() => { pin = result.current.setWorkspacePinned("w", true); });
    // The server committed the pin; its push arrives before its acknowledgement.
    act(() => {
      const snapshot = workspaceSnapshot(2, true);
      result.current.setWorkspaces(snapshot.workspaces, snapshot.version);
    });
    request.mockResolvedValueOnce({ data: workspaceSnapshot(3, false) });
    await act(async () => { await result.current.setWorkspacePinned("w", false); });
    await act(async () => {
      finishPin({ data: workspaceSnapshot(2, true) });
      await pin;
    });
    expect(result.current.workspaces[0]?.pinned).toBe(false);
    act(() => {
      const snapshot = workspaceSnapshot(3, false);
      result.current.setWorkspaces(snapshot.workspaces, snapshot.version);
    });
    expect(result.current.workspaces[0]?.pinned).toBe(false);
  });

  test("an in-flight old workspace pull cannot undo an acknowledged pin", async () => {
    renderWorkspaceCatalog();
    let finishRead!: (response: unknown) => void;
    request.mockReturnValueOnce(new Promise(resolve => { finishRead = resolve; }));
    let read!: Promise<void>;
    act(() => { read = result.current.refreshWorkspaces(); });
    request.mockResolvedValueOnce({ data: workspaceSnapshot(3, true) });
    await act(async () => { await result.current.setWorkspacePinned("w", true); });
    await act(async () => { finishRead({ data: workspaceSnapshot(2, false) }); await read; });
    expect(result.current.workspaces[0]?.pinned).toBe(true);
  });

  test.each(["client", "epoch"])("workspace pin replies cannot cross a changed %s", async change => {
    renderWorkspaceCatalog();
    let finish!: (response: unknown) => void;
    request.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    let pin!: Promise<void>;
    act(() => { pin = result.current.setWorkspacePinned("w", true); });
    act(() => {
      if (change === "client") clientRef.current = { request } as unknown as RemoteClient;
      else result.current.setCatalogEpoch("replacement");
    });
    await act(async () => { finish({ data: workspaceSnapshot(2, true) }); await pin; });
    expect(result.current.workspaces[0]?.pinned).toBe(false);
  });

  test("workspace pin failures and malformed replies leave the catalog intact", async () => {
    renderWorkspaceCatalog();
    request.mockRejectedValueOnce(new Error("offline"));
    await expect(result.current.setWorkspacePinned("w", true)).rejects.toThrow("offline");
    request.mockResolvedValueOnce({ data: {} });
    await expect(result.current.setWorkspacePinned("w", true)).rejects.toThrow("Invalid workspace snapshot");
    expect(result.current.workspaces[0]?.pinned).toBe(false);
  });

  test("setWorkspacePinned rejects an empty id or missing connection", async () => {
    render();
    await expect(result.current.setWorkspacePinned("", true)).rejects.toThrow("Workspace unavailable");
    clientRef.current = null;
    await expect(result.current.setWorkspacePinned("w", true)).rejects.toThrow("Workspace unavailable");
    expect(request).not.toHaveBeenCalled();
  });
  test("versioned pulls and pushes converge at one commit point and report sync readiness", async () => {
    render();
    act(() => {
      result.current.setCatalogEpoch("source");
      result.current.applySessionSnapshot([session("new")], { epoch: "source", revision: 2 });
    });
    request.mockResolvedValueOnce({
      data: { sessions: [session("latest")], version: { epoch: "source", revision: 3 } },
    });
    await act(async () => {
      await result.current.refreshSessions();
    });
    act(() => {
      result.current.applySessionSnapshot([session("old")], { epoch: "source", revision: 1 });
    });
    expect(result.current.sessions[0]?.sessionId).toBe("latest");
    expect(result.current.catalogSync.sessions).toBe("ready");
    request.mockResolvedValueOnce({
      data: { sessions: [session("latest")], version: { epoch: "source", revision: 3 } },
    });
    await act(async () => {
      await result.current.refreshSessions();
    });
    expect(result.current.catalogSync.sessions).toBe("ready");
  });

  /**
   * The presence heartbeat is the only recovery signal left for a catalog push
   * that the at-most-once event lane dropped: the desktop no longer re-sends an
   * unchanged snapshot on a timer. Both directions matter — missing the pull
   * leaves the phone permanently stale, and pulling on a revision that already
   * arrived turns every heartbeat into a redundant round trip.
   */
  test("a presence revision newer than the applied snapshot pulls, a current one does not", async () => {
    render();
    act(() => { result.current.setCatalogEpoch("E"); });
    // Both domains applied, so the heartbeat below is exactly current.
    act(() => {
      result.current.applySessionSnapshot([session("s1")], { epoch: "E", revision: 4 });
      result.current.setWorkspaces([], { epoch: "E", revision: 2 });
    });
    // The heartbeat agrees with what was applied: the push arrived, nothing to do.
    act(() => { result.current.noteCatalogRevisions({ epoch: "E", sessions: 4, workspaces: 2 }); });
    expect(request).not.toHaveBeenCalled();

    // Newer than applied proves the pushed snapshot was lost, so pull.
    request.mockResolvedValue({
      data: { sessions: [session("s1")], version: { epoch: "E", revision: 5 } },
    });
    await act(async () => {
      result.current.noteCatalogRevisions({ epoch: "E", sessions: 5, workspaces: 2 });
    });
    expect(request).toHaveBeenCalledWith({ type: "list_sessions" }, "list");
  });

  test("presence revisions are ignored across epochs and drive each domain separately", async () => {
    render();
    act(() => { result.current.setCatalogEpoch("E"); });
    // Another desktop generation: the reconnect path owns that transition.
    act(() => { result.current.noteCatalogRevisions({ epoch: "OTHER", sessions: 9, workspaces: 9 }); });
    expect(request).not.toHaveBeenCalled();

    // Pre-handshake presence carries no authenticated epoch to compare against.
    act(() => { result.current.setCatalogEpoch(undefined); });
    act(() => { result.current.noteCatalogRevisions({ epoch: "E", sessions: 9, workspaces: 9 }); });
    expect(request).not.toHaveBeenCalled();

    // A workspaces-only advance pulls only workspaces.
    act(() => { result.current.setCatalogEpoch("E"); });
    request.mockResolvedValue({
      data: { workspaces: [], version: { epoch: "E", revision: 1 } },
    });
    await act(async () => {
      result.current.noteCatalogRevisions({ epoch: "E", workspaces: 1 });
    });
    expect(request).toHaveBeenCalledWith({ type: "list_workspaces" }, "list");
    expect(request).not.toHaveBeenCalledWith({ type: "list_sessions" }, "list");
  });

  test("a revision beacon with nothing to compare against is ignored", () => {
    render();
    act(() => { result.current.noteCatalogRevisions(undefined); });
    // An absent beacon must not be dereferenced (a TypeError here would take
    // the presence handler down with it) and must not trigger a pull.
    expect(request).not.toHaveBeenCalled();
  });

  test("catalogue refreshes without a client are no-ops", async () => {
    render();
    clientRef.current = null;
    await act(async () => {
      await result.current.refreshSessions();
      await result.current.refreshSettings();
      await result.current.refreshWorkspaces();
      await result.current.refreshModels();
    });
    // Nothing to read from: no request may be sent on a torn-down coupling.
    expect(request).not.toHaveBeenCalled();
  });

  test("a live terminal event with an unusable payload is not announced", () => {
    render();
    const base = { runId: "r1", type: "agent_end" } as never;
    act(() => {
      // No run to attribute it to, and a type that carries no completion.
      result.current.observeRunEvent({ type: "agent_start" } as never, "s1");
      result.current.observeRunEvent({ ...(base as object), type: "message" } as never, "s1");
      result.current.observeRunEvent({ ...(base as object), runId: undefined } as never, "s1");
      result.current.observeRunEvent({ ...(base as object), data: "{" } as never, "s1");
      result.current.observeRunEvent({ ...(base as object), data: "[]" } as never, "s1");
    });
    // A completion whose payload cannot be read (truncated JSON, an array where
    // an object is promised) must not be reported as a finished run.
    expect(onFinished).not.toHaveBeenCalled();
  });

  test("the notified-run ledger stays bounded without losing recent deduplication", () => {
    render();
    const finish = (runId: string) => act(() => {
      result.current.observeRunEvent({ runId, type: "agent_end", data: "{}" } as never, "s1");
    });
    for (let index = 0; index < 513; index += 1) finish(`run-${index}`);
    expect(onFinished).toHaveBeenCalledTimes(513);
    // The oldest entries are dropped to keep the ledger bounded, but the run
    // that just finished is still remembered: a repeated terminal frame must not
    // announce the same completion twice.
    finish("run-512");
    expect(onFinished).toHaveBeenCalledTimes(513);
  });

  test("a model recovery timer that outlives the catalogue is cleared on unmount", async () => {
    jest.useFakeTimers();
    try {
      render();
      request.mockResolvedValue({ data: { models: [] } });
      await act(async () => { await result.current.refreshModels(); });
      expect(request).toHaveBeenCalledTimes(1);
      await act(async () => { renderer!.unmount(); renderer = null; });
      await act(async () => { await jest.runAllTimersAsync(); });
      // The warm-up retries belong to a catalogue that no longer exists.
      expect(request).toHaveBeenCalledTimes(1);
    } finally {
      jest.useRealTimers();
    }
  });

  test("a newer model refresh abandons the pending warm-up retry", async () => {
    jest.useFakeTimers();
    try {
      render();
      request.mockResolvedValue({ data: { models: [] } });
      await act(async () => { await result.current.refreshModels(); });
      expect(request).toHaveBeenCalledTimes(1);
      // A reconnect asks again before the first retry fires.
      await act(async () => { await result.current.refreshModels(); });
      expect(request).toHaveBeenCalledTimes(2);
      await act(async () => { await jest.runAllTimersAsync(); });
      // The superseded timer was cancelled: only the newer generation's retries
      // may run, so the request count follows one recovery chain, not two.
      expect(request).toHaveBeenCalledTimes(6);
    } finally {
      jest.useRealTimers();
    }
  });

  test("a warm-up retry whose epoch was replaced or client removed does nothing", async () => {
    jest.useFakeTimers();
    try {
      render();
      request.mockResolvedValue({ data: { models: [] } });
      await act(async () => { await result.current.refreshModels(); });
      expect(request).toHaveBeenCalledTimes(1);
      // A new authenticated epoch invalidates the catalogue generation.
      act(() => { result.current.setCatalogEpoch("E2"); });
      await act(async () => { await jest.advanceTimersByTimeAsync(5_000); });
      expect(request).toHaveBeenCalledTimes(1);

      await act(async () => { await result.current.refreshModels(); });
      expect(request).toHaveBeenCalledTimes(2);
      clientRef.current = null;
      await act(async () => { await jest.advanceTimersByTimeAsync(5_000); });
      expect(request).toHaveBeenCalledTimes(2);
    } finally {
      jest.useRealTimers();
    }
  });

  test("reset cancels a pending model warm-up", async () => {
    jest.useFakeTimers();
    try {
      render();
      request.mockResolvedValue({ data: { models: [] } });
      await act(async () => { await result.current.refreshModels(); });
      act(() => { result.current.reset(); });
      await act(async () => { await jest.runAllTimersAsync(); });
      // An unpaired catalogue must not keep probing the desktop it just left.
      expect(request).toHaveBeenCalledTimes(1);
    } finally {
      jest.useRealTimers();
    }
  });

  test("title generation refuses to run without a connection or a session", async () => {
    render();
    clientRef.current = null;
    await expect(result.current.generateTitle("s1", "en")).rejects.toThrow("not_connected");
    clientRef.current = { request, requestRetry: request } as unknown as RemoteClient;
    await expect(result.current.generateTitle("", "en")).rejects.toThrow("not_connected");
    expect(request).not.toHaveBeenCalled();
  });

  test("title generation refuses a reply from a desktop generation that was replaced", async () => {
    render();
    let finish!: (value: unknown) => void;
    request.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const pending = result.current.generateTitle("s1", "en");
    // The desktop re-authenticated while the title was being written.
    act(() => { result.current.setCatalogEpoch("E2"); });
    finish({ success: true, data: { title: "Renamed" } });
    await expect(pending).rejects.toThrow("connection_changed");
  });

  test("title generation refuses an empty or refused answer", async () => {
    render();
    request.mockResolvedValueOnce({ success: false, error: "title_unavailable" });
    await expect(result.current.generateTitle("s1", "en")).rejects.toThrow("title_unavailable");
    request.mockResolvedValueOnce({ success: true, data: { title: "   " } });
    // A blank title is not a title: accepting it would rename the session to
    // nothing.
    await expect(result.current.generateTitle("s1", "en")).rejects.toThrow("title_generation_failed");
  });

  test("a rename acknowledged by a replaced desktop generation is dropped", async () => {
    render();
    let finish!: (value: unknown) => void;
    request.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const pending = result.current.rename("s1", "Renamed");
    act(() => { result.current.setCatalogEpoch("E2"); });
    finish({ data: {} });
    await act(async () => { await pending; });
    // The overlay and the local title belong to the pairing that asked for the
    // rename, not to whatever desktop is connected now.
    expect(result.current.titleOverrides).toEqual({});
    expect(result.current.sessions.some(item => item.title === "Renamed")).toBe(false);
  });

  test("a deletion acknowledged by a replaced desktop generation keeps the session", async () => {
    render();
    await act(async () => {
      request.mockResolvedValue({ data: { sessions: [session("s1")] } });
      await result.current.refreshSessions();
    });
    let finish!: (value: unknown) => void;
    request.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const pending = result.current.deleteSession("s1", "thread-s1");
    act(() => { result.current.setCatalogEpoch("E2"); });
    finish({ data: {} });
    // `false` means the conversation must stay open: the delete was answered by
    // a desktop generation this screen no longer shows.
    await expect(pending).resolves.toBe(false);
    expect(result.current.sessions.map(item => item.sessionId)).toEqual(["s1"]);
  });

  test("a workspace deletion acknowledged by a replaced desktop generation keeps the catalogue", async () => {
    render();
    await act(async () => {
      request.mockResolvedValue({
        data: { sessions: [{ ...session("s1"), workspaceId: "w1" }] },
      });
      await result.current.refreshSessions();
    });
    let finish!: (value: unknown) => void;
    request.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const pending = result.current.deleteWorkspace("w1");
    act(() => { result.current.setCatalogEpoch("E2"); });
    finish({ data: {} });
    // `false` keeps the caller's conversation open and leaves the local
    // catalogue intact: the cascade was answered by a desktop this screen no
    // longer shows.
    await expect(pending).resolves.toBe(false);
    expect(result.current.sessions.map(item => item.sessionId)).toEqual(["s1"]);
  });

  test("a pin acknowledged by a replaced desktop generation does not reorder", async () => {
    render();
    await act(async () => {
      request.mockResolvedValue({ data: { sessions: [session("s1"), session("s2")] } });
      await result.current.refreshSessions();
    });
    const before = result.current.sessions.map(item => item.sessionId);
    let finish!: (value: unknown) => void;
    request.mockReturnValueOnce(new Promise(resolve => { finish = resolve; }));
    const pending = result.current.setSessionPinned("s2", "thread-s2", true);
    act(() => { result.current.setCatalogEpoch("E2"); });
    finish({ data: {} });
    await act(async () => { await pending; });
    expect(result.current.sessions.map(item => item.sessionId)).toEqual(before);
  });
});
