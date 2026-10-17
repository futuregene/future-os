import { buildSessionTree, catalogRows, groupMergedRows, mergedRows } from "../sessionTree";
import type { MergedSessionRow } from "../../remote/mergeSessions";
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
    // `workspaceId` is the id the machine knows the workspace by — the same id
    // this list folds it under, since here every heading is the active
    // machine's; a merged list's fold key is machine-qualified but the id a
    // command names is not (see mergedRows).
    { kind: "workspace", key: "workspace:w", workspace, workspaceId: "w", count: 2 },
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

/**
 * Two groups with nothing else to tell them apart are ordered by their key.
 *
 * The host orders workspaces by their own place, and two workspaces of the same
 * machine can share that place — or have none at all, which is what a workspace
 * registered but never used looks like. Without a final tie-break their order
 * would come from whichever row happened to arrive first, so the same list could
 * be drawn two ways.
 */
test("merged groups with the same place and machine are ordered by their key", () => {
  const row = (desktopId: string, workspaceId: string, key: string) => ({
    key,
    desktopId,
    lastMessageAt: null,
    session: session(key, undefined, { mode: "workspace", workspaceId }),
  });
  const { workspaces } = groupMergedRows([
    row("desktop_1", "w2", "b"),
    row("desktop_1", "w", "a"),
  ]);
  expect(workspaces.map(group => group.key)).toEqual(["desktop_1::w", "desktop_1::w2"]);
  expect(workspaces.flatMap(group => group.rows.map(item => item.key))).toEqual(["a", "b"]);
});

/**
 * A group's fold belongs to the group, not to whichever machine the phone is
 * talking to.
 *
 * Opening a conversation in a peer's workspace moves the phone onto that
 * machine — the row opens where it lives. While a heading's fold key depended on
 * that choice (the active machine's plain workspace id, every peer's qualified
 * by its machine), that move renamed *every* heading in the list: a group the
 * reader had folded earlier, while that machine was the one being read, came
 * back folded under them and its neighbour opened, with no tap on any heading.
 * Every heading therefore folds by its own machine — the one it lives on, which
 * the phone's position cannot change.
 */
test("a workspace's fold belongs to its machine, not to the phone's place", () => {
  const project = (id: string): RemoteSession =>
    session(id, undefined, { mode: "workspace", workspaceId: "w", workspaceName: "Project" });
  const rows: MergedSessionRow[] = [
    { key: "desk-a::a1", desktopId: "desk-a", lastMessageAt: 1, session: project("a1") },
    { key: "desk-b::b1", desktopId: "desk-b", lastMessageAt: 1, session: project("b1") },
  ];
  const heading = (collapsed: Set<string>) =>
    mergedRows(rows, "", "workspace", collapsed).filter(row => row.kind === "workspace");
  const listed = (collapsed: Set<string>) =>
    mergedRows(rows, "", "workspace", collapsed).flatMap(row =>
      row.kind === "session" ? [row.session.title] : [],
    );

  // Same-named workspaces on two machines, one fold key each — neither plain.
  expect(heading(new Set()).map(row => (row.kind === "workspace" ? row.workspace.id : "")))
    .toEqual(["desk-a::w", "desk-b::w"]);
  // And folding one leaves the other open, whichever machine the phone is on.
  expect(listed(new Set(["desk-a::w"]))).toEqual(["b1"]);
  expect(listed(new Set(["desk-b::w"]))).toEqual(["a1"]);
});
