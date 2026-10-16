/**
 * When a prompt is longer than its bubble can usefully show, and how much of it
 * a collapsed bubble keeps.
 *
 * A prompt is the user's own words, so the bubble has no author-supplied
 * structure to fold the way a code block or a table does — the only thing that
 * can be bounded is height. Two consequences shaped this module:
 *
 * - The decision is made from the **source**, before anything is laid out.
 *   Measuring a rendered bubble would mean painting the whole giant row first
 *   (the very layout this exists to avoid) and then re-laying it out shorter.
 * - The numbers are the ones the code block already uses (more than 16 lines, or
 *   more than 10 000 characters, is long; a collapsed block keeps 16 wrapped
 *   lines), so a reader meets the same budget in both places.
 */
import { spacing } from "../../../theme/tokens";

/** Wrapped chat lines a collapsed prompt shows, and the count beyond which a
 * prompt is collapsed at all — the code block's own budget and threshold. */
export const PROMPT_COLLAPSE_LINES = 16;

/** Source characters beyond which a prompt is long even on a single line: one
 * 4 000-character paragraph wraps to well over 16 lines on a phone, and the
 * line count alone cannot see that. The code block's second clause. */
const PROMPT_COLLAPSE_CHARS = 10_000;

/** The bubble's own vertical padding, which a height cap has to include for the
 * capped box to still show the promised lines of text. */
const PROMPT_BUBBLE_PADDING = spacing.md * 2;

/**
 * A collapsed prompt never owns more than this share of the window, whatever
 * the reader's text size: 16 lines is a comfortable slice of a tall phone but
 * most of a short one at the largest size, and a row that fills the viewport is
 * exactly the shape this module exists to prevent.
 */
const COLLAPSED_WINDOW_SHARE = 0.5;

/**
 * Whether this prompt should start collapsed.
 *
 * The bail-out length in `split` keeps a 7 KB prompt from materialising an
 * array of every line: all this needs to know is whether the count exceeds 16,
 * and the code block reads its source the same way.
 */
export function isLongPrompt(text: string): boolean {
  return (
    text.length > PROMPT_COLLAPSE_CHARS ||
    text.split("\n", PROMPT_COLLAPSE_LINES + 2).length > PROMPT_COLLAPSE_LINES
  );
}

/**
 * The height cap a collapsed prompt bubble takes, in dp.
 *
 * `lineHeight` is the reader's own scaled chat line height, so the cap grows
 * with the text size the way every other measurement here does; the window
 * share is what keeps it bounded on a short screen with the largest type.
 */
export function collapsedPromptHeight(lineHeight: number, windowHeight: number): number {
  return Math.min(
    lineHeight * PROMPT_COLLAPSE_LINES + PROMPT_BUBBLE_PADDING,
    Math.round(windowHeight * COLLAPSED_WINDOW_SHARE),
  );
}
