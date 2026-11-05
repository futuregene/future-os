import type { MachineStatus, MachineSummary } from "../../remote/desktopStatus";
import type { RemoteSession, RemoteWorkspace } from "../../remote/types";
import {
  activeWorkspaces,
  archivedCatalogRows,
  isArchivedSession,
  isArchivedWorkspace,
  splitArchived,
  unreadableShelfMachines,
} from "../archived";
import {
  archivedRows,
  filterArchivedRows,
  pageArchivedRows,
  type CatalogRow,
} from "../sessionTree";

const session = (
  id: string,
  extra: Partial<RemoteSession> = {},
): RemoteSession => ({
  sessionId: id,
  threadId: `t-${id}`,
  title: id,
  streaming: false,
  ...extra,
});
const workspace = (
  id: string,
  extra: Partial<RemoteWorkspace> = {},
): RemoteWorkspace => ({ id, name: id, path: `C:\\work\\${id}`, ...extra });

const never = () => false;
const keys = (rows: CatalogRow[]) => rows.map(row => row.key);
const sessionKeys = (rows: CatalogRow[]) =>
  rows.filter(row => row.kind === "session").map(row => row.key);

describe("what counts as archived", () => {
  it("is the row's own flag, and nothing else", () => {
    expect(isArchivedSession(session("a", { archived: true }))).toBe(true);
    expect(isArchivedSession(session("a"))).toBe(false);
    // A desktop too old to report the field says nothing, which reads as active
    // — exactly what that desktop's own list still did.
    expect(isArchivedSession(session("a", { archived: undefined }))).toBe(false);
    expect(isArchivedWorkspace(workspace("w", { archived: true }))).toBe(true);
    expect(isArchivedWorkspace(workspace("w"))).toBe(false);
  });

  it("splits a list in two without dropping a row", () => {
    const { active, archived } = splitArchived([
      { session: session("a") },
      { session: session("b", { archived: true }) },
      { session: session("c") },
    ]);
    expect(active.map(row => row.session.sessionId)).toEqual(["a", "c"]);
    expect(archived.map(row => row.session.sessionId)).toEqual(["b"]);
  });

  it("hides archived workspaces from the default pages", () => {
    // Their conversations are hidden too, so a heading over none of them would
    // be a group the reader could open and find nothing in.
    expect(activeWorkspaces([workspace("open"), workspace("shelved", { archived: true })])
      .map(item => item.id)).toEqual(["open"]);
  });
});

describe("the archived shelf's rows", () => {
  it("draws a shelved workspace with its conversations, then the loose ones", () => {
    const rows = archivedRows(
      [
        session("in-shelf", { mode: "workspace", workspaceId: "w", archived: true }),
        session("loose", { archived: true }),
        // Archived because its workspace is, even though its own thread is not.
        session("carried", { mode: "workspace", workspaceId: "w", archived: true }),
        session("active", { mode: "workspace", workspaceId: "w" }),
      ],
      [workspace("w", { archived: true })],
      () => true,
    );
    expect(keys(rows)).toEqual([
      "archived-workspace:w",
      "in-shelf",
      "carried",
      "loose",
    ]);
    const heading = rows[0];
    expect(heading?.kind).toBe("workspace");
    expect(heading?.kind === "workspace" ? heading.count : null).toBe(2);
    // A conversation under a shelf sits one level in; a loose one starts at 0.
    expect(rows.map(row => (row.kind === "session" ? row.depth : "heading")))
      .toEqual(["heading", 1, 1, 0]);
  });

  it("files a workspace's shelved conversation under that workspace's heading", () => {
    const rows = archivedRows(
      [
        session("one", { mode: "workspace", workspaceId: "w", archived: true }),
        session("chat", { archived: true }),
      ],
      // The workspace itself is still in the active list; its heading earns its
      // place from the shelved row under it.
      [workspace("w", { name: "Live project" })],
      () => true,
    );
    expect(keys(rows)).toEqual(["archived-workspace:w", "one", "chat"]);
    const heading = rows[0];
    expect(heading?.kind).toBe("workspace");
    if (heading?.kind === "workspace") {
      expect(heading.workspace.name).toBe("Live project");
      expect(heading.count).toBe(1);
    }
  });

  it("leaves out conversations the caller does not own and rows that are active", () => {
    const rows = archivedRows(
      [
        session("elsewhere", { archived: true }),
        session("here", { archived: true }),
      ],
      [],
      item => item.sessionId === "here",
    );
    expect(sessionKeys(rows)).toEqual(["here"]);
  });

  it("folds a shelved workspace by the same id the active pages fold it by", () => {
    const sessions = [
      session("a", { mode: "workspace", workspaceId: "w", archived: true }),
    ];
    const open = archivedRows(sessions, [workspace("w", { archived: true })], () => true);
    expect(keys(open)).toEqual(["archived-workspace:w", "a"]);
    const folded = archivedRows(
      sessions,
      [workspace("w", { archived: true })],
      () => true,
      new Set(["w"]),
    );
    // The heading stays with its count, so a folded shelf still says how much is
    // behind it — the active pages' behaviour for a folded group.
    expect(keys(folded)).toEqual(["archived-workspace:w"]);
    expect(folded[0]?.kind === "workspace" ? folded[0].count : null).toBe(1);
  });

  it("shows both sections' empty case as an empty list", () => {
    expect(archivedRows([session("active")], [workspace("w")], () => true)).toEqual([]);
    expect(archivedRows([], [], never)).toEqual([]);
  });
});

