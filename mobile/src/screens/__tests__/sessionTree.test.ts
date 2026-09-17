import { buildSessionTree, catalogRows } from "../sessionTree";
import type { RemoteSession, RemoteWorkspace } from "../../remote/types";

const session = (
  id: string,
  parentSessionId?: string,
  extra: Partial<RemoteSession> = {},
): RemoteSession => ({
  sessionId: id,
  threadId: `t-${id}`,
  title: id,
  streaming: false,
  parentSessionId,
  ...extra,
});
const workspace: RemoteWorkspace = { id: "w", name: "Project", path: "C:\\work\\project" };
const keys = (rows: ReturnType<typeof catalogRows>) => rows.map(row => row.key);

test("children default to collapsed and expand one level at a time independent of import order", () => {
  const sessions = [session("grandchild", "child"), session("child", "parent"), session("parent")];
  expect(keys(catalogRows(sessions, [], "chat", new Set(), new Set(), ""))).toEqual(["parent"]);
  const rows = catalogRows(sessions, [], "chat", new Set(), new Set(["parent", "child"]), "");
  expect(keys(rows)).toEqual(["parent", "child", "grandchild"]);
  expect(rows.map(row => row.kind === "session" && row.depth)).toEqual([0, 1, 2]);
});

test("missing parents, self-parents, cycles and pinned children never disappear", () => {
  const sessions = [
    session("a", "b"),
    session("b", "a"),
    session("orphan", "missing"),
    session("self", "self"),
    session("pinned", "a", { pinned: true }),
  ];
  const rows = catalogRows(
    sessions,
    [],
    "chat",
    new Set(),
    new Set(sessions.map(s => s.sessionId)),
    "",
  );
  expect(keys(rows).sort()).toEqual(sessions.map(s => s.sessionId).sort());
  expect(buildSessionTree(sessions).map(node => node.session.sessionId)).toContain("pinned");
});

test("deep lineage caps indentation at three levels without losing descendants", () => {
  const sessions = Array.from({ length: 10 }, (_, i) =>
    session(String(i), i ? String(i - 1) : undefined),
  );
  const rows = catalogRows(
    sessions,
    [],
    "chat",
    new Set(),
    new Set(sessions.map(s => s.sessionId)),
    "",
  );
  expect(rows).toHaveLength(10);
  expect(rows.every(row => row.kind === "session" && row.depth <= 2)).toBe(true);
});

test("workspace collapse retains its count and search reveals hidden child matches", () => {
  const sessions = [
    session("parent", undefined, { mode: "workspace", workspaceId: "w" }),
    session("needle", "parent"),
  ];
  expect(catalogRows(sessions, [workspace], "workspace", new Set(["w"]), new Set(), "")).toEqual([
    { kind: "workspace", key: "workspace:w", workspace, count: 2 },
  ]);
  expect(
    keys(catalogRows(sessions, [workspace], "workspace", new Set(["w"]), new Set(), "NEEDLE")),
  ).toEqual(["workspace:w", "needle"]);
  expect(
    keys(catalogRows(sessions, [workspace], "workspace", new Set(["w"]), new Set(), "project")),
  ).toEqual(["workspace:w", "parent", "needle"]);
  expect(catalogRows(sessions, [workspace], "workspace", new Set(), new Set(), "absent")).toEqual(
    [],
  );
  expect(catalogRows(sessions, [workspace], "chat", new Set(), new Set(), "needle")).toEqual([]);
});

const secondWorkspace: RemoteWorkspace = { id: "w2", name: "Second", path: "/work/second" };
const pinnedWorkspaceSessions = () => [
  session("ordinary", undefined, { mode: "workspace", workspaceId: "w" }),
  session("pin-second", undefined, { mode: "workspace", workspaceId: "w2", pinned: true }),
  session("pin-first", undefined, { mode: "workspace", workspaceId: "w", pinned: true }),
  session("pin-child", "pin-first", { mode: "workspace", workspaceId: "w" }),
  session("plain-chat"),
];

