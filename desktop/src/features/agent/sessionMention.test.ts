import type { StoredThread, StoredWorkspace } from "../../integrations/storage/types";
import { describe, expect, it } from "vitest";
import {
  filterSessionMentions,
  groupSessionMentions,
  sessionMentionOptions,
} from "./sessionMention";

function thread(over: Partial<StoredThread> & { id: string }): StoredThread {
  return {
    agentSessionId: over.id,
    createdAt: 0,
    mode: "chat",
    pinned: false,
    readonly: false,
    status: "active",
    title: over.id,
    updatedAt: 0,
    workspaceId: "",
    ...over,
  };
}

function workspace(id: string, over: Partial<StoredWorkspace> = {}): StoredWorkspace {
  return {
    cleanupStatus: "active",
    createdAt: 0,
    id,
    kind: "user",
    name: id,
    path: `/w/${id}`,
    updatedAt: 0,
    ...over,
  };
}

describe("sessionMentionOptions", () => {
  it("files chats first, then each workspace in pinned-first order", () => {
    const workspaces = [workspace("plain"), workspace("vip", { pinned: true })];
    const threads = [
      thread({ id: "s1", title: "chat", mode: "chat" }),
      thread({ id: "s2", title: "plain one", mode: "workspace", workspaceId: "plain" }),
      thread({ id: "s3", title: "vip one", mode: "workspace", workspaceId: "vip" }),
      thread({ id: "s4", title: "vip two", mode: "workspace", workspaceId: "vip" }),
    ];
    expect(sessionMentionOptions(threads, workspaces).map(option => option.sessionId))
      .toEqual(["s1", "s3", "s4", "s2"]);
    expect(sessionMentionOptions(threads, workspaces)[1]?.workspace)
      .toEqual({ id: "vip", name: "vip" });
  });

  it("keeps the caller's order inside a workspace", () => {
    const threads = [
      thread({ id: "b", workspaceId: "w", mode: "workspace" }),
      thread({ id: "a", workspaceId: "w", mode: "workspace" }),
    ];
    expect(sessionMentionOptions(threads, [workspace("w")]).map(option => option.sessionId))
      .toEqual(["b", "a"]);
  });

  it("drops the conversation being composed in, so no self-reference is offered", () => {
    const threads = [thread({ id: "s1" }), thread({ id: "s2" })];
    expect(sessionMentionOptions(threads, [], "s2").map(option => option.sessionId))
      .toEqual(["s1"]);
  });

  it("skips conversations with no agent session id and deleted ones", () => {
    const threads = [
      thread({ id: "never-prompted", agentSessionId: null }),
      thread({ id: "gone", status: "deleted" }),
      thread({ id: "live" }),
    ];
    expect(sessionMentionOptions(threads, []).map(option => option.sessionId))
      .toEqual(["live"]);
  });

  it("reports an unlisted workspace's thread without inventing a name", () => {
    // The store can still hold a thread whose workspace was removed; the group
    // heading falls back to the localized label rather than a blank name.
    const options = sessionMentionOptions(
      [thread({ id: "orphan", mode: "workspace", workspaceId: "ghost" })],
      [],
    );
    expect(options[0]?.workspace).toBeNull();
  });
});

describe("filterSessionMentions", () => {
  const options = sessionMentionOptions([
    thread({ id: "s1", title: "Fix the flaky test", mode: "workspace", workspaceId: "w1" }),
    thread({ id: "s2", title: "Unrelated title", mode: "workspace", workspaceId: "w2" }),
    thread({ id: "s3", title: "Another chat" }),
  ], [workspace("w1", { name: "Payments" }), workspace("w2", { name: "Renderer" })]);

  it("matches a title case-insensitively", () => {
    expect(filterSessionMentions(options, "FLAKY").map(option => option.sessionId)).toEqual(["s1"]);
  });

  it("brings along conversations filed under a matching workspace", () => {
    expect(filterSessionMentions(options, "payments").map(option => option.sessionId)).toEqual(["s1"]);
  });

  it("returns everything for an empty or whitespace query", () => {
    expect(filterSessionMentions(options, "   ")).toEqual(options);
  });

  it("returns nothing when no title or workspace matches", () => {
    expect(filterSessionMentions(options, "zzz")).toEqual([]);
  });
});

describe("groupSessionMentions", () => {
  it("bundles adjacent conversations of one workspace and drops empty groups", () => {
    const options = sessionMentionOptions([
      thread({ id: "c1", title: "chat" }),
      thread({ id: "a1", mode: "workspace", workspaceId: "a" }),
      thread({ id: "a2", mode: "workspace", workspaceId: "a" }),
      thread({ id: "b1", mode: "workspace", workspaceId: "b" }),
    ], [workspace("a"), workspace("b")]);
    const groups = groupSessionMentions(options);
    expect(groups.map(group => group.workspace?.id ?? null)).toEqual([null, "a", "b"]);
    expect(groups.map(group => group.sessions.length)).toEqual([1, 2, 1]);

    // A query that empties the middle group must not leave a heading behind.
    const filtered = groupSessionMentions(filterSessionMentions(options, "b1"));
    expect(filtered.map(group => group.workspace?.id ?? null)).toEqual(["b"]);
  });
});
