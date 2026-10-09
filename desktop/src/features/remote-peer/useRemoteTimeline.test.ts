// @vitest-environment jsdom
import type { SessionEntry } from "@future-os/thread-projection";
import { act } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushAsync, renderHook } from "../../test/renderHook";

/**
 * The timeline hook's contract is about *when* it reads and what it refuses to
 * mix: history on open, only this session's pushes appended, and a previous
 * session's page never landing under the new session's title. Those are the
 * failures a user experiences as "my history is wrong", so they are what is
 * asserted.
 */

const requestMock = vi.fn();
vi.mock("./remotePeerClient", () => ({
  requestRemotePeer: (...args: unknown[]) => requestMock(...args),
}));

let listeners: Array<(event: { payload: unknown }) => void> = [];
vi.mock("@tauri-apps/api/event", () => ({
  listen: async (_name: string, handler: (event: { payload: unknown }) => void) => {
    listeners.push(handler);
    return () => {
      listeners = listeners.filter(item => item !== handler);
    };
  },
}));

const { useRemoteTimeline } = await import("./useRemoteTimeline");

function entry(id: string, role: "user" | "assistant", text: string): SessionEntry {
  return {
    id,
    kind: "message",
    role,
    createdAtMs: 1_000,
    blocks: [{ kind: "text", text }],
  };
}

function page(entries: unknown[], overrides: Record<string, unknown> = {}): unknown {
  return { entries, hasMore: false, nextOffset: 0, ...overrides };
}

function push(payload: Record<string, unknown>): void {
  act(() => {
    listeners.forEach(listener => listener({ payload: { desktopId: "desktop_a", kind: "event", payload } }));
  });
}

/** Wait until the hook has settled, with a bounded number of microtask turns. */
async function settle(): Promise<void> {
  for (let i = 0; i < 8; i += 1) await flushAsync();
}

beforeEach(() => {
  requestMock.mockReset();
  listeners = [];
});

describe("useRemoteTimeline", () => {
  it("reads history on open and exposes what it read", async () => {
    requestMock.mockResolvedValue(page([
      entry("e1", "user", "hello"),
      entry("e2", "assistant", "hi there"),
    ]));

    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();

    expect(hook.current.entries.map(item => item.id)).toEqual(["e1", "e2"]);
    expect(hook.current.entries[1]!.blocks[0]).toEqual({ kind: "text", text: "hi there" });
    expect(requestMock).toHaveBeenCalledWith(
      "desktop_a",
      expect.objectContaining({ type: "get_session_entries", sessionId: "sess_1" }),
      "sess_1",
    );
    hook.unmount();
  });

  /** Dropping malformed rows must not make a page look empty or throw. */
  it("keeps only renderable entries", async () => {
    requestMock.mockResolvedValue(page([
      entry("e1", "user", "ok"),
      { id: "", role: "user" },
      { role: "user" },
      { id: "e2", role: "nonsense" },
      null,
    ]));

    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    expect(hook.current.entries.map(item => item.id)).toEqual(["e1"]);
    hook.unmount();
  });

  /**
   * Older pages are asked for with the *host's* cursor (`before` = the previous
   * response's `nextOffset`). Anything else — an entry count, an index we
   * invented — silently returns the wrong slice.
   */
  it("pages older with the host's own cursor and appends in front", async () => {
    requestMock
      .mockResolvedValueOnce(page([entry("e3", "user", "newest")], { hasMore: true, nextOffset: 42 }))
      .mockResolvedValueOnce(page([entry("e1", "user", "oldest")], { hasMore: false, nextOffset: 0 }));

    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    expect(hook.current.hasMore).toBe(true);

    await act(async () => {
      await hook.current.loadOlder();
    });

    expect(requestMock).toHaveBeenLastCalledWith(
      "desktop_a",
      expect.objectContaining({ before: 42 }),
      "sess_1",
    );
    expect(hook.current.entries.map(item => item.id)).toEqual(["e1", "e3"]);
    expect(hook.current.hasMore).toBe(false);
    hook.unmount();
  });

  it("does not ask for an older page when the host says there is none", async () => {
    requestMock.mockResolvedValue(page([entry("e1", "user", "only")]));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    await act(async () => {
      await hook.current.loadOlder();
    });
    expect(requestMock).toHaveBeenCalledTimes(1);
    hook.unmount();
  });

  it("surfaces a read failure without inventing an empty conversation", async () => {
    requestMock.mockRejectedValue(new Error("peer_not_connected"));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    expect(hook.current.error).toBe("peer_not_connected");
    expect(hook.current.entries).toEqual([]);
    hook.unmount();
  });

  it("appends this session's pushes and ignores every other session's", async () => {
    requestMock.mockResolvedValue(page([entry("e1", "user", "hello")]));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();

    // Another host, another session, and a non-event push: none may append.
    act(() => {
      listeners.forEach(listener => listener({
        payload: {
          desktopId: "desktop_b",
          kind: "event",
          payload: { sessionId: "sess_1", type: "agent_text", eventId: "x", timestamp: "2026-01-01T00:00:00Z" },
        },
      }));
      listeners.forEach(listener => listener({
        payload: {
          desktopId: "desktop_a",
          kind: "event",
          payload: { sessionId: "sess_other", type: "agent_text", eventId: "y", timestamp: "2026-01-01T00:00:00Z" },
        },
      }));
      listeners.forEach(listener => listener({
        payload: { desktopId: "desktop_a", kind: "presence", payload: { online: true } },
      }));
    });
    await settle();
    expect(hook.current.entries.map(item => item.id)).toEqual(["e1"]);

    push({
      sessionId: "sess_1",
      type: "agent_text",
      eventId: "e2",
      timestamp: "2026-01-01T00:00:05Z",
      data: JSON.stringify({ text: "live line" }),
    });
    await settle();
    expect(hook.current.entries.map(item => item.id)).toEqual(["e1", "e2"]);
    expect(hook.current.entries[1]!.blocks[0]).toEqual({ kind: "text", text: "live line" });
    hook.unmount();
  });

  /**
   * A re-delivered event must not duplicate a line: NATS Core is at-most-once,
   * but a reconnect can replay, and a doubled message is immediately visible.
   */
  it("does not duplicate a re-delivered event", async () => {
    requestMock.mockResolvedValue(page([]));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();

    const once = {
      sessionId: "sess_1",
      type: "agent_text",
      eventId: "e1",
      timestamp: "2026-01-01T00:00:00Z",
      data: JSON.stringify({ text: "once" }),
    };
    push(once);
    await settle();
    push(once);
    await settle();
    expect(hook.current.entries).toHaveLength(1);
    hook.unmount();
  });

  it("reads nothing while disabled", async () => {
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", false));
    await settle();
    expect(requestMock).not.toHaveBeenCalled();
    expect(hook.current.entries).toEqual([]);
    hook.unmount();
  });
});
