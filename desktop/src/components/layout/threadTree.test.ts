import type { StoredThread } from "../../integrations/storage/threadStore";
import { describe, expect, it } from "vitest";
import { buildThreadTree, visibleThreadRows } from "./threadTree";

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
    expect(tree.map(n => n.thread.id)).toEqual(["a", "b"]);
    expect(tree[1]!.children.map(n => n.thread.id)).toEqual(["c"]);
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
    expect(buildThreadTree([child, parent])[0]!.children[0]!.thread).toBe(child);
  });
});
