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
 * The page a swipe opens. The row of pages closes into a loop.
 *
 * Both ends wrap: dragging left from the recency page lands on the workspaces
 * page, dragging right from workspaces lands on the recency page. Each of those
 * gestures had nothing behind it — the row simply ended — and each is the
 * shortest way to the page a reader wants from there: the last things they
 * touched from the workspace list, and the workspace list from that list of last
 * things.
 *
 * A swipe always moves one page when it moves at all, so the loop never skips
 * the middle: reaching conversations from recency is still one drag right, and
 * reaching workspaces from conversations is still one drag right.
 */
export function swipeTarget(tab: Tab, direction: PageDirection): Tab | null {
  const neighbour = adjacentTab(tab, direction);
  if (neighbour) return neighbour;
  return direction === "prev"
    ? TABS[TABS.length - 1] ?? null
    : TABS[0] ?? null;
}
