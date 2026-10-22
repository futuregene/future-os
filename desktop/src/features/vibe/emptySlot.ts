import type { StoredThread } from "../../integrations/storage/threadStore";
import type { RemoteCatalog } from "../remote-peer/mergeConversations";

/**
 * The grid's empty slot, and what fills it.
 *
 * Two things live here because both are "what the grid does when it has room":
 *
 * - **the add tile** — a free slot at the end of the grid that opens the panel's
 *   conversation picker. It is a *slot*, not a dialog button, because the empty square in
 *   the bottom-right corner of a 3×2 grid is the one place a reader already looks for
 *   "something missing here";
 * - **the page** — what happens past twelve pinned conversations (`PAGE_SIZE`). The panel
 *   refused to scroll on purpose (`VIBE.md` §4: a grid you have to scroll is not an
 *   overview), and letting the cells shrink without limit is worse: at 13 cells a 4×4 of
 *   320px cells is unreadable. So the grid pages, twelve at a time, and the reader says
 *   which page.
 *
 * Pure: a page boundary and a "recently active" sort are both worth pinning down without a
 * DOM.
 */

/** The key that stands for the add tile in the focus ring. Not a cell key. */
export const ADD_SLOT_KEY = "__add__";

/** How many conversations one page holds. */
export const PAGE_SIZE = 12;

/** How many of the reader's recent conversations the picker offers before "browse all". */
export const RECENT_LIMIT = 5;

/** The number of pages needed for `total` cells: always at least one (an empty page). */
export function pageCount(total: number, size = PAGE_SIZE): number {
  return Math.max(1, Math.ceil(total / size));
}

/** `page` clamped into `[0, pageCount - 1]`: an unpin must never leave the reader nowhere. */
export function clampPage(page: number, total: number, size = PAGE_SIZE): number {
  return Math.min(Math.max(0, page), pageCount(total, size) - 1);
}

/** The cells on one page, in order. */
export function pageSlice<T>(cells: readonly T[], page: number, size = PAGE_SIZE): T[] {
  const from = clampPage(page, cells.length, size) * size;
  return cells.slice(from, from + size);
}

/** Whether this page has a free slot for the add tile. */
export function hasAddSlot(cellsOnPage: number, size = PAGE_SIZE): boolean {
  return cellsOnPage < size;
}

/**
 * One row of the picker: a conversation the reader has been in recently, not yet pinned.
 *
 * The list is ordered by **when it was last touched**, which is the only ordering that
 * answers "I was just in this one" — the panel's own order is creation time (§2) and is
 * deliberately the opposite.
 */
export interface RecentChoice {
  key: string;
  title: string;
  /** The machine's name, for a host's conversation; `null` for this machine's. */
  machine: string | null;
}

export function recentChoices(
  threads: readonly StoredThread[],
  catalogs: readonly RemoteCatalog[],
  inPanel: ReadonlySet<string>,
  peerLabels: ReadonlyMap<string, string>,
  limit = RECENT_LIMIT,
): RecentChoice[] {
  const local = threads
    .filter(thread => thread.status === "active" && !thread.pinned)
    .map(thread => ({
      at: thread.lastMessageAt ?? thread.lastOpenedAt ?? thread.updatedAt,
      choice: { key: `local:${thread.id}`, machine: null, title: thread.title } as RecentChoice,
    }));
  const remote = catalogs.flatMap(catalog => catalog.sessions
    // A host's `status` is its last run's, not the thread's lifecycle (see
    // `vibeCells.ts`), so it says nothing about whether the conversation can be
    // added. Filtering on it hid every host conversation that had ever run.
    .filter(session => session.pinned !== true)
    .map(session => ({
      at: session.lastMessageAt ?? 0,
      choice: {
        key: `remote:${catalog.desktopId}:${session.sessionId}`,
        machine: peerLabels.get(catalog.desktopId) ?? catalog.desktopId,
        title: session.title,
      } as RecentChoice,
    })));
  return [...local, ...remote]
    .filter(entry => !inPanel.has(entry.choice.key))
    .sort((left, right) => right.at - left.at)
    .slice(0, limit)
    .map(entry => entry.choice);
}
