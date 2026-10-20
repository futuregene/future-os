import type { PageDirection } from "./useTabSwipe";

/**
 * The list's pages, in the order the toolbar's control shows them and the
 * horizontal swipe reads them: workspaces are the left page, conversations the
 * right one.
 *
 * The screen and the list both need this — the screen decides which page a
 * conversation is created for, the list draws the pages and reads the swipe —
 * so the order lives in one place rather than in a copy of it each.
 */
export const TABS = ["workspace", "chat"] as const;

export type Tab = (typeof TABS)[number];

/** The page beside `tab` in a swipe direction, or null at either end of the row. */
export function adjacentTab(tab: Tab, direction: PageDirection): Tab | null {
  const index = TABS.indexOf(tab) + (direction === "next" ? 1 : -1);
  return TABS[index] ?? null;
}
