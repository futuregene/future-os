import type { StoredThread } from "../../integrations/storage/threadStore";

/** One archived row and the batch (scope) it is addressed by. */
export interface ArchivedScopeEntry {
  scope: string;
  thread: StoredThread;
}

/** The batch a row of the archived conversations section belongs to. */
export const ARCHIVED_SCOPE = "archived";

/** The batch one shelved workspace's contents belong to. */
export function archivedWorkspaceScope(workspaceId: string): string {
  return `archived-workspace:${workspaceId}`;
}

/**
 * The archived view's rows, one entry per conversation.
 *
 * Two streams feed the archived list: the shelved conversations, and the
 * contents of each shelved workspace the reader has opened. They are filled
 * separately (a group by its own paged read), so one conversation can be
 * described by both — and this list is the batch a selection addresses, so a
 * duplicate is a duplicate in the toolbar's count and in the names the delete
 * dialog lists.
 *
 * The first description wins: a shelved conversation is listed before any
 * group's contents, so it keeps the scope of the section it is drawn under.
 */
export function archiveScopeEntries(
  archivedThreads: readonly StoredThread[],
  workspaceThreads: Readonly<Record<string, readonly StoredThread[]>>,
): ArchivedScopeEntry[] {
  const entries: ArchivedScopeEntry[] = [
    ...archivedThreads.map(thread => ({ scope: ARCHIVED_SCOPE, thread })),
    ...Object.entries(workspaceThreads).flatMap(([workspaceId, rows]) =>
      rows.map(thread => ({ scope: archivedWorkspaceScope(workspaceId), thread }))),
  ];
  const seen = new Set<string>();
  return entries.filter((entry) => {
    if (seen.has(entry.thread.id))
      return false;
    seen.add(entry.thread.id);
    return true;
  });
}

/**
 * Which batch each archived row belongs to, by conversation id.
 *
 * A row belongs to the batch of the heading it is DRAWN under, which is not
 * always the scope it was first described with: a shelved workspace's contents
 * are drawn inside that workspace's group, and the group opens the batch for
 * `archived-workspace:<id>`. A conversation the conversations stream also
 * listed keeps its first description in {@link archiveScopeEntries} — but the
 * row the reader ticks is the one under the heading, so that is the scope it
 * must belong to. Giving it the other one left it outside every batch: the
 * checkbox ticked, the toolbar read "1 selected", and both Delete and Restore
 * did nothing because the open batch held no row it could address.
 */
export function archiveThreadScopes(
  archivedThreads: readonly StoredThread[],
  workspaceThreads: Readonly<Record<string, readonly StoredThread[]>>,
): Map<string, string> {
  const scopes = new Map<string, string>(
    archiveScopeEntries(archivedThreads, workspaceThreads)
      .map(entry => [entry.thread.id, entry.scope] as const),
  );
  // A group's own rows win for the ids that group draws.
  for (const [workspaceId, rows] of Object.entries(workspaceThreads)) {
    for (const thread of rows)
      scopes.set(thread.id, archivedWorkspaceScope(workspaceId));
  }
  return scopes;
}
