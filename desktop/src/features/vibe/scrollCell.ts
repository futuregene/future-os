/**
 * Scrolling one panel cell's transcript, from the keyboard.
 *
 * The grid shows several conversations at once, and the ring moving between cells is not
 * the same act as reading one of them: a cell is small, so its text has to move while the
 * ring stays put. That is what these commands do — and what makes them possible is one
 * attribute the transcript already carries (`data-chat-scroll`), addressed **inside the
 * cell** rather than through a new prop on the shared conversation view.
 *
 * Pure where it can be: the distance and the element lookup. The DOM part is one function
 * so the "which element" question has one answer, and a test can stage it.
 */

/** How much of the viewport one press moves: most of it, keeping a line of context. */
export const SCROLL_STEP_RATIO = 0.85;

/**
 * The scrollable transcript of the cell drawn for `key`, or `null`.
 *
 * `root` is the panel element: the lookup is scoped to it so a transcript somewhere else
 * in the app (the main window behind the panel is unmounted, but a future one might not
 * be) can never be the one that moves.
 */
export function transcriptOf(root: ParentNode | null, key: string | null): HTMLElement | null {
  if (!root || !key)
    return null;
  const cell = root.querySelector<HTMLElement>(`[data-vibe-transcript-for="${cssEscape(key)}"]`);
  return cell?.querySelector<HTMLElement>("[data-chat-scroll]") ?? null;
}

/**
 * `CSS.escape` is missing in jsdom and in some older engines, and a cell key contains `:`
 * (`local:<id>`), which is a selector separator — so the escaping is explicit rather than
 * borrowed.
 */
function cssEscape(value: string): string {
  return value.replace(/["\\]/g, "\\$&");
}

/** Move one transcript by a page, keeping the reader's place if the direction is up. */
export function scrollTranscript(element: HTMLElement | null, direction: -1 | 1): boolean {
  if (!element)
    return false;
  const viewport = element.clientHeight || element.offsetHeight;
  const delta = Math.round(viewport * SCROLL_STEP_RATIO) * direction;
  if (typeof element.scrollBy === "function") {
    element.scrollBy({ behavior: "smooth", top: delta });
    return true;
  }
  // An engine without `scrollBy` still scrolls through the property.
  element.scrollTop += delta;
  return true;
}

/** Put a transcript at its newest message — the panel's "jump to the live end". */
export function scrollToLiveEnd(element: HTMLElement | null): boolean {
  if (!element)
    return false;
  if (typeof element.scrollTo === "function") {
    element.scrollTo({ behavior: "smooth", top: element.scrollHeight });
    return true;
  }
  element.scrollTop = element.scrollHeight;
  return true;
}
