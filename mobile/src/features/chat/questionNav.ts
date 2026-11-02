import type { TimelineItem } from "../../remote/types";

/**
 * The chat list is rendered inverted (`newestFirst`), so a row's *view index* is
 * its distance from the newest message: index 0 is the newest row and larger
 * indices are older. Everything here works in that space, because those are the
 * indices `FlatList.scrollToIndex` takes.
 *
 * The ↑/↓ pair walks one list: the reader's own prompts, newest first. ↑ takes
 * the closest prompt above the reader, ↓ the closest prompt below, and answers
 * are never a stop in either direction — the pair is for finding what was
 * *asked*, so a long reply costs one press to pass, not one per screenful.
 * Desktop's `questionNav` decides the same two landings over its chronological
 * row list; the two must agree, because both clients promise the same walk.
 */

/** View-space indices of the user messages, ascending (oldest last). */
export function questionIndices(items: readonly TimelineItem[]): number[] {
  const indices: number[] = [];
  items.forEach((item, index) => {
    if (item.kind === "message" && item.role === "user") indices.push(index);
  });
  return indices;
}

/**
 * The rung ↑ lands on: the first prompt older than the reader.
 *
 * `top` is the topmost row with any part on screen, so every row above it is
 * off screen. The row *at* the top edge is not a destination: its start is at
 * the reader (or just above, once a drag has moved a few pixels into it), which
 * is "where I am", not "the previous one". Skipping it is what makes the press
 * after a landing reach the prompt above instead of re-aligning what is already
 * on screen, and it is why a press never looks dead.
 */
export function previousQuestion(
  questions: readonly number[],
  top: number | null,
): number | null {
  if (top === null) return null;
  for (const index of questions) {
    if (index > top) return index;
  }
  return null;
}

/**
 * The rung ↓ lands on: the closest prompt newer than the reader.
 *
 * The stop at the top edge is excluded (it is not below), which is what makes ↓
 * land on something new after a jump instead of re-selecting the landed row. A
 * press from inside an answer therefore reaches the next prompt past it — the
 * answer itself is never a stop, however many screens it fills.
 */
export function nextQuestion(
  questions: readonly number[],
  top: number | null,
): number | null {
  if (top === null) return null;
  for (let position = questions.length - 1; position >= 0; position -= 1) {
    const index = questions[position]!;
    if (index < top) return index;
  }
  return null;
}
