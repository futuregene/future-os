import type { MergedSessionRow } from "../remote/mergeSessions";
import { effectiveRunStatus } from "../remote/sessionStatus";
import type { CatalogRow } from "./sessionTree";

/**
 * The recency page: every conversation on the list, newest reply first, split
 * into what is still producing output and what is not.
 *
 * This page exists because the other two answer "where does this live" — a
 * workspace tree, a list of loose chats — and neither answers "what did I last
 * touch". So the page ignores the workspace tree, forks and pinning for its
 * *order*: one rule, the time of the last reply. A fork is therefore a row of
 * its own here (it has its own reply time, and there is no parent row above it
 * to indent under), and a pinned conversation keeps its marker but not its
 * place at the top — the same sessions are still a shortcut on the other two
 * pages.
 *
 * "Active" is a run that has not finished: producing output, queued to start,
 * or waiting for the reader's approval. The last one matters most — a run
 * blocked on a question is the one place a reader is being waited on, and it
 * is *not* streaming, so a page that read `streaming` alone would file it under
 * "earlier" and make them hunt for it.
 */

/** Run states that mean "this conversation is not done yet". */
const ACTIVE_STATUSES = new Set(["running", "queued", "waiting_approval"]);

/** Whether a run is still in flight, by the same rule the rows' icons use. */
export function isActiveSession(row: MergedSessionRow): boolean {
  const status = effectiveRunStatus(row.session.status, row.session.streaming);
  return status !== undefined && ACTIVE_STATUSES.has(status);
}

/** Newest reply first; a row with no stamp sorts below every dated row. */
function byRecency(a: MergedSessionRow, b: MergedSessionRow): number {
  const at = a.lastMessageAt ?? Number.NEGATIVE_INFINITY;
  const bt = b.lastMessageAt ?? Number.NEGATIVE_INFINITY;
  if (at !== bt) return bt - at;
  return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
}

function sessionRow(row: MergedSessionRow): CatalogRow {
  return {
    kind: "session",
    key: row.key,
    session: row.session,
    // No tree on this page: every conversation is a row of its own.
    depth: 0,
    hasChildren: false,
    foldId: row.key,
    ...(row.desktopId ? { desktopId: row.desktopId } : {}),
  };
}

/**
 * The page's rows.
 *
 * A search flattens it, as on the other two pages: captions are about *where a
 * conversation sits* in the page's order, and a filtered list has no order to
 * caption. A caption with nothing under it is never drawn either.
 */
export function recentRows(
  rows: readonly MergedSessionRow[],
  query: string,
): CatalogRow[] {
  const search = query.trim().toLocaleLowerCase();
  const matches = (row: MergedSessionRow): boolean =>
    !search
    || row.session.title.toLocaleLowerCase().includes(search)
    || (row.session.workspaceName ?? "").toLocaleLowerCase().includes(search);
  const listed = rows.filter(matches);
  const active = listed.filter(isActiveSession).sort(byRecency);
  const earlier = listed.filter(row => !isActiveSession(row)).sort(byRecency);
  if (search) return [...active, ...earlier].map(sessionRow);
  const out: CatalogRow[] = [];
  for (const [titleKey, section] of [
    ["sessions.activeSection", active],
    ["sessions.earlierSection", earlier],
  ] as const) {
    // An empty stretch is not a section: a caption over nothing reads as a list
    // that failed to load its rows.
    if (section.length === 0) continue;
    out.push({
      kind: "section",
      key: `recent:${titleKey}`,
      titleKey,
      count: section.length,
    });
    out.push(...section.map(sessionRow));
  }
  return out;
}
