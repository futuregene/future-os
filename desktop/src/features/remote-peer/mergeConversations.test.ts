import type {
  DeviceFilter,
  LocalThreadLike,
  RemoteCatalog,
} from "./mergeConversations";
import { describe, expect, it } from "vitest";
import {
  compareConversations,
  compareSource,
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
  it("interleaves local threads and remote sessions by when they started, not by source", () => {
    const merged = mergeConversations(
      [thread({ id: "local_old", createdAt: 100 }), thread({ id: "local_new", createdAt: 900 })],
      [catalog("desktop_a", [
        { sessionId: "a_mid", title: "a mid", createdAt: 500 },
        { sessionId: "a_newest", title: "a newest", createdAt: 1_000 },
      ])],
      ALL,
    );

    expect(merged.map(row => row.id)).toEqual(["a_newest", "local_new", "a_mid", "local_old"]);
  });

  /**
   * The start time is the key, not the last reply.
   *
   * A run rewrites `lastMessageAt` about once a second while it streams, so a
   * list ranked by it swapped rows under the reader's thumb for as long as a run
   * was going: the row that had just started climbed above its neighbours and
   * stayed there once it finished. Ranking by the creation time leaves every row
   * where it was made, whatever it is doing now.
   */
  it("keeps a streaming row in place while its last message advances", () => {
    const ids = (streamingLastMessage: number) =>
      mergeConversations(
        [thread({ id: "older_started", createdAt: 100 })],
        [catalog("desktop_a", [
          {
            sessionId: "streaming",
            title: "streaming",
            createdAt: 50,
            lastMessageAt: streamingLastMessage,
            streaming: true,
          },
        ])],
        ALL,
      ).map(row => row.id);

    // The run is producing output, so its activity stamp is the newest thing in
    // the list — and it still sits below the newer conversation, as when idle.
    expect(ids(9_000)).toEqual(["older_started", "streaming"]);
    expect(ids(9_000)).toEqual(ids(200));
  });

  /**
   * A host older than the `createdAt` field has no start to rank by. It falls
   * back to `lastMessageAt` — the key this list used for every row before — so
   * its rows still interleave sensibly instead of dropping to the bottom.
   */
  it("ranks a row from a host without a start time by its last activity", () => {
    const merged = mergeConversations(
      [thread({ id: "local", createdAt: 500 })],
      [catalog("desktop_a", [
        { sessionId: "legacy_newest", title: "legacy", lastMessageAt: 2_000 },
        { sessionId: "legacy_oldest", title: "legacy", lastMessageAt: 100 },
      ])],
      ALL,
    );

    expect(merged.map(row => row.id)).toEqual(["legacy_newest", "local", "legacy_oldest"]);
  });

  /**
   * Two hosts and the local machine can each hold a session with the same id
   * (ids are unique per host, not globally). They must stay two rows, and each
   * must carry the host it belongs to — a row that lost its source would be
   * routed to the wrong machine.
   */
  it("keeps same-id sessions from different machines apart, each labelled", () => {
    const merged = mergeConversations(
      [thread({ id: "shared", createdAt: 100 })],
      [
        catalog("desktop_a", [{ sessionId: "shared", title: "on a", createdAt: 300 }]),
        catalog("desktop_b", [{ sessionId: "shared", title: "on b", createdAt: 200 }]),
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
      [thread({ id: "local_pinned", pinned: true, createdAt: 1 })],
      [catalog("desktop_a", [
        { sessionId: "a_old_pinned", title: "pinned", pinned: true, createdAt: 10 },
        { sessionId: "a_fresh", title: "fresh", createdAt: 10_000 },
      ])],
      ALL,
    );

    // Within the pinned group the same start-time rule applies, so the pinned
    // row started later leads — pinned is a section, not a tie-breaker. The
    // unpinned conversation started last is still below the pins.
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

  /**
   * A tie is broken on the machine before the row id: this machine first, then a
   * host by its desktop id.
   *
   * The same conversation can exist on two machines with the same timestamp (the
   * same message, a copied store), and a row's key starts with its machine — so
   * without this rule the order of a tie would be decided by how `"desktop_a"`
   * and `"local"` happen to sort as text.
   */
  it("puts this machine first, then a host by its device id, when timestamps tie", () => {
    const merged = mergeConversations(
      [thread({ id: "shared", createdAt: 7, lastMessageAt: 7 })],
      [
        catalog("desktop_b", [{ sessionId: "shared", title: "on b", createdAt: 7, lastMessageAt: 7 }]),
        catalog("desktop_a", [{ sessionId: "shared", title: "on a", createdAt: 7, lastMessageAt: 7 }]),
      ],
      ALL,
    );

    expect(merged.map(row => row.desktopId)).toEqual([null, "desktop_a", "desktop_b"]);
  });

  /**
   * A remote fork names its parent, resolved to the row the list holds.
   *
   * The host reports the *session* id it was forked from; the list's tree is
   * keyed by row, so the merge is where the two are related. A row that lost this
   * is drawn as a root — no indent, no fold toggle — which is what a fork on
   * another machine looked like beside the same fork on this one.
   */
  it("resolves a remote fork to its parent's row", () => {
    const merged = mergeConversations(
      [],
      [catalog("desktop_a", [
        { sessionId: "parent", title: "parent", lastMessageAt: 500 },
        { sessionId: "child", title: "child", parentSessionId: "parent", lastMessageAt: 400 },
        { sessionId: "grandchild", title: "grandchild", parentSessionId: "child", lastMessageAt: 300 },
      ])],
      ALL,
    );

    const byId = (id: string) => merged.find(row => row.id === id)!;
    expect(byId("parent").parentKey).toBeNull();
    expect(byId("child").parentKey).toBe(byId("parent").key);
    expect(byId("grandchild").parentKey).toBe(byId("child").key);
  });

  /**
   * This machine's forks name their parent the same way, in this list's keys.
   *
   * The rail resolves a local fork's parent from its own store (a fork records an
   * agent session id, not a thread id) and hands it in; the merge is where that
   * becomes a row key, so the merged list's tree holds both machines' forks with
   * one rule. A local fork whose parent is not listed — archived, or another
   * machine's filter — stays a root, as it does in the local tree.
   */
  it("resolves a local fork to its parent's row", () => {
    const merged = mergeConversations(
      [
        thread({ id: "local_parent", parentThreadId: null }),
        thread({ id: "local_child", parentThreadId: "local_parent" }),
        thread({ id: "local_orphan", parentThreadId: "archived" }),
      ],
      [],
      ALL,
    );

    const byId = (id: string) => merged.find(row => row.id === id)!;
    expect(byId("local_parent").parentKey).toBeNull();
    expect(byId("local_child").parentKey).toBe(rowKey(null, "local_parent"));
    expect(byId("local_orphan").parentKey).toBeNull();
  });

  /**
   * A parent the list does not hold makes its child a root — including a parent
   * that happens to share the session id on *another* machine, which is not the
   * conversation it was forked from.
   */
  it("resolves a fork's parent on its own machine only", () => {
    const merged = mergeConversations(
      [],
      [
        catalog("desktop_a", [{ sessionId: "orphan", title: "orphan", parentSessionId: "gone", lastMessageAt: 300 }]),
        // The same parent session id on another host: same id, different
        // conversation, and not this row's parent.
        catalog("desktop_b", [
          { sessionId: "parent", title: "the other machine's", lastMessageAt: 200 },
          { sessionId: "elsewhere", title: "elsewhere", parentSessionId: "parent", lastMessageAt: 100 },
        ]),
      ],
      ALL,
    );

    const byId = (id: string) => merged.find(row => row.id === id)!;
    expect(byId("orphan").parentKey).toBeNull();
    expect(byId("elsewhere").parentKey).toBe(byId("parent").key);

    // Filtered to the child's own machine, its parent is not a row here either.
    const oneHost = mergeConversations(
      [],
      [catalog("desktop_b", [
        { sessionId: "parent", title: "the other machine's", lastMessageAt: 200 },
        { sessionId: "elsewhere", title: "elsewhere", parentSessionId: "parent", lastMessageAt: 100 },
      ])],
      { kind: "device", desktopId: "desktop_a" },
    );
    expect(oneHost).toEqual([]);
  });

  /**
   * The workspace's own place rides on the row, like its name and its pin: it is
   * what the group is ordered by, and only the owner knows it.
   */
  it("carries the workspace's own place from both sources", () => {
    const merged = mergeConversations(
      [thread({ id: "local", workspaceId: "ws_local", workspaceOrderedAt: 4_000 })],
      [catalog("desktop_a", [{
        sessionId: "remote",
        title: "remote",
        mode: "workspace",
        workspaceId: "ws_a",
        workspaceOrderedAt: 9_000,
        lastMessageAt: 1,
      }])],
      ALL,
    );

    expect(merged.find(row => row.id === "local")!.workspaceOrderedAt).toBe(4_000);
    expect(merged.find(row => row.id === "remote")!.workspaceOrderedAt).toBe(9_000);
    // A source that does not report one says so, rather than reporting "now".
    const undated = mergeConversations([], [catalog("desktop_a", [{ sessionId: "old", title: "old", lastMessageAt: 1 }])], ALL);
    expect(undated[0]!.workspaceOrderedAt).toBeNull();
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
    const threads = [thread({ id: "local", createdAt: 2_000 })];
    const catalogs = [catalog("desktop_a", [{ sessionId: "a", title: "a", createdAt: 1_000 }])];
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
     * A fork is a conversation's, so it is ranked — and filed — by that
     * conversation, not by its own time.
     *
     * A fork is started after the conversation it was forked from, by
     * construction, so ranking the rows themselves put the fork *above* its own
     * conversation: pressing the parent's `+` looked like it did nothing, because
     * the row it revealed had already been on screen, one line higher. The forks
     * of a pinned or workspace conversation landed under somebody else's heading
     * for the same reason. `rowRoots` is the tree's answer to "which conversation
     * is this row's", handed in by the rail.
     */
    it("ranks a fork by its conversation, next to it", () => {
      const merged = rows(
        [
          thread({ id: "parent", mode: "workspace", workspaceId: "ws_1", createdAt: 500 }),
          thread({ id: "fork", mode: "workspace", workspaceId: "ws_1", parentThreadId: "parent", createdAt: 900 }),
          thread({ id: "other", mode: "workspace", workspaceId: "ws_1", createdAt: 950 }),
        ],
        [],
      );
      const byId = (id: string) => merged.find(row => row.id === id)!;

      // The newer conversation still leads; the fork follows its own conversation
      // rather than jumping over it.
      expect(
        groupMergedRows(merged, new Map([[byId("fork").key, byId("parent")]]))
          .workspaces[0]!.rows.map(row => row.id),
      ).toEqual(["other", "parent", "fork"]);
    });

    /**
     * …and it takes that conversation's *section*, not one its own fields pick: the
     * forks of a pinned conversation are pinned rows.
     *
     * The sections are drawn apart — pinned at the top, workspaces below — so a
     * fork that kept its own workspace filed itself under a heading far from the
     * `+` the user had just pressed on its parent.
     */
    it("files a fork in its conversation's section", () => {
      const merged = rows(
        [
          thread({ id: "parent", mode: "workspace", workspaceId: "ws_1", pinned: true, updatedAt: 500 }),
          thread({ id: "fork", mode: "workspace", workspaceId: "ws_1", pinned: false, parentThreadId: "parent", updatedAt: 900 }),
          thread({ id: "chat", updatedAt: 100 }),
        ],
        [],
      );
      const byId = (id: string) => merged.find(row => row.id === id)!;

      const grouped = groupMergedRows(merged, new Map([[byId("fork").key, byId("parent")]]));
      expect(grouped.pinned.map(row => row.id)).toEqual(["parent", "fork"]);
      // No heading for a workspace whose only conversation went with its parent:
      // an empty group is not a group.
      expect(grouped.workspaces).toEqual([]);
      expect(grouped.chats.map(row => row.id)).toEqual(["chat"]);
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
     * A group is ordered by the workspace's *own* place, never by its rows'.
     *
     * Ordering groups by their newest conversation reads as reasonable — the
     * list is ordered by time — and is wrong in the two ways that matter: the
     * group jumps while the reader is working in it (a reply moves it to the top,
     * and the next catalogue read moves it back), and the merged list then
     * disagrees with the list of this machine alone, which orders workspaces by
     * when *they* were last touched. So the rows' recency decides the order
     * inside a group and nothing about the group's place.
     */
    it("orders groups by the workspace's own place, not by their conversations", () => {
      const grouped = groupMergedRows(rows(
        [
          // The busier workspace, and the older one: a list that ordered groups
          // by their rows would put it first.
          thread({ id: "busy", mode: "workspace", workspaceId: "ws_busy", updatedAt: 900, workspaceOrderedAt: 10 }),
          thread({ id: "quiet", mode: "workspace", workspaceId: "ws_quiet", updatedAt: 20, workspaceOrderedAt: 500 }),
        ],
        [],
      ));

      // Keyed by the workspace id alone for this machine, which is also the key
      // the local list folds by (see `groupMergedRows`).
      expect(grouped.workspaces.map(group => group.key)).toEqual(["ws_quiet", "ws_busy"]);
      expect(grouped.workspaces.map(group => group.orderedAt)).toEqual([500, 10]);
    });

    /** A conversation arriving inside a workspace must not move the workspace. */
    it("holds a group's place while a reply lands in it", () => {
      const group = (updatedAt: number) => groupMergedRows(rows(
        [thread({ id: "t", mode: "workspace", workspaceId: "ws_1", updatedAt, workspaceOrderedAt: 100 })],
        [],
      )).workspaces[0]!;

      // The row's own time moves by hours; the group's place does not.
      const before = group(10);
      const after = group(9_000);
      expect(after.orderedAt).toBe(before.orderedAt);
      expect(after.rows[0]!.lastMessageAt).toBeGreaterThan(before.rows[0]!.lastMessageAt!);
    });

    /**
     * Two workspaces equally placed: this machine's first, then a host by its
     * device id — the same rule the rows of a chat section are ordered by.
     */
    it("breaks a tie on the machine, this machine first", () => {
      const grouped = groupMergedRows(rows(
        [thread({ id: "local_ws", mode: "workspace", workspaceId: "ws_x", workspaceOrderedAt: 7 })],
        [
          catalog("desktop_b", [{
            sessionId: "b_ws",
            title: "on b",
            mode: "workspace",
            workspaceId: "ws_x",
            workspaceOrderedAt: 7,
            lastMessageAt: 7,
          }]),
          catalog("desktop_a", [{
            sessionId: "a_ws",
            title: "on a",
            mode: "workspace",
            workspaceId: "ws_x",
            workspaceOrderedAt: 7,
            lastMessageAt: 7,
          }]),
        ],
      ));

      expect(grouped.workspaces.map(group => group.desktopId))
        .toEqual([null, "desktop_a", "desktop_b"]);
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

/**
 * The comparator every order in the module ends on: this machine (`null`) first,
 * then a host by its device id.
 */
describe("compareSource", () => {
  it("sorts this machine ahead of every host", () => {
    expect(compareSource(null, "desktop_a")).toBe(-1);
    expect(compareSource("desktop_a", null)).toBe(1);
  });

  it("orders two hosts by device id and treats a missing id as itself", () => {
    expect(compareSource("desktop_a", "desktop_b")).toBeLessThan(0);
    expect(compareSource("desktop_b", "desktop_a")).toBeGreaterThan(0);
    expect(compareSource("desktop_a", "desktop_a")).toBe(0);
    expect(compareSource(null, null)).toBe(0);
  });

  it("is the rule that decides a tie in the merged list", () => {
    // Same timestamp on this machine and on a host: the local row must lead,
    // and the tie must not be decided by how "local" and "desktop_a" sort.
    const merged = mergeConversations(
      [thread({ id: "tie-local", lastMessageAt: 400 })],
      [catalog("desktop_a", [{ sessionId: "tie-remote", title: "remote", lastMessageAt: 400 }])],
      ALL,
    );
    expect(merged.map(row => row.desktopId)).toEqual([null, "desktop_a"]);
    expect(compareSource(merged[0]!.desktopId, merged[1]!.desktopId)).toBeLessThan(0);
  });
});
