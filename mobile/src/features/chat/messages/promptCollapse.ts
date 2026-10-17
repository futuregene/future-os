/**
 * How a long prompt is shown in the transcript, and what "show me the rest"
 * opens.
 *
 * A prompt is the user's own words, so the bubble has no author-supplied
 * structure to fold the way a code block or a table does — the only thing that
 * can be bounded is how much of it the transcript draws.
 *
 * Two measurements shaped this module. First, a prompt is often multi-kilobyte
 * goal-envelope prose whose bubble is several viewports tall; on the reporting
 * device such a bubble painted its accent fill and **none of its text**. Second,
 * that failure is a function of the row's height, not of the text: the same
 * prompt renders when it is clipped to a few lines, and stops rendering as soon
 * as it is allowed to grow again. So the transcript must never grow a row past a
 * small budget — an expanded "read everything here" state cannot exist, and the
 * rest of the prompt has to live somewhere a reader can scroll instead (the
 * prompt reader, `PromptReader.tsx`).
 *
 * The numbers are the ones the code block already uses (more than 16 lines, or
 * more than 10 000 characters, is long; 16 wrapped lines is what stays), so a
 * reader meets the same budget in both places.
 */
import { spacing } from "../../../theme/tokens";

/** Wrapped chat lines the collapsed bubble shows, and the count beyond which a
 * prompt is collapsed at all — the code block's own budget and threshold. */
export const PROMPT_COLLAPSE_LINES = 16;

/** Source characters beyond which a prompt is long even on a single line: one
 * 4 000-character paragraph wraps to well over 16 lines on a phone, and the
 * line count alone cannot see that. The code block's second clause. */
const PROMPT_COLLAPSE_CHARS = 10_000;

/**
 * Characters the collapsed bubble's preview may carry, which is what actually
 * bounds the row: 16 wrapped lines is roughly this many characters at a phone's
 * width and the default text size, and a preview that stopped at 16 *source*
 * lines would be a whole paragraph — several dozen wrapped lines — for prompts
 * that are one long line.
 */
const PROMPT_PREVIEW_CHARS = 720;

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
 * Whether this prompt is long enough to preview rather than to draw whole.
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
 * The head of a long prompt that the bubble draws, and whether the rest was left
 * out.
 *
 * The cut lands on a block boundary (a blank line) whenever one is close enough,
 * so a preview does not end inside a table or a fence; only a prompt that is one
 * unbroken block — the commonest shape for these — is cut mid-paragraph, which
 * Markdown renders as the partial sentence it is. The preview is never longer
 * than the budget, because its length is what bounds the row.
 */
export function promptPreview(text: string): { head: string; truncated: boolean; } {
  if (!isLongPrompt(text)) return { head: text, truncated: false };
  const byLine = text.split("\n", PROMPT_COLLAPSE_LINES + 1)
    .slice(0, PROMPT_COLLAPSE_LINES)
    .join("\n");
  const head = byLine.length > PROMPT_PREVIEW_CHARS
    ? byLine.slice(0, PROMPT_PREVIEW_CHARS)
    : byLine;
  // A block boundary inside the second half is worth taking: the preview then
  // ends where the author ended a block instead of mid-sentence.
  const boundary = head.lastIndexOf("\n\n");
  const cut = boundary > PROMPT_PREVIEW_CHARS / 2 ? head.slice(0, boundary) : head;
  return { head: cut.trimEnd(), truncated: true };
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
