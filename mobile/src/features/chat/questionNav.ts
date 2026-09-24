import type { TimelineItem } from "../../remote/types";

/**
 * The chat list is rendered inverted (`newestFirst`), so a row's *view index* is
 * its distance from the newest message: index 0 is the newest row and larger
 * indices are older. Everything here works in that space, because those are the
 * indices `FlatList.scrollToIndex` takes.
 *
 * The two directions are a ladder over question starts. The reader's position is
 * the top edge of the viewport, approximated by the topmost row the list reports
 * as visible: ↑ takes the question start closest above it, ↓ the closest below,
 * so each press moves exactly one question and the pair is symmetric.
 */
export interface ViewportRows {
  /** Visually topmost row that is at least partly visible, or null. */
  top: number | null;
  /**
   * That row is cut off by the viewport's top edge, so its own start is *above*
   * the reader rather than at it. This is what separates "the question I am
   * reading" from "the question just above me" — and it is why a jump never
   * re-aligns the row already on screen instead of moving.
   */
  topClipped: boolean;
}

export const EMPTY_ROWS: ViewportRows = { top: null, topClipped: false };

/** View-space indices of the user messages, ascending (oldest last). */
export function questionIndices(items: readonly TimelineItem[]): number[] {
  const indices: number[] = [];
  items.forEach((item, index) => {
    if (item.kind === "message" && item.role === "user") indices.push(index);
  });
  return indices;
}

/**
 * The question ↑ lands on: the closest question start above the reader. When the
 * topmost visible row is a question that is already fully on screen, its start
 * is *below* the reader, and ↑ steps to the one above instead of re-aligning it.
 */
export function previousQuestion(
  questions: readonly number[],
  rows: ViewportRows,
): number | null {
  if (rows.top === null) return null;
  const limit = rows.topClipped ? rows.top : rows.top + 1;
  for (const index of questions) if (index >= limit) return index;
  return null;
}

/**
 * The question ↓ lands on: the closest question start below the reader. The row
 * at the top edge is excluded (its start is not below), which is what makes ↓
 * the inverse of ↑ after a jump instead of re-selecting the landed question.
 */
export function nextQuestion(
  questions: readonly number[],
  rows: ViewportRows,
): number | null {
  if (rows.top === null) return null;
  const limit = rows.top - 1;
  for (let position = questions.length - 1; position >= 0; position -= 1) {
    const index = questions[position]!;
    if (index <= limit) return index;
  }
  return null;
}

/**
 * The topmost visible row and whether its start is above the viewport's top
 * edge. Two viewability configurations supply it: "at least partly visible"
 * gives the row, "fully visible" tells whether it is cut. Deriving the cut flag
 * from the list's own accounting keeps the anchor independent of any coordinate
 * space a platform might measure rows in.
 */
export function viewportRows({
  partialTop,
  fullTop,
}: {
  partialTop: number | null;
  fullTop: number | null;
}): ViewportRows {
  if (partialTop === null) return EMPTY_ROWS;
  return {
    top: partialTop,
    // The topmost visible row can only stick out at the top edge.
    topClipped: fullTop === null || partialTop > fullTop,
  };
}
