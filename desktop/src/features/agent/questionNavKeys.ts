/**
 * The keyboard half of the transcript's ↑/↓ pair: which key press means which
 * direction (see `useQuestionNav`, which consumes this).
 *
 * The pair is also two buttons in the conversation's corner, and reading with
 * the arrow keys is the same walk — so a press has to be judged as carefully as
 * a click, and mostly by what it must *not* take over. An unmodified arrow key
 * is the most-shared key in a desktop app: it moves a caret, picks an IME
 * candidate, walks a menu, scrolls a list. The transcript only answers a press
 * when nobody else is using it.
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
   * editor, a search field, a dialog input (see `lib/editableTarget`). There the
   * arrow key belongs to the caret, not to the transcript.
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
 * A modifier means the press is something else: Shift+Arrow extends a selection
 * or moves inside text, Option/Alt+Arrow is a word-wise motion, Cmd/Ctrl+Arrow is
 * the platform's own line motion. A composing press belongs to the IME, where
 * the arrows pick candidates.
 *
 * Reachability is deliberately *not* decided here: whether there is a stop in
 * that direction is a fact about the current layout, which only `useQuestionNav`
 * can read. It resolves the direction against the live DOM before swallowing the
 * key, so a press that would move nothing stays whatever it already was — a line
 * scroll of a short transcript.
 */
export function navDirectionForKey(event: NavKeyEvent, context: NavKeyContext): NavDirection | null {
  if (event.key !== "ArrowUp" && event.key !== "ArrowDown")
    return null;
  if (event.altKey || event.ctrlKey || event.metaKey || event.shiftKey)
    return null;
  if (event.isComposing)
    return null;
  if (context.editable || context.owned)
    return null;
  return event.key === "ArrowUp" ? "previous" : "next";
}
