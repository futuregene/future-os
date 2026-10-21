import type { RemoteSession, RemoteWorkspace } from "../../../remote/types";
import { sessionMentionOptions } from "../sessionMention";

const session = (sessionId: string, title: string, extra: Partial<RemoteSession> = {}): RemoteSession => ({
  sessionId,
  threadId: `t_${sessionId}`,
  title,
  mode: "workspace",
  streaming: false,
  ...extra,
});

const workspace = (id: string, name: string, pinned = false): RemoteWorkspace => ({
  id,
  name,
  path: `/p/${id}`,
  pinned,
});

const ids = (options: ReturnType<typeof sessionMentionOptions>) =>
  options.map(option => option.sessionId);

it("orders the conversations by last activity, newest first, across workspaces", () => {
  // The order is one sequence, not a stretch per workspace: the conversation
  // used a minute ago leads whatever workspace it lives in.
  const options = sessionMentionOptions(
    [
      session("a-old", "Older in a", { workspaceId: "a", lastMessageAt: 100 }),
      session("chat-new", "Loose chat", { mode: "chat", lastMessageAt: 500 }),
      session("a-new", "Newer in a", { workspaceId: "a", lastMessageAt: 900 }),
      session("b-mid", "Middle in b", { workspaceId: "b", lastMessageAt: 300 }),
    ],
    [workspace("a", "Dopamine"), workspace("b", "Sparse attention")],
  );
  expect(ids(options)).toEqual(["a-new", "chat-new", "b-mid", "a-old"]);
  // The workspace still rides on the row — the list has no headings to say it.
  expect(options.map(option => option.workspace?.name ?? null))
    .toEqual(["Dopamine", null, "Sparse attention", "Dopamine"]);
});

it("dates a row the host sent no activity for by when it began", () => {
  // A desktop older than `lastMessageAt` sends none: the start time stands in,
  // and such a row sorts below the ones that can be dated rather than as "now".
  const options = sessionMentionOptions(
    [
      session("undated", "Old host"),
      session("dated", "Dated", { lastMessageAt: 10 }),
      session("started", "Newest host", { createdAt: 20 }),
    ],
    [],
  );
  expect(ids(options)).toEqual(["started", "dated", "undated"]);
});

it("does not hoist a pinned conversation above a more recent one", () => {
  const options = sessionMentionOptions(
    [
      session("pinned-old", "Pinned", { pinned: true, lastMessageAt: 1 }),
      session("recent", "Recent", { lastMessageAt: 2 }),
    ],
    [],
  );
  expect(ids(options)).toEqual(["recent", "pinned-old"]);
});

it("breaks a tie on the session id, so a poll cannot reshuffle the menu", () => {
  const options = sessionMentionOptions(
    [session("b", "B", { lastMessageAt: 7 }), session("a", "A", { lastMessageAt: 7 })],
    [],
  );
  expect(ids(options)).toEqual(["a", "b"]);
});

it("drops the conversation being composed in, so nothing references itself", () => {
  const options = sessionMentionOptions(
    [session("s1", "One"), session("s2", "Two")],
    [],
    "",
    "s2",
  );
  expect(ids(options)).toEqual(["s1"]);
});

it("narrows by title and by the workspace name the conversation is filed under", () => {
  const sessions = [
    session("s1", "Fix the flaky test", { workspaceId: "a", lastMessageAt: 2 }),
    session("s2", "Unrelated", { workspaceId: "b", lastMessageAt: 1 }),
  ];
  const workspaces = [workspace("a", "Payments"), workspace("b", "Renderer")];
  expect(ids(sessionMentionOptions(sessions, workspaces, "flaky"))).toEqual(["s1"]);
  expect(ids(sessionMentionOptions(sessions, workspaces, "payments"))).toEqual(["s1"]);
  expect(ids(sessionMentionOptions(sessions, workspaces, "zzz"))).toEqual([]);
});

it("keeps the recency order while narrowing, so the top row stays the last used", () => {
  const sessions = [
    session("old", "Task old", { lastMessageAt: 1 }),
    session("new", "Task new", { lastMessageAt: 2 }),
  ];
  expect(ids(sessionMentionOptions(sessions, [], "task"))).toEqual(["new", "old"]);
});

it("skips a conversation with no session id, which the reference could not carry", () => {
  const options = sessionMentionOptions([session("", "Nameless"), session("s1", "Real")], []);
  expect(ids(options)).toEqual(["s1"]);
});

// The desktop stores every chat conversation in a temporary workspace of its
// own, named "<title> Workspace" — so a chat has to be told apart by `mode`, or
// its row carries a workspace label that is its own title.
it("labels no workspace on a chat stored under a temporary workspace", () => {
  const options = sessionMentionOptions(
    [
      session("c1", "Fix the flaky test", { mode: "chat", workspaceId: "ws_temp" }),
      session("s1", "Real work", { mode: "workspace", workspaceId: "a" }),
    ],
    [workspace("a", "Dopamine"), workspace("ws_temp", "Fix the flaky test Workspace")],
  );
  expect(options.map(option => [option.sessionId, option.workspace?.name ?? null]))
    .toEqual([
      ["c1", null],
      ["s1", "Dopamine"],
    ]);
});

it("falls back to the row's own workspace name when the workspace list lacks it", () => {
  const options = sessionMentionOptions(
    [session("g1", "Ghost", { workspaceId: "ghost", workspaceName: "Deleted ws" })],
    [],
  );
  expect(options[0]?.workspace).toEqual({ id: "ghost", name: "Deleted ws" });
});
