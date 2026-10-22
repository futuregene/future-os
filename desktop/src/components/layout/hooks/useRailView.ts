import { useCallback, useEffect, useState } from "react";

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

/**
 * The left-to-right order of the switch's two segments.
 *
 * A tuple rather than an array so the two ids cannot go missing: the switch
 * always offers both views, and the reader only ever chooses which one is on
 * the left.
 */
export type RailViewOrder = readonly [RailView, RailView];

/** The order the switch has always drawn, and the one a first launch gets. */
export const DEFAULT_RAIL_ORDER: RailViewOrder = ["workspace", "recent"];

const STORAGE_KEY = "future.activityRail.view";
const ORDER_STORAGE_KEY = "future.activityRail.viewOrder";

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

function isRailView(value: unknown): value is RailView {
  return value === "workspace" || value === "recent";
}

/**
 * A stored order, or the default.
 *
 * Only a permutation of both views is accepted: a stored value missing one, or
 * naming one twice, would drop a segment from the switch and leave the reader
 * with a list they cannot get back to.
 */
function readRailOrder(): RailViewOrder {
  try {
    const raw = localStorage.getItem(ORDER_STORAGE_KEY);
    if (!raw)
      return DEFAULT_RAIL_ORDER;
    const parsed: unknown = JSON.parse(raw);
    if (
      Array.isArray(parsed)
      && parsed.length === 2
      && isRailView(parsed[0])
      && isRailView(parsed[1])
      && parsed[0] !== parsed[1]
    ) {
      return [parsed[0], parsed[1]];
    }
    return DEFAULT_RAIL_ORDER;
  }
  catch {
    // Unavailable storage or a corrupt write — as if this were a first launch.
    return DEFAULT_RAIL_ORDER;
  }
}

/**
 * The rail's view and its switch order, both persisted so a restart brings back
 * the list the user left.
 *
 * Their own keys rather than a field on any conversation: they are properties of
 * how this window lists things, and two windows on one machine may disagree.
 */
export function useRailView() {
  const [view, setView] = useState<RailView>(readRailView);
  const [order, setOrder] = useState<RailViewOrder>(readRailOrder);

  useEffect(() => {
    try {
      localStorage.setItem(STORAGE_KEY, view);
    }
    catch { /* Storage is best effort. */ }
  }, [view]);

  useEffect(() => {
    try {
      localStorage.setItem(ORDER_STORAGE_KEY, JSON.stringify(order));
    }
    catch { /* Storage is best effort. */ }
  }, [order]);

  /**
   * Put `from` where `to` is, moving `to` into the vacated slot.
   *
   * A swap rather than an insert: there are exactly two segments, so the only
   * sensible drag is "these two trade places", and an insert would be the same
   * move with an index to recompute.
   */
  const moveView = useCallback((from: RailView, to: RailView) => {
    setOrder((current) => {
      if (from === to || !current.includes(from) || !current.includes(to))
        return current;
      const [left, right] = current;
      return [right, left];
    });
  }, []);

  return { moveView, order, setView, view };
}
