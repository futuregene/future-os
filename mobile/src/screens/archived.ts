import type { RemoteSession, RemoteWorkspace } from "../remote/types";

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
