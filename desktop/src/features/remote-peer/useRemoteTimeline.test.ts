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

const { forgetRemoteTimeline, precedingUserEntryId, useRemoteTimeline } = await import("./useRemoteTimeline");
const { fetchRemoteToolTarget, forgetRemoteToolTargets } = await import("./remoteToolTarget");

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
  // What was read for a conversation is kept for the window, so one test's
  // transcript would otherwise seed the next one's.
  for (const desktop of ["desktop_a", "desktop_b"]) {
    for (const session of ["sess_1", "sess_2", "sess_3"])
      forgetRemoteTimeline(desktop, session);
  }
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

  /**
   * The first read asks for the *tail*.
   *
   * A history read is backward: the host serves the exchanges that end at
   * `before`, and only that cursor can name the newest page. Omitting it is the
   * host's *other* read (a forward page from offset 0), so opening a
   * conversation showed its oldest entries — one question and the first frames
   * of its reply — and the page after it repeated those same ordinals, which is
   * the "load earlier" that showed the same question twice.
   */
  it("opens on the newest page, not the oldest", async () => {
    requestMock.mockResolvedValue(page([entry("e1", "user", "newest")]));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();

    const [, command] = requestMock.mock.calls[0]!;
    // `Number.MAX_SAFE_INTEGER`, the value the host documents as "the page at
    // the end of the session" (and the phone opens with).
    expect((command as { before?: number }).before).toBe(Number.MAX_SAFE_INTEGER);
    hook.unmount();
  });

  /**
   * An older page that overlaps what is already drawn adds nothing.
   *
   * The cursor is the host's, so an overlap is not supposed to happen — but a
   * host that re-serves a range (a restarted session, a cursor that did not
   * advance) would then draw the same question and reply a second time under
   * the "load earlier" control the reader just used.
   */
  it("does not prepend an older page's entries twice", async () => {
    requestMock
      .mockResolvedValueOnce(page([entry("e2", "user", "newest")], { hasMore: true, nextOffset: 42 }))
      .mockResolvedValueOnce(page([entry("e1", "user", "oldest"), entry("e2", "user", "newest")], { hasMore: false, nextOffset: 0 }));

    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    await act(async () => {
      await hook.current.loadOlder();
    });

    expect(hook.current.entries.map(item => item.id)).toEqual(["e1", "e2"]);
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
 * The lean feed, as the peer actually receives it.
 *
 * A host that was told `lean_events_v1` trims the event envelope to
 * `type`/`data`/`runId`/`idx`, so a live entry has to be built from what is
 * left: the record inside `data` (a `user_message` always carries its entry id
 * and the host's wall clock) and, when even that is absent, the run-scoped
 * frame id and the moment the push arrived. These payloads are the lean shape
 * exactly — `sessionId` is present because the peer restores it from the
 * subject before it reaches this hook.
 */
describe("useRemoteTimeline on a lean feed", () => {
  async function openTimeline() {
    requestMock.mockResolvedValue(page([]));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    return hook;
  }

  it("builds a live entry from the event's own record", async () => {
    const hook = await openTimeline();

    push({
      sessionId: "sess_1",
      type: "user_message",
      runId: "r_1",
      idx: 3,
      data: JSON.stringify({
        text: "hello",
        entry_id: "entry_1",
        run_id: "r_1",
        created_at_ms: 1_700_000_000_123,
      }),
    });
    await settle();

    expect(hook.current.entries).toHaveLength(1);
    const entry = hook.current.entries[0]!;
    // The host's own entry id, not a wire id: this is what a fork and the
    // history read both address the turn by.
    expect(entry.id).toBe("entry_1");
    expect(entry.createdAtMs).toBe(1_700_000_000_123);
    expect(entry.role).toBe("user");
    expect(entry.blocks).toEqual([{ kind: "text", text: "hello" }]);
    hook.unmount();
  });

  it("falls back to the run-scoped frame id and the arrival time", async () => {
    const hook = await openTimeline();

    const before = Date.now();
    push({
      sessionId: "sess_1",
      type: "user_message",
      runId: "r_2",
      idx: 7,
      data: JSON.stringify({ text: "no record" }),
    });
    await settle();

    expect(hook.current.entries).toHaveLength(1);
    const entry = hook.current.entries[0]!;
    expect(entry.id).toBe("r_2:7");
    // "Now" is the only honest time when the host sent none: arrival order is
    // the order it sent, and a missing timestamp must not drop the line.
    expect(entry.createdAtMs).toBeGreaterThanOrEqual(before);
    expect(entry.createdAtMs).toBeLessThanOrEqual(Date.now());
    hook.unmount();
  });

  /** A frame with neither identity is not something to draw under an invented id. */
  it("drops a frame with no identity at all", async () => {
    const hook = await openTimeline();

    push({ sessionId: "sess_1", type: "user_message", data: JSON.stringify({ text: "nameless" }) });
    await settle();

    expect(hook.current.entries).toEqual([]);
    hook.unmount();
  });

  /** The envelope still wins when a full-feed host sent it. */
  it("prefers the envelope's id and time when they are present", async () => {
    const hook = await openTimeline();

    push({
      sessionId: "sess_1",
      type: "user_message",
      eventId: "wire_1",
      timestamp: "2026-01-01T00:00:00Z",
      runId: "r_3",
      idx: 1,
      data: JSON.stringify({ text: "full feed", entry_id: "entry_3", created_at_ms: 1 }),
    });
    await settle();

    const entry = hook.current.entries[0]!;
    expect(entry.id).toBe("wire_1");
    expect(entry.createdAtMs).toBe(Date.parse("2026-01-01T00:00:00Z"));
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
 * `persistedEntryIds` is what separates an id the host can resolve in its store * from one that only names a frame on the wire — the difference between a fork
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

/**
 * `compacting` gates the composer, because a host that is compacting refuses a
 * prompt. It is derived from the host's own lifecycle events, not from the fact
 * that we asked — the command only acknowledges the request.
 */
describe("useRemoteTimeline compaction state", () => {
  const event = (type: string) => ({
    sessionId: "sess_1",
    eventId: `e_${type}`,
    timestamp: "2026-01-01T00:00:00Z",
    type,
    data: JSON.stringify({ operationId: "op_1" }),
  });

  async function openTimeline() {
    requestMock.mockResolvedValue(page([]));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    return hook;
  }

  it("tracks a compaction from its start to its commit", async () => {
    const hook = await openTimeline();
    expect(hook.current.compacting).toBe(false);

    push(event("compaction_started"));
    expect(hook.current.compacting).toBe(true);

    push(event("compaction_committed"));
    expect(hook.current.compacting).toBe(false);
    hook.unmount();
  });

  /** "Nothing worth compacting" is still the end of the operation. */
  it("treats every terminal compaction outcome as the end", async () => {
    for (const type of ["compaction_failed", "compaction_unchanged", "compaction_end"]) {
      const hook = await openTimeline();
      push(event("compaction_started"));
      expect(hook.current.compacting).toBe(true);

      push(event(type));
      expect(hook.current.compacting).toBe(false);
      hook.unmount();
    }
  });

  /**
   * A compaction must not leave the composer waiting forever: the terminal frame
   * can be lost over at-most-once NATS, and nothing can still be compacting once
   * a run has settled.
   */
  it("clears a compaction when a run settles", async () => {
    const hook = await openTimeline();
    push(event("compaction_started"));
    expect(hook.current.compacting).toBe(true);

    push(event("run_finished"));
    expect(hook.current.compacting).toBe(false);
    hook.unmount();
  });

  /** A compaction is not a run, so it must not switch the composer to Stop. */
  it("does not make a compaction look like a run", async () => {
    const hook = await openTimeline();
    push(event("compaction_started"));
    expect(hook.current.compacting).toBe(true);
    expect(hook.current.streaming).toBe(false);
    hook.unmount();
  });

  it("forgets a compaction when another conversation is opened", async () => {
    let session = "sess_1";
    requestMock.mockResolvedValue(page([]));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", session, true));
    await settle();
    push(event("compaction_started"));
    expect(hook.current.compacting).toBe(true);

    session = "sess_2";
    hook.rerender();
    await settle();
    expect(hook.current.compacting).toBe(false);
    hook.unmount();
  });
});

/**
 * A link that comes back is the one thing the live stream cannot repair on its
 * own: everything the host pushed while it was down is gone (NATS Core is
 * at-most-once), and the transcript already on screen is exactly what is now
 * stale.
 */
describe("useRemoteTimeline resume", () => {
  const KIND_RESUMED = "resumed";

  /** A push of the given kind, from the host the timeline is watching. */
  function pushKind(kind: string, desktopId = "desktop_a"): void {
    act(() => {
      listeners.forEach(listener => listener({ payload: { desktopId, kind, payload: {} } }));
    });
  }

  it("re-reads the transcript when its host's link comes back", async () => {
    requestMock.mockResolvedValue(page([entry("e1", "user", "old line")]));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    expect(requestMock).toHaveBeenCalledTimes(1);

    // The host answered while the link was down: this is the history the client
    // would otherwise never show.
    requestMock.mockResolvedValue(page([
      entry("e1", "user", "old line"),
      entry("e2", "assistant", "sent while we were away"),
    ]));
    pushKind(KIND_RESUMED);
    await settle();

    expect(requestMock).toHaveBeenCalledTimes(2);
    expect(hook.current.entries.map(item => item.id)).toEqual(["e1", "e2"]);
    hook.unmount();
  });

  it("ignores another host's resume", async () => {
    requestMock.mockResolvedValue(page([]));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    requestMock.mockClear();

    pushKind(KIND_RESUMED, "desktop_b");
    await settle();

    // Another machine coming back says nothing about this conversation, and
    // re-reading on it would turn N hosts into N request storms.
    expect(requestMock).not.toHaveBeenCalled();
    hook.unmount();
  });

  it("does not ask a host that is not being watched", async () => {
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", false));
    await settle();
    pushKind(KIND_RESUMED);
    await settle();
    expect(requestMock).not.toHaveBeenCalled();
    hook.unmount();
  });
});

/**
 * A remote run's reply is drawn as it is written.
 *
 * The host publishes its agent's events the moment they happen — the same
 * vocabulary a local run produces — and the transcript folds them into the reply
 * bubble. Without this a prompt sent from this window was followed by silence
 * until the run ended, and the answer appeared whole, seconds later.
 */
describe("useRemoteTimeline live reply", () => {
  const runId = "run_live";
  /** One pushed event of the run, in the host's own body shape. */
  const event = (type: string, data: unknown, idx: number) => push({
    sessionId: "sess_1",
    runId,
    idx,
    eventId: `sess_1:${runId}:${idx}`,
    timestamp: "2026-01-01T00:00:00Z",
    type,
    data: JSON.stringify(data),
  });

  async function open(text: string) {
    requestMock.mockResolvedValue(page([entry("e1", "user", text)]));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    return hook;
  }

  it("draws the reply as its text arrives, and clears it when the run ends", async () => {
    const hook = await open("hello host");
    expect(hook.current.live).toBeNull();

    event("agent_start", {}, 0);
    await settle();
    // Nothing to draw yet: an empty bubble with an author line reads as a broken
    // reply, so the transcript waits for the first word.
    expect(hook.current.live).toBeNull();

    event("text_chunk", { text: "Wor" }, 1);
    event("text_chunk", { text: "king" }, 2);
    await settle();
    expect(hook.current.live?.content).toBe("Working");
    expect(hook.current.live?.status).toBe("streaming");
    expect(hook.current.live?.runId).toBe(runId);
    // The settled transcript is still the history it was: the live reply is drawn
    // *after* it, not folded into it.
    expect(hook.current.messages.map(message => message.content)).toEqual(["hello host"]);

    // The run settles: the live bubble steps aside for the record, which is read
    // back from the host (the journal has the reply now).
    requestMock.mockResolvedValue(page([
      entry("e1", "user", "hello host"),
      entry("e2", "assistant", "Working"),
    ]));
    event("agent_end", {}, 3);
    await settle();
    expect(hook.current.live).toBeNull();
    expect(hook.current.messages.map(message => message.content)).toEqual(["hello host", "Working"]);
    hook.unmount();
  });

  /**
   * A settled run's projection is dropped, so the next run's reply is only its own.
   *
   * The projection accumulates deltas; keeping it would prefix the finished reply
   * to the next one.
   */
  it("does not carry one run's text into the next", async () => {
    const hook = await open("first");
    event("text_chunk", { text: "answer one" }, 1);
    await settle();
    expect(hook.current.live?.content).toBe("answer one");

    event("agent_end", {}, 2);
    await settle();

    push({
      sessionId: "sess_1",
      runId: "run_two",
      idx: 0,
      eventId: "sess_1:run_two:0",
      timestamp: "2026-01-01T00:00:10Z",
      type: "text_chunk",
      data: JSON.stringify({ text: "answer two" }),
    });
    await settle();
    expect(hook.current.live?.content).toBe("answer two");
    expect(hook.current.live?.id).toBe("stream_run_two");
    hook.unmount();
  });

  /** Another conversation's reply is never drawn under this one's title. */
  it("ignores another session's reply", async () => {
    const hook = await open("hello");
    push({
      sessionId: "sess_other",
      runId,
      idx: 0,
      eventId: "x",
      timestamp: "2026-01-01T00:00:00Z",
      type: "text_chunk",
      data: JSON.stringify({ text: "not mine" }),
    });
    await settle();
    expect(hook.current.live).toBeNull();
    hook.unmount();
  });

  /** A settings change is not a reply: it must not open an empty bubble. */
  it("does not treat a session-level event as a reply", async () => {
    const hook = await open("hello");
    push({ sessionId: "sess_1", type: "model_changed", data: JSON.stringify({ model: "m" }) });
    push({ sessionId: "sess_1", type: "user_message", eventId: "u", timestamp: "2026-01-01T00:00:00Z", data: JSON.stringify({ text: "hello" }) });
    await settle();
    expect(hook.current.live).toBeNull();
    hook.unmount();
  });
});

/**
 * The `live` reply is gone when the conversation changes.
 *
 * The bubble is a projection of *that* run's deltas: drawn under another
 * conversation's title it would be a reply to a question the reader cannot see.
 */
it("drops the live reply when another conversation is opened", async () => {
  requestMock.mockResolvedValue(page([entry("e1", "user", "first")]));
  let session = "sess_1";
  const hook = renderHook(() => useRemoteTimeline("desktop_a", session, true));
  await settle();
  push({
    sessionId: "sess_1",
    runId: "run_1",
    idx: 0,
    eventId: "e",
    timestamp: "2026-01-01T00:00:00Z",
    type: "text_chunk",
    data: JSON.stringify({ text: "answering" }),
  });
  await settle();
  expect(hook.current.live?.content).toBe("answering");

  session = "sess_2";
  hook.rerender();
  await settle();
  expect(hook.current.live).toBeNull();
  hook.unmount();
});

/**
 * A conversation's fetched tool arguments are dropped when it is left.
 *
 * The cache is keyed by run and call id, which never repeat, so nothing in it
 * would ever be read again — leaving it in place would grow for the life of the
 * window. What makes the dropping correct rather than merely tidy is that the
 * next open re-reads: the assertion is a second request, not a smaller map.
 */
describe("tool-argument cache lifetime", () => {
  it("re-reads a conversation's tool arguments after it was left and reopened", async () => {
    forgetRemoteToolTargets("desktop_a", "sess_1");
    requestMock.mockResolvedValue({ name: "shell", arguments: { command: "pwd" } });
    const call = { desktopId: "desktop_a", runId: "run_1", sessionId: "sess_1", toolCallId: "call_1" };

    await expect(fetchRemoteToolTarget(call)).resolves.toBe("pwd");
    await expect(fetchRemoteToolTarget(call)).resolves.toBe("pwd");
    expect(requestMock).toHaveBeenCalledTimes(1);

    // Opening and leaving the conversation is what drops it.
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    hook.unmount();

    await expect(fetchRemoteToolTarget(call)).resolves.toBe("pwd");
    expect(requestMock).toHaveBeenCalledTimes(3);
  });
});

/**
 * Three user exchanges per page, not a hundred.
 *
 * The client asked the host for a hundred and the host answered with the whole
 * conversation, so every open paid for history nobody had asked to see — the
 * difference between a remote conversation appearing and appearing seconds
 * later. The phone asks for three; this asks for the same, and older pages
 * arrive as the reader scrolls up.
 */
it("asks the host for three exchanges, not the whole conversation", async () => {
  requestMock.mockResolvedValue(page([]));
  const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
  await settle();

  const [, command] = requestMock.mock.calls[0]!;
  expect((command as { limit?: number }).limit).toBe(3);
  hook.unmount();
});

/**
 * The page size and the tail cursor are one contract with the phone's client.
 *
 * `mobile/src/remote/timeline/historyPaging.ts` holds the same two constants and
 * this file's are asserted against the phone's values, because the two clients
 * diverged once (100 exchanges against 3) precisely because neither could see the
 * other's copy.
 */
it("opens with the phone's page size and tail cursor", async () => {
  requestMock.mockResolvedValue(page([]));
  const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
  await settle();

  const [, command] = requestMock.mock.calls[0]!;
  expect((command as { limit?: number }).limit).toBe(3);
  expect((command as { before?: number }).before).toBe(Number.MAX_SAFE_INTEGER);
  hook.unmount();
});

/**
 * Reopening a conversation shows what was read, then catches up.
 *
 * Switching between two conversations is constant, and re-reading from nothing
 * made the second visit cost what the first did: a blank column, then the
 * transcript. The read still happens — a push missed while the window was
 * elsewhere never arrives (NATS Core is at-most-once) — but it no longer decides
 * whether anything is on screen.
 */
it("shows a conversation's last read state on reopen, and re-reads behind it", async () => {
  requestMock.mockResolvedValue(page([entry("e1", "user", "hello")]));
  const first = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
  await settle();
  expect(first.current.entries.map(item => item.id)).toEqual(["e1"]);
  first.unmount();

  // A host that has since moved on.
  requestMock.mockResolvedValue(page([
    entry("e1", "user", "hello"),
    entry("e2", "assistant", "and this is new"),
  ]));
  const second = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));

  // Before the reply lands, the previous read is already on screen.
  expect(second.current.entries.map(item => item.id)).toEqual(["e1"]);
  expect(second.current.loading).toBe(false);

  await settle();
  expect(second.current.entries.map(item => item.id)).toEqual(["e1", "e2"]);
  second.unmount();
});

/** One conversation's read state is not another's. */
it("keeps each conversation's read state to itself", async () => {
  requestMock.mockResolvedValue(page([entry("e1", "user", "first conversation")]));
  const first = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
  await settle();
  first.unmount();

  requestMock.mockResolvedValue(page([entry("e9", "user", "another conversation")]));
  const other = renderHook(() => useRemoteTimeline("desktop_a", "sess_2", true));
  await settle();

  expect(other.current.entries.map(item => item.id)).toEqual(["e9"]);
  other.unmount();
});
