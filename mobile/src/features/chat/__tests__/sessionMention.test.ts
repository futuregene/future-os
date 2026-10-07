import type { RemoteSession, RemoteWorkspace } from "../../../remote/types";
import { sessionMentionGroups } from "../sessionMention";

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

const ids = (groups: ReturnType<typeof sessionMentionGroups>) =>
  groups.flatMap(group => group.sessions.map(item => item.sessionId));

it("files the conversations the way the session list does", () => {
  const groups = sessionMentionGroups(
    [
      session("c1", "Loose chat", { mode: "chat" }),
      session("s1", "Clean the data", { workspaceId: "a" }),
      session("s2", "Run the baseline", { workspaceId: "b" }),
    ],
    [workspace("a", "Dopamine"), workspace("b", "Sparse attention")],
  );
  expect(groups.map(group => group.workspace?.name ?? null))
    .toEqual([null, "Dopamine", "Sparse attention"]);
  expect(ids(groups)).toEqual(["c1", "s1", "s2"]);
});

it("drops the conversation being composed in, so nothing references itself", () => {
  const groups = sessionMentionGroups(
    [session("s1", "One"), session("s2", "Two")],
    [],
    "",
    "s2",
  );
  expect(ids(groups)).toEqual(["s1"]);
});

it("narrows by title and by the workspace name the conversation is filed under", () => {
  const sessions = [
    session("s1", "Fix the flaky test", { workspaceId: "a" }),
    session("s2", "Unrelated", { workspaceId: "b" }),
  ];
  const workspaces = [workspace("a", "Payments"), workspace("b", "Renderer")];
  expect(ids(sessionMentionGroups(sessions, workspaces, "flaky"))).toEqual(["s1"]);
  expect(ids(sessionMentionGroups(sessions, workspaces, "payments"))).toEqual(["s1"]);
  expect(ids(sessionMentionGroups(sessions, workspaces, "zzz"))).toEqual([]);
});

it("skips a conversation with no session id, which the reference could not carry", () => {
  const groups = sessionMentionGroups([session("", "Nameless"), session("s1", "Real")], []);
  expect(ids(groups)).toEqual(["s1"]);
});
