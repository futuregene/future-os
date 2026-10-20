/**
 * Whether a key event's target is somewhere the user is typing.
 *
 * A global handler for an unmodified key (the transcript's PageUp/PageDown pair,
 * the approval prompt's Enter) must leave a key alone when the caret is in a text
 * field: inside the composer's editor the key belongs to the field, not to the
 * transcript.
 *
 * The ancestor walk is what covers `contentEditable`: a keystroke's target is
 * usually a text node's element, and the editor is a div with pills inside it.
 * `isContentEditable` is checked as well because some engines report only the
 * property, and the attribute walk is what also answers in jsdom, which
 * implements the property as `undefined`.
 */

/**
 * Marks an editor whose caret has **nothing to move through** right now, because
 * it holds no text at all.
 *
 * The composer autofocuses on mount and after every send, so at the moment a
 * reader takes over a finished reply the caret is sitting in an empty editor —
 * and "a page key belongs to the field" would leave them with no keyboard way
 * to page the transcript at all. An empty editor is not a place anyone is typing
 * in*, so the key is free for the surrounding view; the moment one character or
 * mention lands in it the marker is gone and the key is the field's again.
 *
 * `MentionEditor` (and the remote composer's textarea) set it themselves, since
 * only they know whether they are empty.
 */
export const EMPTY_EDITOR_ATTR = "data-empty-editor";

/**
 * The editor a key event is aimed at, or null when the target is not one.
 *
 * `<select>` counts: a keystroke in one belongs to the control (type-ahead, or
 * the list it opens), and a global handler must not take it.
 */
function editingElement(target: HTMLElement): HTMLElement | null {
  const tagName = target.tagName.toLowerCase();
  if (tagName === "input" || tagName === "textarea" || tagName === "select")
    return target;
  if (target.isContentEditable)
    return target;
  return target.closest<HTMLElement>("[contenteditable]:not([contenteditable='false'])");
}

export function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement))
    return false;
  const editor = editingElement(target);
  return editor !== null && !editor.hasAttribute(EMPTY_EDITOR_ATTR);
}
