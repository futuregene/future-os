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
