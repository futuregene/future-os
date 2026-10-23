/**
 * The activity rail's recent view: every conversation in one list, split into
 * what is still running and what is not.
 *
 * The rail's tree answers "where does this conversation live" — a workspace, or
 * the chats section — and the tree is the product's primary shape, so it stays
 * the default. This is the second view: it ignores the tree and pinning for its
 * *order*, on one question, "what did I last touch". A pinned conversation
 * keeps its marker but not its place.
 *
 * - A conversation that has children is **one row here**, not several. A fork,
 *   and a session an agent started to do one piece of work (a `/future-goal`
 *   worker), are not conversations the reader scans the recent list for — they
 *   are the work *of* a conversation they did scan it for. So the stretches rank
 *   only the top level, a conversation with no parent in this list, and a child
 *   is drawn under its parent, indented, when that parent's `+` is opened. The
 *   fold state is the rail's own (the one the tree folds with), so a parent
 *   opened here is opened there too. Search is the one exception where a row
 *   without its parent is drawn on its own: a match the reader asked for must not
 *   be hidden behind a parent that did not match.
 *
 * The two stretches are ranked by different clocks, on purpose:
 *
 * - **Active** — a run that has not finished: producing output, queued to
 *   start, or waiting on the reader's approval. Ordered by when the
 *   conversation *started*, because a streaming run rewrites its activity stamp
 *   about once a second, and ranking by that swaps rows under the reader's thumb
 *   while a handful of runs take turns producing output. A start time does not
 *   move, so a row holds its place for the whole run.
 * - **Earlier** — everything else, ordered by the last reply, which is the
 *   question that stretch answers.
 *
 * "Active" includes `waiting_approval` most of all: a run blocked on a question
 * is the one place a reader is being waited on, and it is *not* streaming, so a
 * list that read `streaming` alone would file it under "earlier".
 *
 * Kept free of React and of the rail's own types so the ordering, the split and
 * the window can be tested on their own — this is the part that must not drift
 * between the two lists (the rail's local one and the one that holds a paired
 * machine's rows too), and the part a screenshot's structural assertions read.
 */

/** Run states that mean "this conversation is not done yet". */
const ACTIVE_STATUSES = new Set(["running", "queued", "waiting_approval"]);

/**
 * How many finished conversations the view draws before it stops and offers the
 * rest.
 *
 * A reader with a few hundred conversations should not scroll past all of them
 * to reach the bottom of a view whose point is "the last few things". The window
 * is 30 here and 20 on the phone because the desktop list is taller.
 */
export const RECENT_PAGE_SIZE = 30;

/** One row, reduced to the facts the view orders, splits and nests by. */
export interface RecentItem {
  /** Unique within the list; breaks ties so a poll cannot reshuffle rows. */
  key: string;
  pinned: boolean;
  /** Whether this conversation's run is still in flight (see `ACTIVE_STATUSES`). */
  active: boolean;
  /** When the conversation was started, or null when the source has no such stamp. */
  createdAt: number | null;
  /** When it last produced a reply, or null when the source has no such stamp. */
  lastActivityAt: number | null;
  /**
   * The conversation this one hangs under, as a key in this list, or `null`.
   *
   * Resolved by the merge, not read from the row's own fields: a parent is only
   * addressable when it is a row *of this list*, and a parent the device filter
   * dropped leaves its child a root rather than a row that cannot be drawn.
   * Absent for a root, and for a source that reports no lineage at all.
   */
  parentKey?: string | null;
}

export type RecentSectionKind = "active" | "earlier";

export interface RecentSection<T> {
  kind: RecentSectionKind;
  /** Every conversation in this stretch, ranked — the caption's count. */
  all: T[];
  /** The part the view draws: all of "active", a window of "earlier". */
  rows: T[];
}

export interface RecentSplit<T> {
  /** The stretches to draw, in order; an empty stretch is left out. */
  sections: RecentSection<T>[];
  /** Finished conversations the window is holding back; 0 once it holds them all. */
  more: number;
  /** Every row drawn, in draw order — what the keyboard walk steps through. */
  rows: T[];
}

/**
 * Whether this run state counts as "still in flight".
 *
 * The same judgement the row's own indicator makes (`effectiveRunStatus` in the
 * rail): a local run status wins, and a session the agent reports as streaming
 * with no local run row — a prompt started by the phone, the TUI or another
 * machine — still reads as running.
 */
export function isActiveRun(status: string | undefined | null, streaming?: boolean): boolean {
  if (status === "running" || status === "queued")
    return true;
  if (streaming === true)
    return true;
  return status !== undefined && status !== null && ACTIVE_STATUSES.has(status);
}

/**
 * Newest first on one stamp.
 *
 * A row with no stamp sorts *below* every dated one: "unknown age" must not
 * masquerade as "just now", and a source that predates the field must not
 * displace a row whose age is known.
 */
function byStamp(
  left: number | null,
  right: number | null,
  a: RecentItem,
  b: RecentItem,
): number {
  const at = left ?? Number.NEGATIVE_INFINITY;
  const bt = right ?? Number.NEGATIVE_INFINITY;
  if (at !== bt)
    return bt - at;
  return a.key < b.key ? -1 : a.key > b.key ? 1 : 0;
}

/** The active stretch's order: newest conversation first. */
export function byCreation(a: RecentItem, b: RecentItem): number {
  // A source without a start time falls back to the activity stamp: a run's
  // stamp moves, but a list with no other key must still draw its rows, and
  // recency is what every other list here uses.
  return byStamp(a.createdAt ?? a.lastActivityAt, b.createdAt ?? b.lastActivityAt, a, b);
}

