import { applyCatalogDelta } from "../src/catalogDelta";
import type { RemoteSession } from "../src/types";

/**
 * The rules `catalog_delta_v1` rests on. Each case here is a way a delta could
 * corrupt the list if it were read as a snapshot.
 */

function row(sessionId: string, extra: Partial<RemoteSession> = {}): RemoteSession {
  return {
    sessionId,
    threadId: `thread_${sessionId}`,
    title: `title ${sessionId}`,
    streaming: false,
    lastMessageAt: 100,
    ...extra,
  } as RemoteSession;
}

describe("applyCatalogDelta", () => {
  it("treats a full packet as the whole catalogue", () => {
    const current = [row("a", { lastMessageAt: 300 }), row("b", { lastMessageAt: 200 })];
    const next = applyCatalogDelta(current, {
      full: true,
      rows: [row("b", { lastMessageAt: 400 }), row("c", { lastMessageAt: 100 })],
      removed: [],
    });
    expect(next.map((item) => item.sessionId)).toEqual(["b", "c"]);
  });

  it("keeps rows a packet does not mention — absence is not deletion", () => {
    const current = [row("a", { lastMessageAt: 300 }), row("b", { lastMessageAt: 200 })];
    const next = applyCatalogDelta(current, {
      full: false,
      rows: [row("a", { title: "renamed", lastMessageAt: 300 })],
      removed: [],
    });
    expect(next.map((item) => item.sessionId)).toEqual(["a", "b"]);
    expect(next[0]?.title).toBe("renamed");
  });

  it("deletes exactly the ids the packet names, and re-ranks what is left", () => {
    const current = [row("a", { lastMessageAt: 300 }), row("b", { lastMessageAt: 200 })];
    const next = applyCatalogDelta(current, {
      full: false,
      rows: [],
      removed: ["a"],
    });
    expect(next.map((item) => item.sessionId)).toEqual(["b"]);

    const reordered = applyCatalogDelta(next, {
      full: false,
      rows: [row("c", { lastMessageAt: 500 })],
      removed: [],
    });
    expect(reordered.map((item) => item.sessionId)).toEqual(["c", "b"]);
  });

  it("ranks pinned rows first, the way the host orders them", () => {
    // The rank is derived, not sent: pinned outranks every timestamp, and within
    // a class the newest activity leads.
    const next = applyCatalogDelta([], {
      full: true,
      rows: [
        row("old", { pinned: true, lastMessageAt: 10 }),
        row("new", { lastMessageAt: 9_000 }),
        row("newer", { lastMessageAt: 9_500 }),
      ],
      removed: [],
    });
    expect(next.map((item) => item.sessionId)).toEqual(["old", "newer", "new"]);
  });

  it("falls back to the creation time when a row has never spoken", () => {
    const next = applyCatalogDelta([], {
      full: true,
      rows: [
        row("silent", { lastMessageAt: null, createdAt: 5_000 }),
        row("chatty", { lastMessageAt: 1_000 }),
      ],
      removed: [],
    });
    expect(next.map((item) => item.sessionId)).toEqual(["silent", "chatty"]);
  });

  it("keeps the workspace a row already knew when the packet does not describe it", () => {
    // The flip-flop this exists for: a streamed row changes every second, its
    // packet carries no dictionary, and replacing the row wholesale dropped the
    // name the list was drawing — so the heading fell back to "workspace" and
    // came back on the next dictionary-bearing packet.
    const current = [
      row("a", {
        workspaceId: "w1",
        workspaceName: "家里",
        workspacePinned: true,
        workspaceOrderedAt: 42,
        lastMessageAt: 100,
      }),
    ];
    const next = applyCatalogDelta(current, {
      full: false,
      rows: [row("a", { workspaceId: "w1", lastMessageAt: 999 })],
      removed: [],
    });
    expect(next[0]?.lastMessageAt).toBe(999);
    expect(next[0]?.workspaceName).toBe("家里");
    expect(next[0]?.workspacePinned).toBe(true);
    expect(next[0]?.workspaceOrderedAt).toBe(42);
  });

  it("joins the workspace dictionary onto rows that carry only an id", () => {
    const next = applyCatalogDelta([], {
      full: true,
      rows: [row("a", { workspaceId: "w1" }), row("b", { workspaceId: "missing" })],
      removed: [],
      workspaces: [{ id: "w1", name: "工作区", pinned: true, orderedAt: 42 }],
    });
    const [a, b] = next as [RemoteSession, RemoteSession];
    expect(a.workspaceName).toBe("工作区");
    expect(a.workspacePinned).toBe(true);
    expect(a.workspaceOrderedAt).toBe(42);
    // An id this packet's dictionary does not describe leaves the row as it is.
    expect(b.workspaceName).toBeUndefined();
  });

  it("re-reads the dictionary onto every held row when it changes", () => {
    const current = [
      row("a", { workspaceId: "w1", workspaceName: "旧名字", workspaceOrderedAt: 1 }),
      row("b", { workspaceId: "w1", workspaceName: "旧名字", workspaceOrderedAt: 1 }),
    ];
    const next = applyCatalogDelta(current, {
      full: false,
      rows: [],
      removed: [],
      workspaces: [{ id: "w1", name: "新名字", pinned: false, orderedAt: 9 }],
    });
    expect(next.map((item) => item.workspaceName)).toEqual(["新名字", "新名字"]);
    expect(next.map((item) => item.workspaceOrderedAt)).toEqual([9, 9]);
  });

  it("ignores a repeated row rather than duplicating it", () => {
    const current = [row("a", { title: "one" })];
    const next = applyCatalogDelta(current, {
      full: false,
      rows: [row("a", { title: "two" }), row("a", { title: "three" })],
      removed: [],
    });
    expect(next).toHaveLength(1);
    expect(next[0]?.title).toBe("three");
  });
});
