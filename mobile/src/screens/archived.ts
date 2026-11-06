import type { MachineSummary } from "../remote/desktopStatus";
import type { DesktopFilter } from "../remote/mergeSessions";
import type { RemoteSession, RemoteWorkspace } from "../remote/types";
import type { CatalogRow } from "./sessionTree";
import { archivedRows } from "./sessionTree";

/**
 * Archive state of a remote row, in one place.
 *
 * The desktop hides archived conversations and archived workspaces from its
 * default lists, and the phone's pages have to agree — otherwise the same
 * conversation appears in two different places depending on which device is
 * looking at it. The host reports the fact on each row (`archived`), and a
 * desktop too old for the field simply reports nothing, which reads as active:
 * exactly what such a desktop's own list still did.
 *
 * The archived rows stay in the catalogue (the host does not filter them out),
 * so the phone can show them in its own archived view without a second round
 * trip — which is what this module is for.
 */
/**
 * How many archived conversations the archived view draws at a time.
 *
 * Ten, like the desktop's archived view: the number belongs to the feature and
 * not to the device, so a shelf opened on a phone is paged the same way it is
 * on a desktop.
 */
export const ARCHIVED_PAGE_SIZE = 10;

export function isArchivedSession(session: RemoteSession): boolean {
  return session.archived === true;
}

export function isArchivedWorkspace(workspace: RemoteWorkspace): boolean {
  return workspace.archived === true;
}

/**
 * Split rows into the two lists the app draws: what the default pages show, and
 * what the archived view shows.
 *
 * A workspace's conversations are archived with it — the host already marks each
 * session row for that (its thread is active, its workspace is shelved), so this
 * split is one comparison per row and no join.
 */
export function splitArchived<T extends { session: RemoteSession; }>(
  rows: readonly T[],
): { active: T[]; archived: T[] } {
  const active: T[] = [];
  const archived: T[] = [];
  for (const row of rows) {
    if (isArchivedSession(row.session)) archived.push(row);
    else active.push(row);
  }
  return { active, archived };
}

/**
 * The workspaces the default pages may draw: archived ones are hidden, because
 * their conversations are too.
 */
export function activeWorkspaces(workspaces: readonly RemoteWorkspace[]): RemoteWorkspace[] {
  return workspaces.filter(workspace => !isArchivedWorkspace(workspace));
}

/** One machine's half of the archived shelf. */
export interface ArchivedCatalog {
  desktopId: string;
  sessions: RemoteSession[];
  /**
   * That machine's workspaces when this phone has them.
   *
   * The shelf can still group without them: an archived conversation carries the
   * name of the workspace holding it, and `archivedCatalogRows` derives headings
   * from those rows. What only this list can supply is a shelved workspace with
   * nothing left inside it — the one row whose only action is "restore".
   */
  workspaces?: RemoteWorkspace[];
}

/**
 * Shelved workspaces derived from the conversations that name them.
 *
 * A host's workspace list is the authority; this is the fallback for a machine
 * whose list has not been read (a peer's workspaces are not part of its session
 * catalogue). Without it, every archived conversation of a shelved workspace
 * would be drawn loose — each one correct, but filed under nothing, which is the
 * one thing the shelf is for.
 *
 * The derivation fills gaps, it does not shadow the list: an id the machine
 * already named is skipped, however that workspace is marked.
 */
function derivedArchivedWorkspaces(
  sessions: readonly RemoteSession[],
  workspaces: readonly RemoteWorkspace[],
): RemoteWorkspace[] {
  if (workspaces.some(workspace => workspace.archived === true)) return [...workspaces];
  const known = new Set(workspaces.map(workspace => workspace.id));
  const byId = new Map<string, RemoteWorkspace>();
  for (const session of sessions) {
    if (session.archived !== true || session.mode !== "workspace" || !session.workspaceId) continue;
    // Only an id the machine's list does not already have needs deriving. A
    // workspace that is in use and merely holds a shelved conversation is in
    // that list, and synthesizing a twin drew the same folder twice on the shelf
    // (115 build: one archived conversation under `case` produced two `case`
    // headings, each opening onto that same conversation).
    if (known.has(session.workspaceId) || byId.has(session.workspaceId)) continue;
    byId.set(session.workspaceId, {
      id: session.workspaceId,
      name: session.workspaceName ?? "",
      path: "",
      pinned: false,
      // Not `archived: true`: that marker means the reader shelved the *folder*,
      // which is exactly what an in-use workspace is not. The heading is earned
      // by the shelved conversation under it (see `archivedRows`).
      archived: false,
    });
  }
  return [...workspaces, ...byId.values()];
}

/**
 * The archived shelf across every machine the list is showing.
 *
 * The desktop's archived view holds all of its paired machines' shelves; the
 * phone's held only the open connection's, so a conversation shelved on another
 * pairing was missing from the one page meant to list it. Each machine's rows
 * carry their source, which is what draws the machine glyph and routes a restore
 * back to the machine that owns the row.
 *
 * `collapsed` is the same fold set the active pages use; a search passes an
 * empty set, so every shelf is open while the reader is looking for something.
 */
export function archivedCatalogRows(
  catalogs: readonly ArchivedCatalog[],
  collapsed: ReadonlySet<string>,
): CatalogRow[] {
  return catalogs.flatMap(catalog => archivedRows(
    catalog.sessions,
    derivedArchivedWorkspaces(catalog.sessions, catalog.workspaces ?? []),
    () => true,
    collapsed,
    catalog.desktopId,
  ));
}

/**
 * The machines whose shelves the archived view cannot read.
 *
 * The shelf draws the catalogues of machines that are `connected` (see
 * `CatalogEntry.status`), which makes an empty shelf a statement about those
 * machines only: "nothing archived on the machines this phone can read". A
 * reader whose archive sits on a machine that is not answering — asleep, off the
 * network, its agent restarting — was told "nothing archived yet" instead, and a
 * page that says an archive does not exist is a page they cannot act on.
 *
 * So the empty state names them. `filter` is the picker's own scope: a machine
 * the reader filtered out is not one they are asking about.
 */
export function unreadableShelfMachines(
  machines: readonly MachineSummary[],
  filter: DesktopFilter,
): MachineSummary[] {
  const scope = filter.kind === "desktop"
    ? machines.filter(machine => machine.desktopId === filter.desktopId)
    : machines;
  return scope.filter(machine => machine.status !== "connected");
}
