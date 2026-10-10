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

const { precedingUserEntryId, useRemoteTimeline } = await import("./useRemoteTimeline");

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

/**
 * `streaming` drives the composer's Stop-vs-Send, so a wrong value puts a wrong
 * affordance in front of the user: "Stop" that stops nothing, or a Send that the
 * host refuses because it is already answering.
 */
describe("useRemoteTimeline run state", () => {
  // `sessionId` is required: the timeline ignores another session's events, so
  // an event without one would never reach the reducer and these assertions
  // would pass without exercising anything.
  const event = (type: string, data: unknown = {}) => ({
    sessionId: "sess_1",
    eventId: `e_${type}`,
    timestamp: "2026-01-01T00:00:00Z",
    type,
    data: JSON.stringify(data),
  });

  async function openTimeline() {
    requestMock.mockResolvedValue(page([]));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    return hook;
  }

  it("runs from the first activity frame to the settling one", async () => {
    const hook = await openTimeline();
    expect(hook.current.streaming).toBe(false);

    push(event("agent_start"));
    expect(hook.current.streaming).toBe(true);

    push(event("text_chunk", { text: "hi" }));
    expect(hook.current.streaming).toBe(true);

    push(event("run_finished"));
    expect(hook.current.streaming).toBe(false);
    hook.unmount();
  });

  /**
   * The defect this replaced: `!isTerminal(type)` counted *every* unrecognized
   * type as "a run just started", so a compaction left the conversation looking
   * busy and the composer offering Stop for a run that did not exist. A type
   * from another family must leave the run state exactly as it was.
   */
  it("does not mistake another family of event for a running agent", async () => {
    const hook = await openTimeline();

    for (const type of [
      "compaction_started",
      "compaction_committed",
      "compaction_failed",
      "compaction_unchanged",
      "settings_changed",
      "presence",
      "future_event_this_client_has_never_seen",
    ]) {
      push(event(type));
      expect(hook.current.streaming).toBe(false);
    }
    hook.unmount();
  });

  /** The same, in the other direction: a mid-run compaction does not end the run. */
  it("keeps a run running when an unrelated event arrives during it", async () => {
    const hook = await openTimeline();

    push(event("agent_start"));
    push(event("compaction_started"));
    expect(hook.current.streaming).toBe(true);

    push(event("compaction_committed"));
    expect(hook.current.streaming).toBe(true);

    push(event("agent_end"));
    expect(hook.current.streaming).toBe(false);
    hook.unmount();
  });
});

/**
 * `persistedEntryIds` is what separates an id the host can resolve in its store
 * from one that only names a frame on the wire — the difference between a fork
 * that branches and one that is refused.
 */
describe("useRemoteTimeline persisted entry ids", () => {
  it("counts history entries as persisted and a live push as not", async () => {
    requestMock.mockResolvedValue(page([entry("e1", "user", "hi")]));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    expect([...hook.current.persistedEntryIds]).toEqual(["e1"]);

    push({
      sessionId: "sess_1",
      type: "agent_text",
      // The shape the host actually publishes: a run-scoped frame id, which
      // names nothing in its store.
      eventId: "s_1:r_1:1:7",
      timestamp: "2026-01-01T00:00:00Z",
      data: JSON.stringify({ text: "live" }),
    });
    await settle();

    expect(hook.current.entries.map(item => item.id)).toEqual(["e1", "s_1:r_1:1:7"]);
    expect(hook.current.persistedEntryIds.has("e1")).toBe(true);
    expect(hook.current.persistedEntryIds.has("s_1:r_1:1:7")).toBe(false);
    hook.unmount();
  });

  it("keeps an older page's ids as well as the newest page's", async () => {
    requestMock
      .mockResolvedValueOnce(page([entry("e3", "user", "newest")], { hasMore: true, nextOffset: 42 }))
      .mockResolvedValueOnce(page([entry("e1", "user", "oldest")], { hasMore: false, nextOffset: 0 }));

    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    await act(async () => {
      await hook.current.loadOlder();
    });

    expect([...hook.current.persistedEntryIds].sort()).toEqual(["e1", "e3"]);
    hook.unmount();
  });

  /**
   * Ids are per conversation. Carrying the previous session's set across would
   * let a fork address an entry the host stores under a different session.
   */
  it("forgets the previous conversation's ids when another is opened", async () => {
    let session = "sess_1";
    requestMock.mockResolvedValue(page([entry("e1", "user", "one")]));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", session, true));
    await settle();
    expect(hook.current.persistedEntryIds.has("e1")).toBe(true);

    requestMock.mockResolvedValue(page([entry("e2", "user", "two")]));
    session = "sess_2";
    hook.rerender();
    await settle();

    expect(hook.current.persistedEntryIds.has("e1")).toBe(false);
    expect(hook.current.persistedEntryIds.has("e2")).toBe(true);
    hook.unmount();
  });
});

/**
 * The fork point: a reply is not itself a stored turn, so a fork branches at the
 * user* entry that produced it — matching the desktop's own Fork button.
 */
describe("precedingUserEntryId", () => {
  const say = (id: string, role: "assistant" | "user") => entry(id, role, id);

  it("returns the user entry that produced the reply", () => {
    expect(precedingUserEntryId([say("u1", "user"), say("a1", "assistant")], "a1")).toBe("u1");
  });

  it("skips intervening assistant/tool rows to reach the turn's prompt", () => {
    const items = [say("u1", "user"), say("t1", "assistant"), say("a1", "assistant")];
    expect(precedingUserEntryId(items, "a1")).toBe("u1");
  });

  it("returns null when there is no preceding prompt to branch at", () => {
    expect(precedingUserEntryId([say("a1", "assistant")], "a1")).toBeNull();
    expect(precedingUserEntryId([], "a1")).toBeNull();
    expect(precedingUserEntryId([say("u1", "user")], "missing")).toBeNull();
  });
});
