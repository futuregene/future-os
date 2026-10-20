/**
 * The keyboard half of the transcript's ↑/↓ pair: which key press means which
 * direction (see `useQuestionNav`, which consumes this).
 *
 * The pair is also two buttons in the conversation's corner, and **Page Up /
 * Page Down** are the keys that do the same walk — the keys whose own stated job
 * is already "move by a screenful", which is what a landing is. The bare arrows
 * are deliberately *not* the walk: on a scroll container ↑/↓ is the reader's
 * ordinary line scroll, the finer control they reach for first on a long reply,
 * and taking it over to jump a whole turn per press is the app answering a key
 * that was aimed at the page. What still has to be judged is what a page key
 * must *not* take over, because a stray hijack here costs the reader a keystroke
 * every time.
 */

/** The part of a key event this policy reads; tests need no real event. */
export interface NavKeyEvent {
  key: string;
  altKey: boolean;
  ctrlKey: boolean;
  metaKey: boolean;
  shiftKey: boolean;
  /** True while the event belongs to an input method composition. */
  isComposing: boolean;
}

export interface NavKeyContext {
  /**
   * Whether the event's target is somewhere the user is typing — the composer's
   * editor, a search field, a dialog input (see `lib/editableTarget`). A page
   * key there belongs to the field's own scrolling, not to the transcript.
   */
  editable: boolean;
  /** Whether a modal or a floating menu holds the keyboard (see `lib/keyboardOwners`). */
  owned: boolean;
}

export type NavDirection = "previous" | "next";

/**
 * The direction a key press asks for, or null when the key is not the
 * transcript's to answer.
 *
 * A modifier means the press is something else: Ctrl+Page Up/Page Down switches
 * tabs (in the app, in a terminal-emulating view, in the browser around it), and
 * Shift+Page Up extends a selection — none of them a request to page the
 * transcript. A composing press belongs to the IME.
 *
 * Reachability is deliberately *not* decided here: whether there is a stop in
 * that direction is a fact about the current layout, which only `useQuestionNav`
 * can read. It resolves the direction against the live DOM before swallowing the
 * key, so a press that would move nothing stays whatever it already was — the
 * page's own screenful scroll of a short transcript.
 */
export function navDirectionForKey(event: NavKeyEvent, context: NavKeyContext): NavDirection | null {
  if (event.key !== "PageUp" && event.key !== "PageDown")
    return null;
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey)
    return null;
  if (event.isComposing)
    return null;
  if (context.editable || context.owned)
    return null;
  return event.key === "PageUp" ? "previous" : "next";
}