describe("the shelf across machines", () => {
  it("draws each machine's half, stamped with the machine that owns it", () => {
    const rows = archivedCatalogRows([
      { desktopId: "desk-a", sessions: [session("a", { archived: true })], workspaces: [] },
      {
        desktopId: "desk-b",
        sessions: [session("b", { mode: "workspace", workspaceId: "w-b", workspaceName: "B project", archived: true })],
        workspaces: [],
      },
    ], new Set());

    // The second machine's workspace list was never read, yet its shelved
    // conversation still gets the heading named by its row — and both rows keep
    // distinct keys, so one FlatList can hold them.
    expect(rows.map(row => row.key)).toEqual(["desk-a::a", "archived-workspace:desk-b:w-b", "desk-b::b"]);
    expect(rows.map(row => (row.kind === "section" ? undefined : row.desktopId)))
      .toEqual(["desk-a", "desk-b", "desk-b"]);
    const heading = rows[1];
    expect(heading?.kind === "workspace" ? heading.workspace.name : null).toBe("B project");
  });

  it("prefers the machine's own workspace list when it has one", () => {
    const rows = archivedCatalogRows([
      {
        desktopId: "desk-a",
        sessions: [],
        // A shelved workspace with nothing left inside it exists only here.
        workspaces: [workspace("w-empty", { name: "Empty shelf", archived: true })],
      },
    ], new Set());
    expect(rows.map(row => row.key)).toEqual(["archived-workspace:desk-a:w-empty"]);
  });

  it("honours the fold set by workspace id, as the active pages do", () => {
    const rows = archivedCatalogRows([
      {
        desktopId: "desk-a",
        sessions: [session("a", { mode: "workspace", workspaceId: "w", archived: true })],
        workspaces: [workspace("w", { archived: true })],
      },
    ], new Set(["w"]));
    expect(rows.map(row => row.kind)).toEqual(["workspace"]);
  });
});

describe("searching the shelf", () => {
  const rows = archivedRows(
    [
      session("alpha", { mode: "workspace", workspaceId: "w", archived: true }),
      session("beta", { mode: "workspace", workspaceId: "w", archived: true }),
      session("gamma", { archived: true }),
    ],
    [workspace("w", { archived: true })],
    () => true,
  );

  it("keeps the shelf when its own name matches", () => {
    // The reader asked for the shelf, so all of its conversations come with it.
    expect(sessionKeys(filterArchivedRows(rows, "w"))).toEqual(["alpha", "beta"]);
  });

  it("keeps only the conversations that match", () => {
    expect(sessionKeys(filterArchivedRows(rows, "alpha"))).toEqual(["alpha"]);
    expect(sessionKeys(filterArchivedRows(rows, "gamma"))).toEqual(["gamma"]);
  });

  it("drops a shelf with nothing left rather than drawing an empty heading", () => {
    const filtered = filterArchivedRows(rows, "gamma");
    expect(keys(filtered)).toEqual(["gamma"]);
    expect(filtered.some(row => row.kind === "workspace")).toBe(false);
  });

  it("is case-insensitive and ignores a blank query", () => {
    expect(sessionKeys(filterArchivedRows(rows, "ALPHA"))).toEqual(["alpha"]);
    expect(filterArchivedRows(rows, "   ")).toBe(rows);
  });
});

