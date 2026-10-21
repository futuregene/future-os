import { useCallback, useState } from "react";

/**
 * Which list the activity rail shows.
 *
 * Three lists over the same conversations, and the tab *is* the choice — the
 * rail no longer has a separate "section" for workspaces and chats, because that
 * was a second control answering the same question:
 *
 * - **workspace** — workspaces, each with the conversations inside it. What the
 *   product is for; the default.
 * - **chat** — the conversations that belong to no workspace.
 * - **recent** — every conversation by one clock, split by whether it is still
 *   running.
 *
 * Switching tabs changes only how the rail draws the list it already holds:
 * nothing about a conversation's own fields, its workspace, or its place in the
 * other two lists.
 */
export type RailView = "workspace" | "chat" | "recent";

/** The view the rail opens on: the workspace tree, as it always has. */
export const DEFAULT_RAIL_VIEW: RailView = "workspace";

/** In the order the switch draws them. */
export const RAIL_VIEWS: readonly RailView[] = ["workspace", "chat", "recent"];

const STORAGE_KEY = "future.activityRail.view";

function isRailView(value: unknown): value is RailView {
  return typeof value === "string" && (RAIL_VIEWS as readonly string[]).includes(value);
}

function readRailView(): RailView {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    // Anything unknown (an older build's value, a corrupt write) falls back to
    // the default rather than to an empty rail. The two-value era wrote
    // `workspace` and `recent`, both of which still mean what they meant, so a
    // stored value needs no migration.
    return isRailView(raw) ? raw : DEFAULT_RAIL_VIEW;
  }
  catch {
    // Unavailable storage — the default, as if this were a first launch.
    return DEFAULT_RAIL_VIEW;
  }
}

/**
 * The rail's view, with the two ways it can change.
 *
 * They are separate on purpose, and the difference is what the preference means:
 *
 * - `select` is the user asking for a list — clicking a tab, or pressing ⌘1/2/3.
 *   That is the choice to remember, so it is written.
 * - `follow` is the shell taking the reader somewhere (they opened a
 *   conversation that lives in the other tab, a notification or a deep link
 *   pointed at one). Remembering that would turn the preference into "the last
 *   place I was dragged to", and the next launch would open there instead of
 *   where the user last chose to be.
 *
 * The write is synchronous with the state change rather than an effect, so a
 * window closed immediately after a click still remembers it.
 */
export function useRailView() {
  const [view, setView] = useState<RailView>(readRailView);

  const select = useCallback((next: RailView) => {
    setView(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    }
    catch { /* Storage is best effort. */ }
  }, []);

  const follow = useCallback((next: RailView) => {
    setView(next);
  }, []);

  return { follow, select, view };
}

/** Which tab a conversation belongs to, from the only field that decides it. */
export function viewForMode(mode: "chat" | "workspace" | null | undefined): RailView {
  return mode === "workspace" ? "workspace" : "chat";
}
