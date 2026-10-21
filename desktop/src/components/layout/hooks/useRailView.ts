import { useEffect, useState } from "react";

/**
 * Which list the activity rail shows.
 *
 * Two views over the same conversations, not two lists: the **tree** (workspaces
 * and chats, which is the product's primary shape and stays the default) and
 * **recent** (every conversation by one clock, split by whether it is still
 * running). Switching views changes only how the rail draws the list it already
 * holds — nothing about a conversation's own fields or order in the tree.
 */
export type RailView = "workspace" | "recent";

/** The view the rail opens on: the tree, as it always has. */
export const DEFAULT_RAIL_VIEW: RailView = "workspace";

const STORAGE_KEY = "future.activityRail.view";

function readRailView(): RailView {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    // Anything unknown (an older build's value, a corrupt write) falls back to
    // the default rather than to an empty rail.
    return raw === "recent" ? "recent" : DEFAULT_RAIL_VIEW;
  }
  catch {
    // Unavailable storage — the default, as if this were a first launch.
    return DEFAULT_RAIL_VIEW;
  }
}

/**
 * The rail's view, persisted so a restart brings back the list the user left.
 *
 * Its own key rather than a field on any conversation: it is a property of how
 * this window lists things, and two windows on one machine may disagree.
 */
export function useRailView() {
  const [view, setView] = useState<RailView>(readRailView);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, view);
    }
    catch { /* Storage is best effort. */ }
  }, [view]);

  return { setView, view };
}
