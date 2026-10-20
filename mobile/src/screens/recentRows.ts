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

/**
 * How many finished conversations the page draws before it stops and offers the
 * rest.
 *
 * A reader with a few hundred conversations should not have to scroll past all
 * of them to reach the bottom of a page whose point is "the last few things".
 * So the finished stretch is a window, and the window opens further on request.
 * The active stretch is never windowed: it is bounded by what is actually
 * running, and it is the part the reader came to see.
 */
export const RECENT_PAGE_SIZE = 20;

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

function sessionRow(row: MergedSessionRow, section: "active" | "earlier"): CatalogRow {
  return {
    kind: "session",
    key: row.key,
    session: row.session,
    // No tree on this page: every conversation is a row of its own.
    depth: 0,
    hasChildren: false,
    foldId: row.key,
    section,
    ...(row.desktopId ? { desktopId: row.desktopId } : {}),
  };
}

export interface RecentPage {
  rows: CatalogRow[];
  /** Finished conversations the window is holding back; 0 when it holds them all. */
  more: number;
}

/**
 * The page's rows, with the finished stretch windowed to `limit`.
 *
 * A search flattens it, as on the other two pages: captions are about *where a
 * conversation sits* in the page's order, and a filtered list has no order to
 * caption. A search also ignores the window — the thing being looked for may be
 * the two-hundredth conversation, and a result list that said "not found" while
 * holding it would be wrong.
 *
 * A caption counts its whole stretch, not the window, so "Earlier 42" stays true
 * whether twenty or all of them are drawn; `more` is what the footer under them
 * offers.
 */
export function recentRows(
  rows: readonly MergedSessionRow[],
  query: string,
  limit: number = RECENT_PAGE_SIZE,
): RecentPage {
  const search = query.trim().toLocaleLowerCase();
  const matches = (row: MergedSessionRow): boolean =>
    !search
    || row.session.title.toLocaleLowerCase().includes(search)
    || (row.session.workspaceName ?? "").toLocaleLowerCase().includes(search);
  const listed = rows.filter(matches);
  const active = listed.filter(isActiveSession).sort(byRecency);
  const earlier = listed.filter(row => !isActiveSession(row)).sort(byRecency);
  if (search)
    return {
      rows: [
        ...active.map(row => sessionRow(row, "active")),
        ...earlier.map(row => sessionRow(row, "earlier")),
      ],
      more: 0,
    };
  const shown = limit < 0 ? earlier : earlier.slice(0, Math.max(0, limit));
  const out: CatalogRow[] = [];
  const section = (
    titleKey: "sessions.activeSection" | "sessions.earlierSection",
    kind: "active" | "earlier",
    all: readonly MergedSessionRow[],
    drawn: readonly MergedSessionRow[],
  ) => {
    // An empty stretch is not a section: a caption over nothing reads as a list
    // that failed to load its rows. A window that has not opened yet is not one
    // either — the caption counts the whole stretch, so a caption over no rows
    // would claim conversations it does not show.
    if (drawn.length === 0) return;
    out.push({ kind: "section", key: `recent:${titleKey}`, titleKey, count: all.length });
    out.push(...drawn.map(row => sessionRow(row, kind)));
  };
  section("sessions.activeSection", "active", active, active);
  section("sessions.earlierSection", "earlier", earlier, shown);
  return { rows: out, more: earlier.length - shown.length };
}