describe("paging the shelf", () => {
  const many = (count: number) => Array.from(
    { length: count },
    (_, index) => session(`s${index}`, { archived: true }),
  );

  it("shows the first ten conversations and counts the rest", () => {
    const { rows, more } = pageArchivedRows(archivedRows(many(13), [], () => true), 10);
    expect(sessionKeys(rows)).toHaveLength(10);
    expect(more).toBe(3);
  });

  it("draws whole shelves, never a heading over nothing", () => {
    const sessions = [
      session("a", { mode: "workspace", workspaceId: "w1", archived: true }),
      session("b", { mode: "workspace", workspaceId: "w1", archived: true }),
      session("c", { mode: "workspace", workspaceId: "w2", archived: true }),
      session("d", { mode: "workspace", workspaceId: "w2", archived: true }),
    ];
    const rows = archivedRows(
      sessions,
      [workspace("w1", { archived: true }), workspace("w2", { archived: true })],
      () => true,
    );
    // A limit of three cannot hold two whole shelves of two, so the window ends
    // after the first shelf instead of cutting the second in half.
    const page = pageArchivedRows(rows, 3);
    expect(keys(page.rows)).toEqual(["archived-workspace:w1", "a", "b"]);
    expect(page.more).toBe(2);
  });

  it("always draws the first shelf, however large", () => {
    // Otherwise a limit smaller than one shelf would leave the reader a page
    // with no footer to open any further.
    const sessions = Array.from(
      { length: 12 },
      (_, index) => session(`s${index}`, { mode: "workspace", workspaceId: "w", archived: true }),
    );
    const rows = archivedRows(sessions, [workspace("w", { archived: true })], () => true);
    const page = pageArchivedRows(rows, 10);
    expect(sessionKeys(page.rows)).toHaveLength(12);
    expect(page.more).toBe(0);
  });

  it("counts nothing held back when the list fits", () => {
    const page = pageArchivedRows(archivedRows(many(2), [], () => true), 10);
    expect(sessionKeys(page.rows)).toHaveLength(2);
    expect(page.more).toBe(0);
  });
});

/**
 * An empty shelf is a statement about the machines the phone can read, not about
 * the archive: the page drew its catalogues only for machines that are
 * `connected`, and then worded the result as "nothing archived yet".
 */
describe("the machines whose shelves cannot be read", () => {
  const machine = (desktopId: string, status: MachineStatus): MachineSummary =>
    ({ desktopId, status, label: desktopId, active: false });

  it("names the paired machines that are not answering", () => {
    const machines = [
      machine("here", "connected"),
      machine("office", "offline"),
      machine("laptop", "preparing"),
    ];
    expect(unreadableShelfMachines(machines, { kind: "all" }).map(item => item.desktopId))
      .toEqual(["office", "laptop"]);
  });

  it("says nothing on a shelf where every machine answers", () => {
    expect(unreadableShelfMachines([machine("here", "connected")], { kind: "all" })).toEqual([]);
  });

  it("follows the picker's scope: a filtered-out machine is not one being asked about", () => {
    const machines = [machine("here", "connected"), machine("office", "offline")];
    expect(unreadableShelfMachines(machines, { kind: "desktop", desktopId: "here" })).toEqual([]);
    expect(unreadableShelfMachines(machines, { kind: "desktop", desktopId: "office" }))
      .toEqual([machine("office", "offline")]);
  });
});
