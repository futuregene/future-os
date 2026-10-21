/**
 * The reading layer's second column: whether it is there, and what that answer is per
 * conversation.
 *
 * Three states, and the third is the one that makes it usable from a sofa:
 *
 * - **hidden** — no column, one full-width transcript;
 * - **auto** — the column appears when the window is wide enough to hold both at a
 *   readable measure, and folds away when it is not. The default, because a reader who
 *   never thinks about it gets the right answer for the window they are in;
 * - **pinned** — always there, whatever the window does. That is a decision about *this
 *   conversation* ("I am reading this one's files"), so it is remembered per
 *   conversation, by the panel's own stable key.
 *
 * **Hiding, though, is a habit, not a fact about one conversation.** Closing the column in
 * one cell and having it open again in the next made the reader close it again and again,
 * so the `hidden`/`auto` half is *sticky*: the last of those two choices becomes the
 * fallback for any conversation that has no entry of its own (`stickyDefault`). `pinned`
 * is deliberately **not** sticky — pinning is a decision about one conversation, and
 * inheriting it would open a column nobody asked for.
 *
 * Kept pure and separate: the rule is a table of cases (preference × width), and the
 * question "is the column there" is asked from the render, the key and the menu.
 */

/** What the reader chose for one conversation's second column. */
export type ReadingPanelPreference = "auto" | "hidden" | "pinned";

/**
 * The width below which `auto` folds the column away.
 *
 * 1100px is the point where two readable columns stop fitting: the transcript's own
 * measure is about 640–700 and the panel's about 400, so anything narrower gives the
 * transcript a width its message layout was not designed for. `pinned` ignores it — that
 * is what pinning is for.
 */
export const PANEL_AUTO_WIDTH = 1100;

/** The column's width when it is shown. Fixed: the panel is not a resizable window. */
export const PANEL_WIDTH = 400;

/** Whether the reading layer draws its second column, given a preference and a width. */
export function showReadingPanel(preference: ReadingPanelPreference, windowWidth: number): boolean {
  switch (preference) {
    case "hidden":
      return false;
    case "pinned":
      return true;
    case "auto":
      return windowWidth >= PANEL_AUTO_WIDTH;
  }
}

/**
 * What `⌘\` does, given where the conversation is now.
 *
 * A **toggle between "hidden" and "auto"**, not a cycle through three: the key answers
 * "give me the wide transcript back" and "bring it back the way it was", and cycling
 * through `pinned` would make one press sometimes *show* a column the reader was trying
 * to hide. Pinning has its own key.
 */
export function toggleHidden(preference: ReadingPanelPreference): ReadingPanelPreference {
  return preference === "hidden" ? "auto" : "hidden";
}

/** What `⌘⇧\` does: pin, or release the pin back to `auto`. */
export function togglePinned(preference: ReadingPanelPreference): ReadingPanelPreference {
  return preference === "pinned" ? "auto" : "pinned";
}

/** The next preference in `delta`'s direction, for the menu's value row. */
export function cyclePreference(preference: ReadingPanelPreference, delta: -1 | 1): ReadingPanelPreference {
  const order: ReadingPanelPreference[] = ["hidden", "auto", "pinned"];
  const at = order.indexOf(preference);
  const next = order[((at === -1 ? 1 : at) + delta + order.length) % order.length]!;
  return next;
}

/** The i18n leaf for a preference (`vibe:panel.hidden` …). */
export function preferenceLabelKey(preference: ReadingPanelPreference): string {
  return `panel.${preference}`;
}

/**
 * Read one conversation's preference out of the panel's stored map.
 *
 * `fallback` is the panel's sticky `hidden`/`auto` choice, used for a conversation nobody
 * has decided about yet: that is what makes closing the column in one cell keep it closed
 * in the next one. A value this build does not know is treated as "no entry" rather than
 * guessed at.
 */
export function readPanelPreference(
  all: Readonly<Record<string, unknown>>,
  key: string,
  fallback: "auto" | "hidden" = "auto",
): ReadingPanelPreference {
  const value = all[key];
  return value === "hidden" || value === "pinned" || value === "auto" ? value : fallback;
}

/**
 * The sticky default a choice implies: `hidden`/`auto` set it, `pinned` leaves it alone.
 *
 * One function rather than a condition at each write site, because the rule is "pinning
 * says nothing about the other conversations" and that is easy to get wrong twice.
 */
export function nextStickyDefault(current: "auto" | "hidden", preference: ReadingPanelPreference): "auto" | "hidden" {
  return preference === "pinned" ? current : preference;
}
