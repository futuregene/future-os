import type {
  DeviceFilter,
  LocalThreadLike,
  RemoteCatalog,
} from "./mergeConversations";
import { describe, expect, it } from "vitest";
import {
  compareConversations,
  groupMergedRows,
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
    // Filtered to that host, its absence is the whole list rather than a stale row.
    expect(mergeConversations([thread({ id: "local" })], [], { kind: "device", desktopId: "desktop_a" }))
      .toEqual([]);
  });

  /** Filtering to one machine selects exactly that machine's rows. */
  it("selects one machine's rows when filtered to it", () => {
    // Explicit timestamps: the local row is the newer one, so "all" must order
    // it first — the filter decides which rows are candidates, not their order.
    const threads = [thread({ id: "local", updatedAt: 2_000 })];
    const catalogs = [catalog("desktop_a", [{ sessionId: "a", title: "a", lastMessageAt: 1_000 }])];
    const ids = (filter: DeviceFilter) =>
      mergeConversations(threads, catalogs, filter).map(row => row.id);

    expect(ids(ALL)).toEqual(["local", "a"]);
    expect(ids({ kind: "device", desktopId: null })).toEqual(["local"]);
    expect(ids({ kind: "device", desktopId: "desktop_a" })).toEqual(["a"]);
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

  /**
   * The merged list keeps the local list's headings. Without this a user who
   * pairs a machine loses the workspace grouping they had before the feature
   * existed, and every row reads as if it were pinned.
   */
  describe("groupMergedRows", () => {
    function rows(threads: LocalThreadLike[], catalogs: RemoteCatalog[]) {
      return mergeConversations(
        threads,
        catalogs,
        ALL,
        new Map([["ws_local", "This machine's workspace"]]),
      );
    }

    it("splits pinned from the rest, and workspaces from chats", () => {
      const grouped = groupMergedRows(rows(
        [
          thread({ id: "chat_1", workspaceId: "ws_local" }),
          thread({ id: "pinned_1", pinned: true }),
          thread({ id: "ws_1", mode: "workspace", workspaceId: "ws_local" }),
        ],
        [],
      ));

      expect(grouped.pinned.map(row => row.id)).toEqual(["pinned_1"]);
      expect(grouped.workspaces.flatMap(group => group.rows.map(row => row.id))).toEqual(["ws_1"]);
      expect(grouped.workspaces[0]!.label).toBe("This machine's workspace");
      expect(grouped.chats.map(row => row.id)).toEqual(["chat_1"]);
    });

    /**
     * Two machines hold a workspace of the same id and name — the common case,
     * since workspaces are named after the folder. One heading for both would
     * file another machine's rows under this one's name.
     */
    it("keeps same-named workspaces on different machines apart", () => {
      const grouped = groupMergedRows(rows(
        [thread({ id: "local_ws", mode: "workspace", workspaceId: "ws_1" })],
        [catalog("desktop_a", [
          {
            sessionId: "a_ws",
            title: "on a",
            mode: "workspace",
            workspaceId: "ws_1",
            workspaceName: "future-os",
            lastMessageAt: 500,
          },
        ])],
      ));

      expect(grouped.workspaces).toHaveLength(2);
      expect(grouped.workspaces.map(group => group.desktopId).sort()).toEqual([
        null,
        "desktop_a",
      ].sort());
      // The remote heading is the host's own name for it.
      expect(grouped.workspaces.find(group => group.desktopId === "desktop_a")!.label).toBe("future-os");
    });

    /**
     * A row whose host predates the name field must not blank a heading another
     * row in the same group already supplied — otherwise the group loses its
     * name as soon as one older session appears in it.
     */
    it("keeps a heading that an older row in the group did not carry", () => {
      const grouped = groupMergedRows(rows([], [catalog("desktop_a", [
        {
          sessionId: "named",
          title: "named",
          mode: "workspace",
          workspaceId: "ws_1",
          workspaceName: "future-os",
          lastMessageAt: 900,
        },
        {
          sessionId: "unnamed",
          title: "unnamed",
          mode: "workspace",
          workspaceId: "ws_1",
          lastMessageAt: 800,
        },
      ])]));

      expect(grouped.workspaces).toHaveLength(1);
      expect(grouped.workspaces[0]!.label).toBe("future-os");
      expect(grouped.workspaces[0]!.rows).toHaveLength(2);
    });

    /**
     * Headings move with activity: a list whose groups are fixed to an arbitrary
     * machine order buries the workspace the user was just working in.
     */
    it("orders groups by their most recent row", () => {
      const grouped = groupMergedRows(rows(
        [
          thread({ id: "old_ws", mode: "workspace", workspaceId: "ws_old", updatedAt: 10 }),
          thread({ id: "new_ws", mode: "workspace", workspaceId: "ws_new", updatedAt: 900 }),
        ],
        [],
      ));

      // Keyed by the workspace id alone for this machine, which is also the key
      // the local list folds by (see `groupMergedRows`).
      expect(grouped.workspaces.map(group => group.key)).toEqual(["ws_new", "ws_old"]);
    });

    /**
     * A pinned workspace leads, on whichever machine it was pinned.
     *
     * The pin is the *owner's*: a folder on the far side of a pairing is pinned
     * there, and the user set it from here. Ordering only by recency left a
     * pinned group in the middle of the list with its pin nowhere to be seen,
     * which is what "clicking pin on a remote workspace does nothing" looked
     * like.
     */
    it("leads with a pinned workspace, local or remote", () => {
      const remote = groupMergedRows(rows([], [catalog("desktop_a", [
        {
          sessionId: "remote_ws",
          title: "pinned over there",
          mode: "workspace",
          workspaceId: "ws_remote",
          workspaceName: "图表脚本",
          workspacePinned: true,
          // Older than the unpinned local group, so recency alone would put it
          // second: only the pin can lead it.
          lastMessageAt: 100,
        },
      ])]));
      expect(remote.workspaces.map(group => group.key)).toEqual(["desktop_a::ws_remote"]);
      expect(remote.workspaces[0]!.pinned).toBe(true);

      const mixed = groupMergedRows(rows(
        [
          thread({ id: "unpinned_ws", mode: "workspace", workspaceId: "ws_new", updatedAt: 900 }),
          thread({ id: "pinned_ws", mode: "workspace", workspaceId: "ws_local", updatedAt: 10, workspacePinned: true }),
        ],
        [],
      ));
      expect(mixed.workspaces.map(group => group.key)).toEqual(["ws_local", "ws_new"]);
    });
  });
});
