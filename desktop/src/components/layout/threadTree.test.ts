import type { StoredThread } from "../../integrations/storage/threadStore";
import { describe, expect, it } from "vitest";
import { buildThreadTree, buildTree, threadParentIds, visibleThreadRows, visibleTreeRows } from "./threadTree";

function thread(id: string, parentSessionId?: string, pinned = false): StoredThread {
  return { id, agentSessionId: id, parentSessionId, pinned, mode: "chat", workspaceId: id, title: id, status: "active", readonly: false, createdAt: 0, updatedAt: 0 };
}

const expanded = new Set(["a", "b", "c", "d", "e"]);

describe("conversation tree", () => {
  it("defaults to collapsed and reveals children one level at a time, regardless of import order", () => {
    const tree = buildThreadTree([thread("c", "b"), thread("b", "a"), thread("a")]);
    expect(visibleThreadRows(tree, new Set()).map(r => r.thread.id)).toEqual(["a"]);
    expect(visibleThreadRows(tree, new Set(["a"])).map(r => [r.thread.id, r.depth, r.hasChildren])).toEqual([["a", 0, true], ["b", 1, true]]);
    expect(visibleThreadRows(tree, expanded).map(r => [r.thread.id, r.depth])).toEqual([["a", 0], ["b", 1], ["c", 2]]);
  });

  it("never exceeds three levels or loses deeper historical descendants", () => {
    const tree = buildThreadTree([thread("e", "d"), thread("d", "c"), thread("c", "b"), thread("b", "a"), thread("a")]);
    const rows = visibleThreadRows(tree, expanded);
    expect(rows.map(r => [r.thread.id, r.depth])).toEqual([["a", 0], ["b", 1], ["e", 2], ["d", 2], ["c", 2]]);
  });

  it("promotes pinned children without duplicating their descendants", () => {
    const tree = buildThreadTree([thread("a"), thread("b", "a", true), thread("c", "b")]);
    expect(tree.map(n => n.item.id)).toEqual(["a", "b"]);
    expect(tree[1]!.children.map(n => n.item.id)).toEqual(["c"]);
    expect(visibleThreadRows(tree, expanded)).toHaveLength(3);
  });

  it("keeps orphaned/self-linked conversations accessible and breaks cycles", () => {
    const tree = buildThreadTree([thread("a", "b"), thread("b", "a"), thread("c", "missing"), thread("d", "d"), thread("e", "b")]);
    const rows = visibleThreadRows(tree, expanded);
    expect(new Set(rows.map(r => r.thread.id))).toEqual(expanded);
    expect(rows).toHaveLength(5);
    expect(rows.every(r => r.depth < 3)).toBe(true);
  });

  it("resolves Agent ids, not Desktop ids, across workspace boundaries", () => {
    const parent = { ...thread("a"), agentSessionId: "session-a", mode: "workspace" as const, workspaceId: "project" };
    const child = thread("b", "session-a");
    expect(buildThreadTree([child, parent])[0]!.children[0]!.item).toBe(child);
  });

  /**
   * The parent links the tree is built from, on their own.
   *
   * The merged list is handed these so a local fork is hung under the row it came
   * from there too, and the one thing that must not drift is the resolution: a
   * fork names its parent by agent session id, and only a thread that is *in the
   * list* can be a parent — a fork of a conversation that is archived or gone is a
   * root in both lists, not a row with a parent neither one can draw.
   */
  it("reports the thread each one was forked from, by thread id", () => {
    const parent = { ...thread("a"), agentSessionId: "session-a" };
    const child = thread("b", "session-a");
    expect(threadParentIds([parent, child])).toEqual(new Map([["b", "a"]]));
    // Its parent is not a thread here (archived, another machine's, never loaded).
    expect(threadParentIds([child])).toEqual(new Map());
    // A thread naming its own session is a root, not its own parent.
    expect(threadParentIds([{ ...parent, parentSessionId: "session-a" }])).toEqual(new Map());
  });

  it("builds the same tree from any list whose rows name their parent", () => {
    // The merged list's remote rows, which know their parent by row key rather
    // than by session id: one algorithm, so a fork is one shape in both lists.
    const rows = [
      { key: "h::a", parentKey: null, pinned: false },
      { key: "h::b", parentKey: "h::a", pinned: false },
      { key: "h::c", parentKey: "h::b", pinned: false },
    ];
    const tree = buildTree(rows, row => row);
    expect(visibleTreeRows(tree, row => row.key, new Set()).map(r => [r.item.key, r.depth]))
      .toEqual([["h::a", 0]]);
    expect(visibleTreeRows(tree, row => row.key, new Set(["h::a", "h::b"])).map(r => [r.item.key, r.depth, r.hasChildren]))
      .toEqual([["h::a", 0, true], ["h::b", 1, true], ["h::c", 2, false]]);
  });
});
