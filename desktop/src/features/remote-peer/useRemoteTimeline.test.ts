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
/**
 * What the host says about the conversation when the hook asks (`get_state`).
 *
 * A separate seam from `requestMock` on purpose: the state read is part of the
 * open* of every conversation while the page reads are what each test is about,
 * and a single mock would have the state reply consume the page a test queued.
 * `{}` — no run in flight, not compacting — is what a quiet conversation answers.
 */
let stateReply: unknown = {};
vi.mock("./remotePeerClient", () => ({
  requestRemotePeer: async (desktopId: string, command: { type?: string }, lane: string) =>
    (command?.type === "get_state" ? stateReply : requestMock(desktopId, command, lane)),
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

function entry(
  id: string,
  role: "user" | "assistant",
  text: string,
  run?: { runId: string; status: string },
): SessionEntry {
  return {
    id,
    kind: "message",
    role,
    createdAtMs: 1_000,
    blocks: [{ kind: "text", text }],
    ...(run ? { runId: run.runId, run: { status: run.status } } : {}),
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
  stateReply = {};
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

  /**
   * After a link drop the host publishes `resumed`.  The timeline should try an
   * incremental replay first; if the host supports it and the replay is valid,
   * only the missed events are fetched.
   */
  it("re-reads the record and the run in flight when a link comes back", async () => {
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    stateReply = { activeRun: { runId: "r1" } };

    // A run whose log this client has never read: the first frame arrives above
    // zero, which is the shape of a run that started before this window did.
    push({
      sessionId: "sess_1",
      type: "text_chunk",
      runId: "r1",
      idx: 7,
      eventId: "r1:7",
      timestamp: "2026-01-01T00:00:00Z",
      data: JSON.stringify({ text: "mid-reply" }),
    });
    await settle();
    requestMock.mockClear();

    requestMock.mockResolvedValue(page([entry("e1", "user", "hi")]));
    act(() => {
      listeners.forEach(listener => listener({ payload: { desktopId: "desktop_a", kind: "resumed", payload: {} } }));
    });
    await settle();

    // The record is re-read (a run that started *and* finished while the link was
    // down is only there) …
    expect(requestMock).toHaveBeenCalledWith(
      "desktop_a",
      expect.objectContaining({ type: "get_session_entries" }),
      "sess_1",
    );
    // … and so is the run in flight, from its beginning: its prefix was never
    // proven, so there is nothing to fill in from.
    expect(requestMock).toHaveBeenCalledWith(
      "desktop_a",
      expect.objectContaining({ type: "get_events_since", sinceIdx: -1, limit: 1000 }),
      "sess_1",
    );
    hook.unmount();
  });

  /**
   * An older host refuses the command, and is not asked again.
   *
   * The reply is then drawn the way it was before run logs could be read: the
   * record's beginning plus the pushed tail, folded into one row. Asking on every
   * frame would be a request per delta for an answer that will never come.
   */
  it("stops asking an older host, and keeps reading its record", async () => {
    requestMock.mockResolvedValue(page([]));
    stateReply = { activeRun: { runId: "r1" } };
    requestMock.mockRejectedValue(new Error("Unsupported command: get_events_since"));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    expect(hook.current.replayUnavailable).toBe(true);

    const attempts = requestMock.mock.calls.filter(([, command]) =>
      (command as { type?: string }).type === "get_events_since").length;
    expect(attempts).toBe(1);

    // A frame of that run arrives, and the transcript is still re-read on a
    // settle — but the log is not asked for again.
    requestMock.mockResolvedValue(page([entry("e1", "user", "hi")]));
    push({
      sessionId: "sess_1",
      type: "text_chunk",
      runId: "r1",
      idx: 1,
      eventId: "r1:1",
      timestamp: "2026-01-01T00:00:00Z",
      data: JSON.stringify({ text: "hi" }),
    });
    push({ sessionId: "sess_1", type: "agent_end", runId: "r1", idx: 2, eventId: "r1:2", timestamp: "2026-01-01T00:00:01Z", data: "{}" });
    await settle();

    expect(requestMock.mock.calls.filter(([, command]) =>
      (command as { type?: string }).type === "get_events_since")).toHaveLength(1);
    expect(requestMock).toHaveBeenCalledWith(
      "desktop_a",
      expect.objectContaining({ type: "get_session_entries" }),
      "sess_1",
    );
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
 * A remote run's reply, and where the client gets it from.
 *
 * A host publishes its agent's events the moment they happen — the same
 * vocabulary a local run produces — so a prompt sent from this window is followed
 * by the answer as it is written rather than by silence. What the *pushes* cannot
 * carry is everything that happened before this window subscribed: a reply it
 * joined mid-flight. That is what the run's own log is read for, and the two are
 * what these tests separate.
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
  /** A read of the run's log, as the host answers `get_events_since`. */
  function logResponse(events: Array<{ type: string; data?: unknown; idx: number }>, extra: Record<string, unknown> = {}) {
    return {
      events: events.map(item => ({ ...item, data: JSON.stringify(item.data ?? {}), runId })),
      watermark: events.length > 0 ? events[events.length - 1]!.idx : -1,
      ...extra,
    };
  }

  async function open(text: string) {
    requestMock.mockResolvedValue(page([entry("e1", "user", text)]));
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    return hook;
  }

  it("draws the reply as its text arrives, and drops its own fragment when the run ends", async () => {
    const hook = await open("hello host");
    expect(hook.current.liveReplies).toEqual([]);

    event("agent_start", {}, 0);
    await settle();
    // Nothing to draw yet: an empty bubble with an author line reads as a broken
    // reply, so the transcript waits for the first word.
    expect(hook.current.liveReplies).toEqual([]);

    event("text_chunk", { text: "Wor" }, 1);
    event("text_chunk", { text: "king" }, 2);
    await settle();
    expect(hook.current.liveReplies[0]!.message.content).toBe("Working");
    expect(hook.current.liveReplies[0]!.message.status).toBe("streaming");
    expect(hook.current.liveReplies[0]!.message.runId).toBe(runId);
    // Nothing was read from the run's log, so this reply is only the pushes: the
    // view folds it into the record's own copy of the turn.
    expect(hook.current.liveReplies[0]!.fromReplay).toBe(false);
    // The settled transcript is still the history it was: the reply is drawn
    // *after* it, not folded into it.
    expect(hook.current.messages.map(message => message.content)).toEqual(["hello host"]);

    // The run ends. This reply was built from pushes alone — its beginning is in
    // the record, not in it — so the fragment is dropped as the record's copy
    // takes over, rather than being drawn beside it.
    requestMock.mockResolvedValue(page([
      entry("e1", "user", "hello host"),
      entry("e2", "assistant", "Working", { runId, status: "completed" }),
    ]));
    event("agent_end", { duration_ms: 1_500 }, 3);
    await settle();
    expect(hook.current.liveReplies).toEqual([]);
    expect(hook.current.messages.map(message => message.content)).toEqual(["hello host", "Working"]);
    hook.unmount();
  });

  /**
   * A reply rebuilt from the run's log survives the run's end.
   *
   * The host's record of a run trails its terminal frame — the agent writes the
   * last model call as the run ends — so a client that dropped its copy the moment
   * the run settled would blink the answer out of the transcript. It is drawn
   * until the record can be read back, and it is a *whole* reply, so drawing it
   * beside that record is not a duplication either.
   */
  it("keeps drawing a rebuilt reply until the host's record can be read", async () => {
    stateReply = { activeRun: { runId } };
    requestMock.mockImplementation(async (_desktop: string, command: { type?: string }) => {
      if (command?.type === "get_events_since")
        return logResponse([
          { type: "agent_start", data: { started_at_ms: 1_700_000_000_000 }, idx: 0 },
          { type: "text_chunk", data: { text: "the answer" }, idx: 1 },
        ]);
      // The record does not carry the run yet: the catch-up read finds the page
      // unchanged.
      return page([entry("e1", "user", "hello host")]);
    });
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    expect(hook.current.liveReplies[0]!.fromReplay).toBe(true);

    event("agent_end", { duration_ms: 1_500 }, 2);
    await settle();

    expect(hook.current.liveReplies).toHaveLength(1);
    expect(hook.current.liveReplies[0]!.message.content).toBe("the answer");
    expect(hook.current.liveReplies[0]!.message.status).toBe("complete");
    hook.unmount();
  });

  /**
   * A run this window joined in flight is rebuilt from its log.
   *
   * The pushes begin where the subscription did, so the reply's first half — and
   * the run's own start time — exist only in the log. The state read on open is
   * what tells the client there is a run to read before the next frame happens to
   * arrive (a long tool call pushes nothing for minutes).
   */
  it("rebuilds a run already in flight when the conversation is opened", async () => {
    const startedAt = Date.now() - 8 * 60_000;
    stateReply = { activeRun: { runId } };
    requestMock.mockImplementation(async (_desktop: string, command: { type?: string }) => {
      if (command?.type === "get_events_since") {
        return logResponse([
          { type: "agent_start", data: { started_at_ms: startedAt }, idx: 0 },
          { type: "text_chunk", data: { text: "half a reply" }, idx: 1 },
        ]);
      }
      return page([
        entry("e1", "user", "keep going"),
        entry("e2", "assistant", "half a reply", { runId, status: "running" }),
      ]);
    });

    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();

    const [reply] = hook.current.liveReplies;
    expect(reply!.fromReplay).toBe(true);
    expect(reply!.message.content).toBe("half a reply");
    // The run's own clock, not this window's arrival: the elapsed time a reader
    // sees on a reply that was already running.
    expect(reply!.message.runStartedAt).toBe(startedAt);
    // The record's own partial copy of the same run is hidden, so the reply is
    // drawn once rather than beside itself.
    expect(reply!.message.runId).toBe(runId);
    expect(requestMock.mock.calls.some(([, command]) =>
      (command as { type?: string }).type === "get_events_since")).toBe(true);
    hook.unmount();
  });

  /**
   * A settled run's reply is only its own words.
   *
   * The projection accumulates deltas, so a run kept across the next one would
   * prefix the finished answer to the new reply.
   */
  it("does not carry one run's text into the next", async () => {
    const hook = await open("first");
    event("text_chunk", { text: "answer one" }, 1);
    await settle();
    expect(hook.current.liveReplies[0]!.message.content).toBe("answer one");

    event("agent_end", {}, 2);
    await settle();
    // Its own fragment is gone — the record holds that reply now.
    expect(hook.current.liveReplies).toEqual([]);

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

    expect(hook.current.liveReplies.map(reply => reply.message.content)).toEqual(["answer two"]);
    expect(hook.current.liveReplies[0]!.message.id).toBe("stream_run_two");
    hook.unmount();
  });

  /**
   * A link that comes back re-establishes the run from its log.
   *
   * This is the one thing the pushes cannot repair: everything the host sent while
   * the link was down is gone, and the reply on screen is exactly what is now
   * stale. On this client's lane — the trimmed feed, which omits event families by
   * design — "how much of the run do I hold" cannot be answered by counting what
   * arrived, so the run is read from its beginning and the pushed copy is
   * replaced rather than added to.
   */
  it("rebuilds the reply in flight from the run's log after a link comes back", async () => {
    const hook = await open("hello host");
    event("agent_start", {}, 0);
    event("text_chunk", { text: "Wor" }, 1);
    await settle();
    expect(hook.current.liveReplies[0]!.message.content).toBe("Wor");

    stateReply = { activeRun: { runId } };
    requestMock.mockImplementation(async (_desktop: string, command: { type?: string }) => {
      if (command?.type === "get_events_since") {
        return logResponse([
          { type: "agent_start", data: { started_at_ms: 1_700_000_000_000 }, idx: 0 },
          { type: "text_chunk", data: { text: "Wor" }, idx: 1 },
          { type: "text_chunk", data: { text: "king" }, idx: 2 },
        ]);
      }
      return page([entry("e1", "user", "hello host")]);
    });
    act(() => {
      listeners.forEach(listener => listener({ payload: { desktopId: "desktop_a", kind: "resumed", payload: {} } }));
    });
    await settle();

    const read = requestMock.mock.calls
      .map(([, command]) => command as { type?: string; sinceIdx?: number })
      .find(command => command.type === "get_events_since");
    expect(read?.sinceIdx).toBe(-1);
    const [reply] = hook.current.liveReplies;
    expect(reply!.fromReplay).toBe(true);
    // The whole reply — the part the pushes built *and* the part they lost — once.
    expect(reply!.message.content).toBe("Working");
    hook.unmount();
  });

  /**
   * A jump in the indices, read two ways.
   *
   * A host that trims its feed omits whole event families, so a jump is expected
   * — and nothing is missing. A host that does not trim sends one event per index,
   * so the same jump is a frame this client never got, and the run's log is where
   * it comes back from.
   */
  it("reads the run's log when the pushes declare a gap, and not when the feed is trimmed", async () => {
    const hook = await open("hello");
    requestMock.mockImplementation(async (_desktop: string, command: { type?: string }) => {
      if (command?.type === "get_events_since") {
        // The host states its pre-trim count: its pushes omit indices by design.
        return logResponse([
          { type: "agent_start", data: {}, idx: 0 },
          { type: "text_chunk", data: { text: "one" }, idx: 1 },
          { type: "text_chunk", data: { text: "two" }, idx: 5 },
        ], { rawEvents: 6 });
      }
      return page([entry("e1", "user", "hello")]);
    });

    event("text_chunk", { text: "one" }, 1);
    await settle();
    // The first frame lands above zero of a run this client has no beginning for,
    // so the log is read once for it.
    expect(requestMock.mock.calls.filter(([, command]) =>
      (command as { type?: string }).type === "get_events_since")).toHaveLength(1);

    // A hole *inside* a run the host has told us it trims is not loss: no second
    // read.
    event("text_chunk", { text: "two" }, 5);
    await settle();
    expect(requestMock.mock.calls.filter(([, command]) =>
      (command as { type?: string }).type === "get_events_since")).toHaveLength(1);
    expect(hook.current.liveReplies[0]!.message.content).toBe("onetwo");
    hook.unmount();
  });

  /**
   * A gap on a feed that numbers every event is filled from where it stopped.
   *
   * There is nothing to re-establish — the client knows exactly what it is missing
   * — and the read says so by starting at the cursor, so a long run costs one page
   * rather than a whole reply.
   */
  it("fills a declared gap from the cursor instead of re-reading the run", async () => {
    stateReply = { activeRun: { runId } };
    // The run as the host's journal holds it, growing as it runs.
    const log = [
      { type: "agent_start", data: {}, idx: 0 },
      { type: "text_chunk", data: { text: "one" }, idx: 1 },
      { type: "text_chunk", data: { text: "two" }, idx: 2 },
      { type: "text_chunk", data: { text: "three" }, idx: 3 },
    ];
    let written = 2;
    requestMock.mockImplementation(async (_desktop: string, command: { type?: string } & Record<string, unknown>) => {
      if (command?.type === "get_events_since") {
        // A host that trims nothing: one event per index, no pre-trim count.
        const since = Number(command.sinceIdx);
        return logResponse(log.slice(0, written).slice(since + 1));
      }
      return page([entry("e1", "user", "hello")]);
    });
    const hook = renderHook(() => useRemoteTimeline("desktop_a", "sess_1", true));
    await settle();
    expect(hook.current.liveReplies[0]!.message.content).toBe("one");

    // A frame arrives two indices ahead of what this client holds: one frame was
    // lost, and the feed numbers every event, so the loss is visible.
    written = 4;
    event("text_chunk", { text: "three" }, 3);
    await settle();

    const reads = requestMock.mock.calls
      .map(([, command]) => command as { type?: string; sinceIdx?: number })
      .filter(command => command.type === "get_events_since");
    expect(reads.map(read => read.sinceIdx)).toEqual([-1, 1]);
    // The lost frame came back from the log, and the rest of the run was not
    // re-read to get it.
    expect(hook.current.liveReplies[0]!.message.content).toBe("onetwothree");
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
    expect(hook.current.liveReplies).toEqual([]);
    hook.unmount();
  });

  /** A settings change is not a reply: it must not open an empty bubble. */
  it("does not treat a session-level event as a reply", async () => {
    const hook = await open("hello");
    push({ sessionId: "sess_1", type: "model_changed", data: JSON.stringify({ model: "m" }) });
    push({ sessionId: "sess_1", type: "user_message", eventId: "u", timestamp: "2026-01-01T00:00:00Z", data: JSON.stringify({ text: "hello" }) });
    await settle();
    expect(hook.current.liveReplies).toEqual([]);
    hook.unmount();
  });
});

/**
 * The `live` reply is gone when the conversation changes.
 *
 * The bubble is a projection of *that* run's deltas: drawn under another
 * conversation's title it would be a reply to a question the reader cannot see.
 */
it("drops the conversation's replies when another one is opened", async () => {
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
  expect(hook.current.liveReplies[0]!.message.content).toBe("answering");

  session = "sess_2";
  hook.rerender();
  await settle();
  expect(hook.current.liveReplies).toEqual([]);
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
