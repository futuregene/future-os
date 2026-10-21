import type { StoredThread, StoredWorkspace } from "../../integrations/storage/types";
import { describe, expect, it } from "vitest";
import { filterSessionMentions, sessionMentionOptions } from "./sessionMention";

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
  it("orders conversations by last activity, newest first, across workspaces", () => {
    // Two conversations in one workspace and an older chat: the order is one
    // sequence, not three workspace stretches that each restart at the top.
    const threads = [
      thread({ id: "a-old", mode: "workspace", workspaceId: "a", lastMessageAt: 100 }),
      thread({ id: "chat-new", lastMessageAt: 500 }),
      thread({ id: "a-new", mode: "workspace", workspaceId: "a", lastMessageAt: 900 }),
      thread({ id: "b-mid", mode: "workspace", workspaceId: "b", lastMessageAt: 300 }),
    ];
    expect(sessionMentionOptions(threads, [workspace("a"), workspace("b")])
      .map(option => option.sessionId))
      .toEqual(["a-new", "chat-new", "b-mid", "a-old"]);
  });

  it("dates a conversation never prompted by when it was last written", () => {
    // `last_message_at` is NULL until the first message; the store's own
    // ordering key falls back to `updated_at`, and so does this list.
    const threads = [
      thread({ id: "prompted", lastMessageAt: 10 }),
      thread({ id: "fresh", lastMessageAt: null, updatedAt: 20 }),
    ];
    expect(sessionMentionOptions(threads, []).map(option => option.sessionId))
      .toEqual(["fresh", "prompted"]);
  });

  it("does not hoist a pinned conversation above a more recent one", () => {
    // A pin places a conversation in the navigation list; the `#` menu's one
    // question is "which one did I just use", so it orders by activity alone.
    const threads = [
      thread({ id: "pinned-old", pinned: true, lastMessageAt: 1 }),
      thread({ id: "recent", lastMessageAt: 2 }),
    ];
    expect(sessionMentionOptions(threads, []).map(option => option.sessionId))
      .toEqual(["recent", "pinned-old"]);
  });

  it("breaks a tie on the id, so a poll cannot reshuffle the menu", () => {
    const threads = [
      thread({ id: "b", lastMessageAt: 7 }),
      thread({ id: "a", lastMessageAt: 7 }),
    ];
    expect(sessionMentionOptions(threads, []).map(option => option.sessionId))
      .toEqual(["a", "b"]);
  });

  // The regression behind the `#` menu's wall of workspaces: every chat
  // conversation owns a temporary workspace row named "<title> Workspace", so
  // filing by workspace id gave each chat a group of its own. A chat is a chat
  // by `mode` (the rail's rule), whatever row it is stored under.
  it("names no workspace for a chat stored under a temporary workspace of its own", () => {
    const workspaces = [workspace("ws_temp", { name: "Fix the flaky test Workspace", kind: "temporary" })];
    const threads = [
      thread({ id: "chat-1", title: "Fix the flaky test", mode: "chat", workspaceId: "ws_temp" }),
      thread({ id: "ws-1", title: "Real work", mode: "workspace", workspaceId: "ws_temp" }),
    ];
    expect(sessionMentionOptions(threads, workspaces)
      .map(option => [option.sessionId, option.workspace?.name ?? null]))
      .toEqual([
        ["chat-1", null],
        ["ws-1", "Fix the flaky test Workspace"],
      ]);
  });

  it("still names a workspace the store no longer lists, with no name to show", () => {
    // A deleted workspace's conversations are not chats; the row says it is a
    // workspace the menu has no name for, and the menu labels it generically.
    const threads = [
      thread({ id: "g1", mode: "workspace", workspaceId: "ghost" }),
    ];
    expect(sessionMentionOptions(threads, [workspace("listed")])[0]?.workspace)
      .toEqual({ id: "ghost", name: "" });
  });

  it("drops the conversation being composed in, so no self-reference is offered", () => {
    const threads = [
      thread({ id: "s1", lastMessageAt: 2 }),
      thread({ id: "s2", lastMessageAt: 1 }),
    ];
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

  it("files a workspace conversation with no workspace id at all as a chat", () => {
    // Degenerate row (no scope to file under): naming workspace "" would read as
    // a workspace the reader cannot identify, so it is left unlabelled.
    const options = sessionMentionOptions(
      [thread({ id: "orphan", mode: "workspace", workspaceId: "" })],
      [],
    );
    expect(options[0]?.workspace).toBeNull();
  });
});

describe("filterSessionMentions", () => {
  const options = sessionMentionOptions([
    thread({ id: "s1", title: "Fix the flaky test", mode: "workspace", workspaceId: "w1", lastMessageAt: 3 }),
    thread({ id: "s2", title: "Unrelated title", mode: "workspace", workspaceId: "w2", lastMessageAt: 2 }),
    thread({ id: "s3", title: "Another chat", lastMessageAt: 1 }),
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

  it("keeps the recency order it was given", () => {
    expect(filterSessionMentions(options, "t").map(option => option.sessionId))
      .toEqual(["s1", "s2", "s3"]);
  });
});
