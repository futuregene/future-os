import type { StoredThread } from "../../integrations/storage/threadStore";
import { describe, expect, it } from "vitest";
import { ARCHIVED_SCOPE, archiveScopeEntries, archiveThreadScopes } from "./archiveScopes";

function thread(id: string, workspaceId = "ws_a"): StoredThread {
  return {
    id,
    title: id,
    status: "archived",
    mode: "workspace",
    workspaceId,
    createdAt: 1,
    updatedAt: 1,
  } as StoredThread;
}

describe("archiveScopeEntries", () => {
  it("names a conversation once even when both streams describe it", () => {
    // The report: archiving a workspace first, then selecting one of its
    // conversations offered to delete two — the row was counted and named twice
    // because the conversations stream and the workspace group both had it.
    const inside = thread("9999输入意图不明");
    const entries = archiveScopeEntries([inside], { ws_a: [inside] });

    expect(entries).toHaveLength(1);
    expect(entries.filter(entry => entry.thread.id === inside.id)).toHaveLength(1);
  });

  it("keeps a conversation listed only by the conversations stream", () => {
    const only = thread("科研流程演示文件概览");
    const entries = archiveScopeEntries([only], {});

    expect(entries).toEqual([{ scope: ARCHIVED_SCOPE, thread: only }]);
  });

  it("lists a group's conversations that the conversations stream does not", () => {
    const inside = thread("聊天文件面板串到错误工作区");
    const entries = archiveScopeEntries([], { ws_a: [inside] });

    expect(entries).toEqual([{ scope: "archived-workspace:ws_a", thread: inside }]);
  });
});

describe("archiveThreadScopes", () => {
  /**
   * A row belongs to the batch of the heading it is drawn under.
   *
   * The regression this locks: keeping the first description gave a
   * conversation that both streams listed the `archived` scope, while the row
   * the reader ticked was drawn inside a workspace group, whose heading opens
   * `archived-workspace:<id>`. The checkbox ticked and the toolbar said
   * "1 selected", but the open batch held no row the ticked id belonged to — so
   * Delete opened no dialog and Restore did nothing.
   */
  it("puts a conversation drawn inside a shelved workspace in that group's batch", () => {
    const inside = thread("9999输入意图不明");
    const scopes = archiveThreadScopes([inside], { ws_a: [inside] });

    expect(scopes.get(inside.id)).toBe("archived-workspace:ws_a");
  });

  it("keeps a shelved conversation that no group draws in the conversations batch", () => {
    const only = thread("科研流程演示文件概览");
    const scopes = archiveThreadScopes([only], {});

    expect(scopes.get(only.id)).toBe(ARCHIVED_SCOPE);
  });

  it("files each group's rows under its own heading", () => {
    const a = thread("a", "ws_a");
    const b = thread("b", "ws_b");
    const scopes = archiveThreadScopes([], { ws_a: [a], ws_b: [b] });

    expect(scopes.get("a")).toBe("archived-workspace:ws_a");
    expect(scopes.get("b")).toBe("archived-workspace:ws_b");
  });
});
