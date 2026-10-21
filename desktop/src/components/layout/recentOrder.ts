/**
 * The activity rail's recent view: every conversation in one list, split into
 * what is still running and what is not.
 *
 * The rail's tree answers "where does this conversation live" — a workspace, or
 * the chats section — and the tree is the product's primary shape, so it stays
 * the default. This is the second view: it ignores the tree, forks and pinning
 * for its *order*, on one question, "what did I last touch". A pinned
 * conversation keeps its marker but not its place; a fork is a row of its own at
 * its own time (there is no parent above it to indent under).
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

/** One row, reduced to the facts the view orders and splits by. */
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

/**
 * Split the list into its two stretches.
 *
 * `rows` must already be filtered by whatever the reader asked for (a device, a
 * search): this function decides order and how much of it is drawn, nothing
 * about what is in the list.
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
