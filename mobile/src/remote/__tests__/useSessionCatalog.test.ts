import React from "react";
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
  let result: { current: Catalog };
  let renderer: ReactTestRenderer | null;

  function TestComponent(): null {
    result.current = useSessionCatalog(
      clientRef as React.MutableRefObject<RemoteClient | null>,
      selectedRef as React.MutableRefObject<string>,
    );
    return null;
  }

  function render(): void {
    act(() => {
      renderer = create(React.createElement(TestComponent));
    });
  }

  beforeEach(() => {
    request = jest.fn();
    clientRef = { current: { request, requestRetry: request } as unknown as RemoteClient };
    selectedRef = { current: "s1" };
    result = { current: undefined as unknown as Catalog };
    renderer = null;
  });

  afterEach(() => {
    if (renderer) {
      act(() => renderer!.unmount());
      renderer = null;
    }
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
    act(() => result.current.applySessionSnapshot([session("s2", "running")]));
    expect(result.current.sessions[0]?.title).toBe("Title s2");

    // Rename s2 to install a title override (synced to titleOverridesRef via effect).
    request.mockResolvedValueOnce({ data: {} });
    await act(async () => {
      await result.current.rename("s2", "Custom Title");
    });

    // Second snapshot: s2 completed — uses the override and flags unread.
    act(() => result.current.applySessionSnapshot([session("s2", "completed")]));
    expect(result.current.sessions[0]).toMatchObject({ sessionId: "s2", title: "Custom Title" });
    expect(result.current.unreadSessions.has("s2")).toBe(true);
  });

  test("applySessionSnapshot leaves the selected session out of unread", () => {
    render();
    act(() => result.current.applySessionSnapshot([session("s1", "running")]));
    act(() => result.current.applySessionSnapshot([session("s1", "completed")]));
    expect(result.current.unreadSessions.has("s1")).toBe(false);
  });

  test("refreshSessions applies the pushed snapshot", async () => {
    render();
    request.mockResolvedValueOnce({ data: { sessions: [session("s2", "running")] } });
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
    request.mockResolvedValueOnce({ data: { models: [{ id: "m1", label: "M1" }] } });
    await act(async () => {
      await result.current.refreshModels();
    });
    expect(result.current.models).toEqual([{ id: "m1", label: "M1" }]);
    expect(request).toHaveBeenCalledTimes(1);
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
    request.mockResolvedValueOnce({ data: { sessions: [session("s2", "running")] } });
    await act(async () => {
      await result.current.refreshSessions();
    });
    expect(result.current.sessions).toHaveLength(1);
    act(() => result.current.reset());
    expect(result.current.sessions).toEqual([]);
    expect(result.current.workspaces).toEqual([]);
    expect(result.current.titleOverrides).toEqual({});
  });

  test("rename trims the name and updates both the override and the session", async () => {
    render();
    act(() =>
      result.current.applySessionSnapshot([session("s1", "running"), session("s2", "running")]),
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
      result.current.sessions.map(s => (s.sessionId === "s2" ? s.title : s.sessionId)),
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
    act(() =>
      result.current.applySessionSnapshot([session("s1", "running"), session("s2", "running")]),
    );
    request.mockResolvedValueOnce({ data: {} });
    let selected = false;
    await act(async () => {
      selected = await result.current.deleteSession("s1", "thread-s1");
    });
    expect(selected).toBe(true);
    expect(result.current.sessions.map(s => s.sessionId)).toEqual(["s2"]);
  });

  test("deleteSession returns false for a non-selected session", async () => {
    render();
    act(() => result.current.applySessionSnapshot([session("s2", "running")]));
    request.mockResolvedValueOnce({ data: {} });
    let selected = false;
    await act(async () => {
      selected = await result.current.deleteSession("s2", "thread-s2");
    });
    expect(selected).toBe(false);
  });

  test("deleteSession ignores empty ids", async () => {
    render();
    let selected = false;
    await act(async () => {
      selected = await result.current.deleteSession("", "");
    });
    expect(selected).toBe(false);
    expect(request).not.toHaveBeenCalled();
  });

  test("setSessionPinned reorders pinned sessions to the top", async () => {
    render();
    act(() =>
      result.current.applySessionSnapshot([session("a", "running"), session("b", "running")]),
    );
    request.mockResolvedValueOnce({ data: {} });
    await act(async () => {
      await result.current.setSessionPinned("b", "thread-b", true);
    });
    expect(result.current.sessions.map(s => s.sessionId)).toEqual(["b", "a"]);
  });

  test("setSessionPinned ignores empty ids", async () => {
    render();
    await act(async () => {
      await result.current.setSessionPinned("", "", true);
    });
    expect(request).not.toHaveBeenCalled();
  });
});
