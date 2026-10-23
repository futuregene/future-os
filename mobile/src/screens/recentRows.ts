import type { MergedSessionRow } from "../remote/mergeSessions";
import { effectiveRunStatus } from "../remote/sessionStatus";
import type { CatalogRow } from "./sessionTree";

/**
 * The recency page: every conversation on the list, newest reply first, split
 * into what is still producing output and what is not.
 *
 * This page exists because the other two answer "where does this live" — a
 * workspace tree, a list of loose chats — and neither answers "what did I last
 * touch". So the page ignores the workspace tree and pinning for its *order*:
 * one rule, the time of the last reply (and the start time for the rows still in
 * flight). A pinned conversation keeps its marker but not its place at the top —
 * the same sessions are still a shortcut on the other two pages.
 *
 * **A conversation's child sessions stay under it, as on the other two pages.**
 * A fork, and a session an agent started for one piece of work (a `/future-goal`
 * worker), is not a conversation the reader scans this page for — it is the work
 * of one they did. So the stretches rank only conversations whose parent is not
 * on this page, a parent carries the same `+` / `−` tree toggle the other pages
 * give it, and its children are drawn — indented, up to the same three levels —
 * only while it is open. A child is never a row of either stretch on its own: a
 * running worker does not lift its conversation into "Active", and the captions
 * count conversations rather than the sessions under them. The fold is the one
 * the other pages use (the same `expanded` set), so a conversation opened here is
 * open there too.
 *
 * Search is the exception that keeps a match reachable: it flattens the page as
 * the other two do, so a child whose parent does not match is drawn as a row at
 * the top level rather than hidden behind a conversation that did not match.
 *
 * "Active" is a run that has not finished: producing output, queued to start,
 * or waiting for the reader's approval. The last one matters most — a run
 * blocked on a question is the one place a reader is being waited on, and it
 * is *not* streaming, so a page that read `streaming` alone would file it under
 * "earlier" and make them hunt for it.
 *
 * The two stretches are ordered by different clocks, on purpose. "Earlier" is
 * ordered by the last reply, which is the question that page answers. "Active"
 * is ordered by when each conversation *started*: a streaming run rewrites the
 * last-reply stamp about once a second, so ranking the running ones by it made
 * rows swap places under the reader's thumb while a handful of runs took turns
 * producing output. A start time does not move, so a row keeps its place for the
 * whole run and a row joining or leaving the stretch is the only thing that
 * shifts the others.
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

/**
 * The deepest indent the page draws.
 *
 * The same three levels the other two pages flatten to (see `buildSessionTree`):
 * as much indentation as a phone row has room for. A deeper chain keeps its rows
 * and stops indenting rather than eating the title's width.
 */
const MAX_DEPTH = 2;

/** Whether a run is still in flight, by the same rule the rows' icons use. */
export function isActiveSession(row: MergedSessionRow): boolean {
  const status = effectiveRunStatus(row.session.status, row.session.streaming);
  return status !== undefined && ACTIVE_STATUSES.has(status);
}

/** Newest first on one stamp; a row without it sorts below every dated row. */
function byStamp(
  at: number | null,
  bt: number | null,
  a: MergedSessionRow,
  b: MergedSessionRow,
): number {
  const left = at ?? Number.NEGATIVE_INFINITY;
  const right = bt ?? Number.NEGATIVE_INFINITY;
  if (left !== right) return right - left;
  return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
}

/** Newest reply first — the "earlier" stretch's order. */
const byRecency = (a: MergedSessionRow, b: MergedSessionRow): number =>
  byStamp(a.lastMessageAt, b.lastMessageAt, a, b);

/**
 * Newest conversation first — the "active" stretch's order.
 *
 * Its own start time, not its activity. A desktop older than that field sends
 * none, and there the last-reply stamp is the closest thing to a stable order
 * available: a run's stamp moves, but a client with no other key must still
 * draw the rows somehow, and recency is what every other page uses.
 */
const byCreation = (a: MergedSessionRow, b: MergedSessionRow): number =>
  byStamp(
    a.session.createdAt ?? a.lastMessageAt,
    b.session.createdAt ?? b.lastMessageAt,
    a,
    b,
  );

/**
 * A row's fold key.
 *
 * The rule the other two pages use, so one conversation folds the same wherever
 * it is drawn: a conversation of the machine the phone is talking to folds by its
 * session id, and a peer's carries its machine — a session id is that machine's,
 * not global, and two machines can each hold a conversation called `s1`.
 */
function foldKey(row: MergedSessionRow, activeDesktopId: string | null): string {
  return row.desktopId === activeDesktopId ? row.session.sessionId : row.key;
}

/** What the page ranks, and what hangs under it. */
interface RecentHierarchy {
  /** Every row with no parent on this page, in the order the page received them. */
  roots: MergedSessionRow[];
  /** Each row's children, in the order the page received them. */
  children: Map<string, MergedSessionRow[]>;
}

