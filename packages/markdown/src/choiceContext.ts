/**
 * How much of a card's `context` is shown before the rest folds away.
 *
 * The card asks the reader to answer it without scrolling back up the reply
 * (§3), so the context has to be long enough to carry the facts the decision
 * turns on. But nothing bounds what a model writes, and a card whose context
 * grows to twenty lines stops being a card and becomes the reply it was meant to
 * replace. The fold is the compromise: below the threshold the whole text shows
 * and there is no control at all; above it, the first four lines show with a
 * "show all" button that is one click from the full text.
 *
 * The decision is made from the text, not from the rendered box, and it *drives*
 * the CSS clamp rather than being corrected by it. That direction is what makes
 * a wrong estimate harmless: if the estimate says "short", no clamp is applied
 * and nothing can be hidden; if it says "long", the clamp hides the tail and the
 * button is right there. There is no state in which text is clipped with no way
 * to read it — the failure mode a `scrollHeight > clientHeight` measurement
 * would have whenever layout and measurement disagree (or when there is no
 * layout at all, as in every test).
 */
export const CHOICE_CONTEXT_VISIBLE_LINES = 4;

/**
 * Fold only when hiding at least two lines, so a context that is barely over
 * the line budget does not trade a button for one hidden line.
 */
const FOLD_FROM_LINE = CHOICE_CONTEXT_VISIBLE_LINES + 2;

/** East Asian wide/fullwidth ranges: one character takes two Latin widths. */
const WIDE = /[\u1100-\u115F\u2E80-\uA4CF\uA960-\uA97F\uAC00-\uD7A3\uF900-\uFAFF\uFE30-\uFE4F\uFF00-\uFF60\uFFE0-\uFFE6]/;

/**
 * Wrapped line count for `text` in a box that holds `unitsPerLine` width units,
 * where a CJK character counts as two units and everything else as one. Rough by
 * construction — real layout depends on the font — but it only has to be right
 * about whether a context is four lines or fourteen, which it is.
 */
export function estimateContextLines(text: string, unitsPerLine: number): number {
  const perLine = Math.max(1, unitsPerLine);
  let lines = 0;
  for (const paragraph of text.split("\n")) {
    let units = 0;
    for (const character of paragraph) units += WIDE.test(character) ? 2 : 1;
    lines += Math.max(1, Math.ceil(units / perLine));
  }
  return lines;
}

/**
 * Whether a card should fold its context down to
 * `CHOICE_CONTEXT_VISIBLE_LINES` lines. `unitsPerLine` is the renderer's own
 * estimate of its text column: a 768px desktop card at 12px holds about 120
 * units, a 350pt phone card at 13px about 48.
 */
export function choiceContextFolds(context: string, unitsPerLine: number): boolean {
  return estimateContextLines(context, unitsPerLine) >= FOLD_FROM_LINE;
}
