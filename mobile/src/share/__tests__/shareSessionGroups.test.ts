import type { RemoteSession, RemoteWorkspace } from "../../remote/types";
import { shareSessionGroups } from "../shareSessionGroups";

const session = (sessionId: string, title: string, extra: Partial<RemoteSession> = {}): RemoteSession => ({
  sessionId,
  threadId: `t_${sessionId}`,
  title,
  mode: "workspace",
  streaming: false,
  ...extra,
});
const chat = (sessionId: string, title: string) => session(sessionId, title, { mode: "chat" });
const workspace = (id: string, name: string, pinned = false): RemoteWorkspace => ({
  id,
  name,
  path: `/p/${id}`,
  pinned,
});
const titles = (sessions: RemoteSession[]) => sessions.map(item => item.title);

it("files a workspace's conversations under it, roots first, in catalogue order", () => {
  const groups = shareSessionGroups(
    [
      chat("c1", "How do I write this?"),
      session("s1", "Clean the data", { workspaceId: "a" }),
      session("s2", "Run the baseline", { workspaceId: "b" }),
      session("s3", "Find markers", { workspaceId: "a" }),
    ],
    [workspace("a", "Dopamine"), workspace("b", "Sparse attention")],
  );
  expect(groups.map(group => group.workspace?.name ?? null)).toEqual([null, "Dopamine", "Sparse attention"]);
  expect(titles(groups[0]!.sessions)).toEqual(["How do I write this?"]);
  expect(titles(groups[1]!.sessions)).toEqual(["Clean the data", "Find markers"]);
  expect(titles(groups[2]!.sessions)).toEqual(["Run the baseline"]);
});

it("keeps the pinned workspace ahead of the rest, as the session list does", () => {
  const groups = shareSessionGroups(
    [
      session("s1", "Run the baseline", { workspaceId: "b" }),
      session("s2", "Clean the data", { workspaceId: "a" }),
    ],
    [workspace("a", "Dopamine", true), workspace("b", "Sparse attention")],
  );
  expect(groups.map(group => group.workspace?.name)).toEqual(["Dopamine", "Sparse attention"]);
});

it("keeps conversations whose workspace is gone, in a group the caller can name", () => {
  const groups = shareSessionGroups(
    [session("s1", "Clean the data", { workspaceId: "deleted" })],
    [workspace("a", "Dopamine")],
  );
  expect(groups).toHaveLength(1);
  expect(groups[0]!.workspace).toEqual({ id: "deleted", name: "", path: "" });
  expect(titles(groups[0]!.sessions)).toEqual(["Clean the data"]);
});

it("matches a title, and every conversation of a matching workspace", () => {
  const sessions = [
    chat("c1", "Explain p-values"),
    session("s1", "Clean the data", { workspaceId: "a" }),
    session("s2", "Find markers", { workspaceId: "a" }),
    session("s3", "Run the baseline", { workspaceId: "b" }),
  ];
  const workspaces = [workspace("a", "Dopamine"), workspace("b", "Sparse attention")];
  const title = shareSessionGroups(sessions, workspaces, "markers");
  expect(title.map(group => group.workspace?.name ?? null)).toEqual(["Dopamine"]);
  expect(titles(title[0]!.sessions)).toEqual(["Find markers"]);
  const byWorkspace = shareSessionGroups(sessions, workspaces, "dopamine");
  expect(byWorkspace.map(group => group.workspace?.name)).toEqual(["Dopamine"]);
  expect(titles(byWorkspace[0]!.sessions)).toEqual(["Clean the data", "Find markers"]);
  expect(shareSessionGroups(sessions, workspaces, "nothing like this")).toEqual([]);
});

it("ignores case and surrounding whitespace, like the session list's search", () => {
  const groups = shareSessionGroups(
    [session("s1", "Run the Baseline", { workspaceId: "b" })],
    [workspace("b", "Sparse Attention")],
    "  SPARSE  ",
  );
  expect(titles(groups[0]!.sessions)).toEqual(["Run the Baseline"]);
});
