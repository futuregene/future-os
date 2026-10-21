import type { PageDirection } from "./useTabSwipe";

/**
 * The list's pages, in the order the toolbar's control shows them and the
 * horizontal swipe reads them: the recency page is the left page, workspaces the
 * middle one, and conversations the right one.
 *
 * Recency leads because it is the page the app opens on, and a row of pages whose
 * default sits in the middle starts the reader in the middle of the row: the
 * leftmost page is where they expect the edge to be. It is also the page that
 * answers "what was I doing" — the first question a phone is opened for — while
 * the two tree pages need the reader to know which workspace a conversation lives
 * in first.
 *
 * The screen and the list both need this — the screen decides which page a
 * conversation is created for, the list draws the pages and reads the swipe —
 * so the order lives in one place rather than in a copy of it each.
 */
export const TABS = ["recent", "workspace", "chat"] as const;

export type Tab = (typeof TABS)[number];

/** What each page calls itself in the switch above the list. */
export const TAB_LABEL: Record<Tab, string> = {
  workspace: "sessions.workspace",
  chat: "sessions.conversations",
  recent: "sessions.recent",
};

/** The page beside `tab` in a swipe direction, or null at either end of the row. */
export function adjacentTab(tab: Tab, direction: PageDirection): Tab | null {
  const index = TABS.indexOf(tab) + (direction === "next" ? 1 : -1);
  return TABS[index] ?? null;
}

/**
 * The page a swipe opens. The row of pages closes into a loop.
 *
 * Both ends wrap: dragging left from the conversations page lands on the recency
 * page, dragging right from recency lands on the conversations page. Each of
 * those gestures had nothing behind it — the row simply ended — and each is the
 * shortest way to the page a reader wants from there: the list of last things
 * from the conversations they were reading, and the conversations from that list.
 *
 * A swipe always moves one page when it moves at all, so the loop never skips the
 * middle: reaching workspaces from conversations is one drag right, and reaching
 * workspaces from recency is one drag left.
 */
export function swipeTarget(tab: Tab, direction: PageDirection): Tab | null {
  const neighbour = adjacentTab(tab, direction);
  if (neighbour) return neighbour;
  return direction === "prev"
    ? TABS[TABS.length - 1] ?? null
    : TABS[0] ?? null;
}