/**
 * Split the rows into what the page ranks and what hangs under it.
 *
 * The page ranks conversations, not sessions: a conversation's children are its
 * work, drawn inside its fold. A row whose parent is not a row here — dropped by
 * a search, hidden by the device filter, archived, or gone — is a root, which is
 * the rule `buildSessionTree` already applies and the only one that never loses a
 * conversation. So is a row that names itself, and a malformed ring loses one
 * edge rather than every row in it.
 *
 * Pinning is deliberately not consulted: a pinned child is still a child. This
 * page has no pinned section to hoist it into — a pin here is a marker on the row
 * — so hoisting one would put a session at the top level that its parent is
 * supposed to explain.
 */
function recentHierarchy(
  rows: readonly MergedSessionRow[],
  activeDesktopId: string | null,
): RecentHierarchy {
  // A parent is named by a session id, which only means something on the machine
  // that issued it: two peers can hold the same id, so the parent is looked up
  // within its own machine.
  const bySession = new Map<string, MergedSessionRow>();
  for (const row of rows)
    bySession.set(`${row.desktopId}::${row.session.sessionId}`, row);
  const parents = new Map<string, string>();
  for (const row of rows) {
    const parentId = row.session.parentSessionId;
    if (!parentId || parentId === row.session.sessionId) continue;
    const parent = bySession.get(`${row.desktopId}::${parentId}`);
    if (parent && parent.key !== row.key) parents.set(row.key, parent.key);
  }
  // Break one edge per ring, including rings disconnected from every root.
  const resolved = new Set<string>();
  for (const key of parents.keys()) {
    const path = new Set<string>();
    let current: string | undefined = key;
    while (current && !resolved.has(current)) {
      path.add(current);
      const parent = parents.get(current);
      if (parent && path.has(parent)) {
        parents.delete(current);
        break;
      }
      current = parent;
    }
    for (const visited of path) resolved.add(visited);
  }
  const roots: MergedSessionRow[] = [];
  const children = new Map<string, MergedSessionRow[]>();
  for (const row of rows) {
    const parent = parents.get(row.key);
    if (!parent) {
      roots.push(row);
      continue;
    }
    const siblings = children.get(parent);
    if (siblings) siblings.push(row);
    else children.set(parent, [row]);
  }
  return { roots, children };
}

function sessionRow(
  row: MergedSessionRow,
  options: {
    activeDesktopId: string | null;
    depth: number;
    hasChildren: boolean;
    search: boolean;
    section: "active" | "earlier";
  },
): CatalogRow {
  return {
    kind: "session",
    key: row.key,
    session: row.session,
    // A search draws one flat list of matches, as the other two pages do: a
    // result that indented a row under a parent it did not show would say where
    // the row lives rather than what was found.
    depth: options.search ? 0 : Math.min(options.depth, MAX_DEPTH),
    hasChildren: !options.search && options.hasChildren,
    foldId: foldKey(row, options.activeDesktopId),
    section: options.section,
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
 *
 * `expanded` is the fold set the other pages use, so a conversation opened on any
 * of them is open here too; a child is drawn only inside its own conversation's
 * fold.
 */
export function recentRows(
  rows: readonly MergedSessionRow[],
  query: string,
  limit: number = RECENT_PAGE_SIZE,
  expanded: ReadonlySet<string> = new Set(),
  activeDesktopId: string | null = null,
): RecentPage {
  const search = query.trim().toLocaleLowerCase();
  const matches = (row: MergedSessionRow): boolean =>
    !search
    || row.session.title.toLocaleLowerCase().includes(search)
    || (row.session.workspaceName ?? "").toLocaleLowerCase().includes(search);
  // The hierarchy is built from the *listed* rows, so a child whose parent does
  // not match is a root of the result rather than a row with nothing above it.
  const listed = rows.filter(matches);
  const { roots, children } = recentHierarchy(listed, activeDesktopId);
  const active = roots.filter(isActiveSession).sort(byCreation);
  const earlier = roots.filter(row => !isActiveSession(row)).sort(byRecency);
  /**
   * One conversation and the children its fold reveals, parents before children.
   *
   * Only a conversation's own fold decides whether its children are drawn: a
   * child is therefore never a row of the stretch itself. A search draws the
   * flattened matches and ignores folds, as the other pages do.
   */
  const block = (
    row: MergedSessionRow,
    section: "active" | "earlier",
    depth = 0,
  ): CatalogRow[] => {
    const kids = children.get(row.key) ?? [];
    const head = sessionRow(row, {
      activeDesktopId,
      depth,
      hasChildren: kids.length > 0,
      search: search.length > 0,
      section,
    }) as Extract<CatalogRow, { kind: "session" }>;
    if (search || kids.length === 0 || !expanded.has(head.foldId)) return [head];
    return [head, ...kids.flatMap(kid => block(kid, section, depth + 1))];
  };
  if (search) {
    return {
      rows: [
        ...active.map(row => sessionRow(row, {
          activeDesktopId,
          depth: 0,
          hasChildren: false,
          search: true,
          section: "active",
        })),
        ...earlier.map(row => sessionRow(row, {
          activeDesktopId,
          depth: 0,
          hasChildren: false,
          search: true,
          section: "earlier",
        })),
      ],
      more: 0,
    };
  }
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
    out.push(...drawn.flatMap(row => block(row, kind)));
  };
  section("sessions.activeSection", "active", active, active);
  section("sessions.earlierSection", "earlier", earlier, shown);
  return { rows: out, more: earlier.length - shown.length };
}
