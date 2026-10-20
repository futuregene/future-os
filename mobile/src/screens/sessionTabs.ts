import type { PageDirection } from "./useTabSwipe";

/**
 * The list's pages, in the order the toolbar's control shows them and the
 * horizontal swipe reads them: workspaces are the left page, conversations the
 * middle one, and everything by recency the right one.
 *
 * The screen and the list both need this — the screen decides which page a
 * conversation is created for, the list draws the pages and reads the swipe —
 * so the order lives in one place rather than in a copy of it each.
 */
export const TABS = ["workspace", "chat", "recent"] as const;

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
 * The page a swipe opens: the neighbour, except that dragging right from the
 * leftmost page leaves the row of pages and lands on the rightmost one.
 *
 * Swiping right from the workspaces page had no page behind it — the row simply
 * ended — and the recency page is the one a reader reaches for from there ("what
 * did I last touch"), so the row closes into a loop. Dragging the other way off
 * the rightmost page stays inert: it is a gesture with nothing behind it, not a
 * trip back to the other end.
 */
export function swipeTarget(tab: Tab, direction: PageDirection): Tab | null {
  const neighbour = adjacentTab(tab, direction);
  if (neighbour) return neighbour;
  return direction === "prev" ? TABS[TABS.length - 1] ?? null : null;
}