test("workspace pins precede all groups in catalog order, without duplicate rows", () => {
  const rows = catalogRows(pinnedWorkspaceSessions(), [workspace, secondWorkspace], "workspace", new Set(), new Set(["pin-first"]), "");
  expect(keys(rows)).toEqual(["pin-second", "pin-first", "pin-child", "workspace:w", "ordinary", "workspace:w2"]);
  expect(new Set(keys(rows)).size).toBe(rows.length);
  expect(rows.filter(row => row.kind === "workspace").map(row => row.count)).toEqual([3, 1]);
  expect(rows[2]).toMatchObject({ depth: 1 });
});

test("collapsed workspaces do not hide their pinned sessions", () => {
  const rows = catalogRows(pinnedWorkspaceSessions(), [workspace, secondWorkspace], "workspace", new Set(["w", "w2"]), new Set(), "");
  expect(keys(rows)).toEqual(["pin-second", "pin-first", "workspace:w", "workspace:w2"]);
});

test("search finds promoted pins by title, workspace name and workspace path", () => {
  const rows = (query: string) => catalogRows(pinnedWorkspaceSessions(), [workspace, secondWorkspace], "workspace", new Set(["w", "w2"]), new Set(), query);
  expect(keys(rows("pin-first"))).toEqual(["pin-first"]);
  expect(keys(rows("pin-child"))).toEqual(["pin-child"]);
  expect(keys(rows("PROJECT"))).toEqual(["pin-first", "pin-child", "workspace:w", "ordinary"]);
  expect(keys(rows("/work/second"))).toEqual(["pin-second", "workspace:w2"]);
  expect(rows("not-found")).toEqual([]);
});

test("unpin restores a child to its parent inside the original workspace", () => {
  const sessions = [
    session("parent", undefined, { mode: "workspace", workspaceId: "w" }),
    session("child", "parent", { mode: "workspace", workspaceId: "w", pinned: true }),
  ];
  expect(keys(catalogRows(sessions, [workspace], "workspace", new Set(), new Set(["parent"]), ""))).toEqual(["child", "workspace:w", "parent"]);
  const unpinned = sessions.map(item => ({ ...item, pinned: false }));
  expect(keys(catalogRows(unpinned, [workspace], "workspace", new Set(), new Set(["parent"]), ""))).toEqual(["workspace:w", "parent", "child"]);
});

test("orphaned workspace pins remain at the page top and chat pins stay on the chat tab", () => {
  const sessions = [
    session("orphan-pin", undefined, { mode: "workspace", workspaceId: "gone", pinned: true }),
    session("chat-pin", undefined, { mode: "chat", pinned: true }),
  ];
  expect(keys(catalogRows(sessions, [], "workspace", new Set(["gone"]), new Set(), ""))).toEqual(["orphan-pin", "workspace:gone"]);
  expect(keys(catalogRows(sessions, [], "chat", new Set(), new Set(), ""))).toEqual(["chat-pin"]);
});

test("pinned workspaces follow the pinned conversations and lead the other groups", () => {
  const sessions = [
    session("pin-session", undefined, { mode: "workspace", workspaceId: "w", pinned: true }),
    session("ordinary", undefined, { mode: "workspace", workspaceId: "w" }),
    session("second", undefined, { mode: "workspace", workspaceId: "w2" }),
  ];
  const rows = catalogRows(
    sessions,
    [workspace, { ...secondWorkspace, pinned: true }],
    "workspace",
    new Set(),
    new Set(),
    "",
  );
  expect(keys(rows)).toEqual([
    "pin-session",
    "workspace:w2",
    "second",
    "workspace:w",
    "ordinary",
  ]);
});

test("among pinned workspaces the desktop's recency order is kept", () => {
  const sessions = [
    session("a", undefined, { mode: "workspace", workspaceId: "w" }),
    session("b", undefined, { mode: "workspace", workspaceId: "w2" }),
  ];
  expect(
    keys(
      catalogRows(
        sessions,
        [{ ...workspace, pinned: true }, { ...secondWorkspace, pinned: true }],
        "workspace",
        new Set(),
        new Set(),
        "",
      ),
    ),
  ).toEqual(["workspace:w", "a", "workspace:w2", "b"]);
});

test("workspace sessions remain accessible if their workspace is absent", () => {
  expect(
    keys(
      catalogRows(
        [session("s", undefined, { mode: "workspace", workspaceId: "gone" })],
        [],
        "workspace",
        new Set(),
        new Set(),
        "",
      ),
    ),
  ).toEqual(["workspace:gone", "s"]);
});
