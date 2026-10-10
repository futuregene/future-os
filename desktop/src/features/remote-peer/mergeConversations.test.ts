import type {
  DeviceFilter,
  LocalThreadLike,
  RemoteCatalog,
} from "./mergeConversations";
import { describe, expect, it } from "vitest";
import {
  compareConversations,
  hiddenCount,
  localTimestamp,
  mergeConversations,
  rowKey,
} from "./mergeConversations";

function thread(overrides: Partial<LocalThreadLike> & { id: string }): LocalThreadLike {
  return {
    title: overrides.id,
    mode: "chat",
    pinned: false,
    status: "active",
    workspaceId: "ws_local",
    updatedAt: 1_000,
    createdAt: 500,
    ...overrides,
  };
}

function catalog(desktopId: string, sessions: RemoteCatalog["sessions"]): RemoteCatalog {
  return { desktopId, sessions };
}

const ALL: DeviceFilter = { kind: "all" };

describe("mergeConversations", () => {
  it("interleaves local threads and remote sessions by time, not by source", () => {
    const merged = mergeConversations(
      [thread({ id: "local_old", updatedAt: 100 }), thread({ id: "local_new", updatedAt: 900 })],
      [catalog("desktop_a", [
        { sessionId: "a_mid", title: "a mid", lastMessageAt: 500 },
        { sessionId: "a_newest", title: "a newest", lastMessageAt: 1_000 },
      ])],
      ALL,
    );

    expect(merged.map(row => row.id)).toEqual(["a_newest", "local_new", "a_mid", "local_old"]);
  });

  /**
   * Two hosts and the local machine can each hold a session with the same id
   * (ids are unique per host, not globally). They must stay two rows, and each
   * must carry the host it belongs to — a row that lost its source would be
   * routed to the wrong machine.
   */
  it("keeps same-id sessions from different machines apart, each labelled", () => {
    const merged = mergeConversations(
      [thread({ id: "shared", updatedAt: 100 })],
      [
        catalog("desktop_a", [{ sessionId: "shared", title: "on a", lastMessageAt: 300 }]),
        catalog("desktop_b", [{ sessionId: "shared", title: "on b", lastMessageAt: 200 }]),
      ],
      ALL,
    );

    expect(merged).toHaveLength(3);
    expect(merged.map(row => row.desktopId)).toEqual(["desktop_a", "desktop_b", null]);
    expect(new Set(merged.map(row => row.key)).size).toBe(3);
    expect(merged[0]!.key).toBe(rowKey("desktop_a", "shared"));
  });

  it("ranks pinned rows above newer unpinned ones, from every source", () => {
    const merged = mergeConversations(
      [thread({ id: "local_pinned", pinned: true, updatedAt: 1 })],
      [catalog("desktop_a", [
        { sessionId: "a_old_pinned", title: "pinned", pinned: true, lastMessageAt: 10 },
        { sessionId: "a_fresh", title: "fresh", lastMessageAt: 10_000 },
      ])],
      ALL,
    );

    // Within the pinned group the same recency rule applies, so the pinned row
    // with the newer activity leads — pinned is a section, not a tie-breaker.
    expect(merged.map(row => row.id)).toEqual(["a_old_pinned", "local_pinned", "a_fresh"]);
  });

  /**
   * A host older than the `lastMessageAt` field sends sessions without one.
   * Those must sort *below* everything dated: treating unknown age as "now"
   * would float a stale conversation to the top of the list.
   */
  it("sorts rows without a timestamp last rather than treating them as current", () => {
    const merged = mergeConversations(
      [thread({ id: "local", updatedAt: 5 })],
      [catalog("desktop_a", [{ sessionId: "undated", title: "undated" }])],
      ALL,
    );

    expect(merged.map(row => row.id)).toEqual(["local", "undated"]);
    expect(merged[1]!.lastMessageAt).toBeNull();
  });

  it("is stable for equal timestamps so a poll cannot reshuffle the list", () => {
    const rows = mergeConversations(
      [],
      [catalog("desktop_b", [{ sessionId: "b", title: "b", lastMessageAt: 7 }]), catalog("desktop_a", [{ sessionId: "a", title: "a", lastMessageAt: 7 }])],
      ALL,
    );
    expect(rows.map(row => row.id)).toEqual(["a", "b"]);
    // And the comparator is a total order, so `sort` cannot fall back to
    // insertion order.
    expect(compareConversations(rows[0]!, rows[1]!)).toBeLessThan(0);
    expect(compareConversations(rows[1]!, rows[0]!)).toBeGreaterThan(0);
    expect(compareConversations(rows[0]!, rows[0]!)).toBe(0);
  });

  it("drops deleted local threads but keeps archived ones listed", () => {
    const merged = mergeConversations(
      [
        thread({ id: "gone", status: "deleted" }),
        thread({ id: "archived", status: "archived" }),
      ],
      [],
      ALL,
    );
    expect(merged.map(row => row.id)).toEqual(["archived"]);
  });

  it("de-duplicates a session that appears twice in one snapshot", () => {
    const merged = mergeConversations(
      [],
      [catalog("desktop_a", [
        { sessionId: "dup", title: "first", lastMessageAt: 1 },
        { sessionId: "dup", title: "second", lastMessageAt: 2 },
      ])],
      ALL,
    );
    expect(merged).toHaveLength(1);
    expect(merged[0]!.title).toBe("first");
  });

  it("filters to one machine without touching the others' rows", () => {
    const threads = [thread({ id: "local", updatedAt: 1 })];
    const catalogs = [
      catalog("desktop_a", [{ sessionId: "a", title: "a", lastMessageAt: 3 }]),
      catalog("desktop_b", [{ sessionId: "b", title: "b", lastMessageAt: 2 }]),
    ];

    expect(mergeConversations(threads, catalogs, { kind: "device", desktopId: "desktop_a" })
      .map(row => row.id)).toEqual(["a"]);
    expect(mergeConversations(threads, catalogs, { kind: "device", desktopId: null })
      .map(row => row.id)).toEqual(["local"]);
  });

  /** A disconnected host has no catalog; its rows must vanish, not go stale. */
  it("omits a host entirely when it has no snapshot", () => {
    const merged = mergeConversations([], [catalog("desktop_a", [])], ALL);
    expect(merged).toEqual([]);
    expect(hiddenCount([thread({ id: "local" })], [], { kind: "device", desktopId: "desktop_a" }))
      .toBe(1);
  });

  it("reports how many conversations a filter hides", () => {
    const threads = [thread({ id: "local" })];
    const catalogs = [catalog("desktop_a", [{ sessionId: "a", title: "a", lastMessageAt: 1 }])];
    expect(hiddenCount(threads, catalogs, ALL)).toBe(0);
    expect(hiddenCount(threads, catalogs, { kind: "device", desktopId: null })).toBe(1);
    expect(hiddenCount(threads, catalogs, { kind: "device", desktopId: "desktop_a" })).toBe(1);
  });

  it("uses the host's own coalesce for a local thread with no messages", () => {
    expect(localTimestamp(thread({ id: "fresh", lastMessageAt: null, updatedAt: 42 }))).toBe(42);
    expect(localTimestamp(thread({ id: "messaged", lastMessageAt: 9, updatedAt: 42 }))).toBe(9);
  });

  /** A workspace session keeps its mode and workspace; the UI groups on both. */
  it("carries mode and workspace through for remote rows", () => {
    const merged = mergeConversations(
      [],
      [catalog("desktop_a", [{
        sessionId: "s",
        title: "s",
        mode: "workspace",
        workspaceId: "ws_a",
        lastMessageAt: 1,
      }])],
      ALL,
    );
    expect(merged[0]!.mode).toBe("workspace");
    expect(merged[0]!.workspaceId).toBe("ws_a");
    expect(merged[0]!.streaming).toBe(false);
  });

  /**
   * The host's pin and delete routes take a *thread* id, which is not the
   * session id. A row that dropped it would leave those two actions
   * unaddressable; a row that conflated the two would pin or delete something
   * else. Both sides are asserted, and so is the host that reports neither.
   */
  it("carries the host thread id, and a local row's own id as its thread id", () => {
    const merged = mergeConversations(
      [thread({ id: "local_1" })],
      [catalog("desktop_a", [
        { sessionId: "sess_1", threadId: "thread_1", title: "remote", lastMessageAt: 2 },
        { sessionId: "sess_2", title: "older host", lastMessageAt: 1 },
      ])],
      ALL,
    );

    const local = merged.find(row => row.id === "local_1")!;
    // The local store addresses threads by this id everywhere, so there is
    // nothing to translate.
    expect(local.threadId).toBe("local_1");

    const remote = merged.find(row => row.id === "sess_1")!;
    expect(remote.threadId).toBe("thread_1");
    expect(remote.threadId).not.toBe(remote.id);

    // A host that predates the field: pin and delete are refused downstream
    // rather than silently addressed with the session id.
    expect(merged.find(row => row.id === "sess_2")!.threadId).toBeNull();
  });
});