/** The "earlier" stretch's order: most recent reply first. */
export function byActivity(a: RecentItem, b: RecentItem): number {
  return byStamp(a.lastActivityAt, b.lastActivityAt, a, b);
}

/** The view's rows, split by which ones rank as rows of their own. */
export interface RecentTree<T> {
  /** What the stretches rank: every row whose parent is not in this list. */
  roots: T[];
  /** Each row's children, in the order they arrived (the list's own order). */
  children: Map<string, T[]>;
}

/**
 * Split the rows into what the view ranks and what hangs under it.
 *
 * The view ranks conversations, not sessions: a conversation's children are its
 * work, drawn inside its fold. A row whose parent is not a row here — dropped by
 * a search, hidden by the device filter, archived, or gone — is a root, which is
 * the tree's own rule and the only one that never loses a conversation. So is a
 * row that names itself, and a malformed ring loses one edge rather than every
 * row in it.
 *
 * Pinning is deliberately not consulted: a pinned child is still a child. The
 * recent view has no pinned section to hoist it into — a pin there is a marker on
 * the row — so hoisting one would put a session on the top level that its parent
 * is supposed to explain.
 */
export function recentTree<T extends RecentItem>(rows: readonly T[]): RecentTree<T> {
  const present = new Set(rows.map(row => row.key));
  const parents = new Map<string, string>();
  for (const row of rows) {
    const parent = row.parentKey;
    if (parent && parent !== row.key && present.has(parent))
      parents.set(row.key, parent);
  }
  // One edge per ring is dropped, so a malformed lineage costs one row its place
  // under a parent instead of costing every row in the ring its place at all.
  const resolved = new Set<string>();
  for (const key of present) {
    const path = new Set<string>();
    let id: string | undefined = key;
    while (id && !resolved.has(id)) {
      path.add(id);
      const parent = parents.get(id);
      if (parent && path.has(parent)) {
        parents.delete(id);
        break;
      }
      id = parent;
    }
    for (const visited of path) resolved.add(visited);
  }
  const roots: T[] = [];
  const children = new Map<string, T[]>();
  for (const row of rows) {
    const parent = parents.get(row.key);
    if (!parent) {
      roots.push(row);
      continue;
    }
    const siblings = children.get(parent);
    if (siblings)
      siblings.push(row);
    else children.set(parent, [row]);
  }
  return { roots, children };
}

/** Where one drawn row sits in the view. */
export interface RecentRowMeta {
  /** How far it is indented under its root; a root itself is 0. */
  depth: number;
  /** Whether opening it would reveal rows of its own. */
  hasChildren: boolean;
}

export interface RecentExpansion<T> {
  /** The rows to draw, each parent before the children its fold reveals. */
  rows: T[];
  /** Where each drawn row sits, by row key. */
  meta: Map<string, RecentRowMeta>;
  /** Which root each drawn row hangs under, by row key — the keyboard's blocks. */
  roots: Map<string, string>;
}

/**
 * The deepest indent the view draws.
 *
 * The same three levels the tree flattens to: as much indentation as a rail has
 * room for. A deeper chain keeps its rows and stops indenting rather than
 * eating the title's width.
 */
const MAX_DEPTH = 2;

/**
 * A stretch's rows, with the children each opened row reveals.
 *
 * `rows` arrives ranked (a stretch's roots, in the order that stretch draws).
 * An opened row is followed immediately by its children, one level in, and by
 * their children if those are opened too. A child is therefore only ever drawn
 * inside an ancestor's expansion — never as a row of the stretch's own.
 */
export function expandRecent<T extends RecentItem>(
  rows: readonly T[],
  tree: Pick<RecentTree<T>, "children">,
  expanded: ReadonlySet<string>,
): RecentExpansion<T> {
  const out: RecentExpansion<T> = { rows: [], meta: new Map(), roots: new Map() };
  function visit(row: T, depth: number, root: string, ancestors: ReadonlySet<string>) {
    const kids = tree.children.get(row.key) ?? [];
    out.rows.push(row);
    out.meta.set(row.key, { depth: Math.min(depth, MAX_DEPTH), hasChildren: kids.length > 0 });
    out.roots.set(row.key, root);
    if (kids.length === 0 || !expanded.has(row.key) || ancestors.has(row.key))
      return;
    const next = new Set(ancestors);
    next.add(row.key);
    for (const kid of kids) visit(kid, depth + 1, root, next);
  }
  for (const row of rows) visit(row, 0, row.key, new Set());
  return out;
}

/**
 * Split the list into its two stretches.
 *
 * `rows` must already be filtered by whatever the reader asked for (a device, a
 * search) and reduced to the conversations that rank on their own (`recentTree`'s
 * roots — a child is ranked by the conversation it belongs to, not by itself):
 * this function decides order and how much of it is drawn, nothing about what is
 * in the list.
 */
export function recentSplit<T extends RecentItem>(
  rows: readonly T[],
  limit: number = RECENT_PAGE_SIZE,
): RecentSplit<T> {
  const active = rows.filter(row => row.active).sort(byCreation);
  const earlier = rows.filter(row => !row.active).sort(byActivity);
  const window = limit < 0 ? earlier : earlier.slice(0, Math.max(0, limit));
  const sections: RecentSection<T>[] = [];
  // An empty stretch is not a section: a caption over nothing reads as a list
  // that failed to load. A window that has not opened yet is not one either —
  // the caption counts the whole stretch, so a caption over no rows would claim
  // conversations it does not draw.
  if (active.length > 0)
    sections.push({ kind: "active", all: active, rows: active });
  if (window.length > 0)
    sections.push({ kind: "earlier", all: earlier, rows: window });
  return {
    sections,
    more: earlier.length - window.length,
    rows: sections.flatMap(section => section.rows),
  };
}
