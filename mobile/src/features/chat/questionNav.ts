import type { TimelineItem } from "../../remote/types";

/**
 * The chat list is rendered inverted (`newestFirst`), so a row's *view index* is
 * its distance from the newest message: index 0 is the newest row and larger
 * indices are older. Everything here works in that space, because those are the
 * indices `FlatList.scrollToIndex` takes.
 *
 * The two directions are a ladder over question turns: ↑ walks the ladder one
 * step older, ↓ one step newer. Each turn has up to three rungs — its reply
 * tail, the question itself, and the question's own tail — so consecutive
 * presses of one direction cover, in order: the previous answer's last slice,
 * the previous question, the end of that question, the next answer's last
 * slice…
 */
export interface ViewportRows {
  /** Visually topmost row that is at least partly visible, or null. */
  top: number | null;
  /**
   * That row is cut off by the viewport's top edge, so its own start is *above*
   * the reader rather than at it. This is what separates "the row I am
   * reading" from "the row just above me" — and it is why a jump never
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
 * A rung on the question ladder — everything a jump can land on, in reading
 * order (newest first). `questionEnd` needs no index of its own: it lands
 * through the row below the question's tail, so it is a jump and a release
 * rather than an alignment. `replyTail` gets no rung in a turn that ends with
 * a plain question (the reply missing, still to come, or folded entirely into
 * thinking).
 */
export type QuestionRung =
  | { kind: "question"; question: number; tail: number | null }
  | { kind: "questionEnd"; question: number; tail: number | null }
  | { kind: "replyTail"; index: number; nextQuestion: number | null };

/** A ladder with neither a question nor a reply tail has nothing to jump to. */
export const EMPTY_LADDER: QuestionRung[] = [];

/**
 * A slice of AI output the reply tail rung stands for — prose, or the last
 * tool call of a reply that ends on one.
 */
function isOutputSegment(segment: { kind: string }): boolean {
  return segment.kind === "text" || segment.kind === "tool";
}

/**
 * Whether a reply without output still deserves a tail rung: only when it
 * *ends* on a compaction divider (the summary replaced the prose, and the
 * reader should still be able to land where the answer was). A reply that
 * ends on folded thinking is just a pause — the next press should skip it.
 */
function endsOnDivider(segments: readonly { kind: string }[]): boolean {
  return segments.length > 0 && segments[segments.length - 1]!.kind === "compaction";
}

/**
 * Build the ladder for a transcript. The list is one question per turn plus
 * one (possibly streaming) assistant message, so the turn above a question is
 * the slice of items between it and the next question down: its reply tail is
 * the oldest assistant message in that slice that still has output on screen
 * (prose, or a tool call the reply ends on), or — when the reply's prose was
 * compacted away — the oldest one that still ends on the divider.
 * `lastIndex` is the list's own row count, the rung the oldest questionEnd
 * bounces against when the tail is a question.
 */
export function buildLadder(
  items: readonly TimelineItem[],
  questions: readonly number[],
): { rungs: QuestionRung[]; lastIndex: number } {
  const lastIndex = Math.max(0, items.length - 1);
  const rungs: QuestionRung[] = [];
  let nextQuestion: number | null = null;
  questions.forEach((question, position) => {
    let replyTail: number | null = null;
    let fallback: number | null = null;
    const above = questions[position - 1];
    const newest = above === undefined ? 0 : above + 1;
    for (let index = question - 1; index >= newest; index -= 1) {
      const item = items[index];
      if (!item || item.kind !== "message" || item.role !== "assistant") continue;
      const segments = item.segments ?? [];
      if (
        segments.length > 0
          ? segments.some(isOutputSegment)
          : item.text.trim().length > 0
      ) {
        replyTail = index;
        break;
      }
      if (fallback === null && endsOnDivider(segments)) {
        fallback = index;
      }
    }
    if (replyTail === null) replyTail = fallback;
    if (replyTail !== null) {
      rungs.push({ kind: "replyTail", index: replyTail, nextQuestion });
    }
    rungs.push({ kind: "question", question, tail: replyTail });
    rungs.push({ kind: "questionEnd", question, tail: replyTail });
    nextQuestion = question;
  });
  return { rungs, lastIndex };
}

/** The rung ↑ lands on: the closest rung above the reader. When the topmost
 *  visible row is a rung that is already fully on screen, its start is *below*
 *  the reader, and ↑ steps to the rung above instead of re-aligning it. A
 *  questionEnd needs no target row — it lands through the row below its
 *  question's tail, which the reader can already see — so the question's own
 *  row at the top edge satisfies it, and it is never the selection.
 *  A reply tail is the one rung a cut topmost row does *not* satisfy: its
 *  landing is a bottom-edge alignment, and a top edge parked anywhere above
 *  the tail's end — the jump's own approximate landing included — means the
 *  reader has not necessarily seen the answer out. ↑ then re-issues the same
 *  rung, which the re-align pass lands exactly; the press after it climbs on
 *  to the question. */
export function previousRung(
  rungs: readonly QuestionRung[],
  rows: ViewportRows,
): QuestionRung | null {
  if (rows.top === null) return null;
  const limit = rows.topClipped ? rows.top : rows.top + 1;
  for (const rung of rungs) {
    const index = rung.kind === "replyTail" ? rung.index : rung.question;
    if (index === null || index < limit) continue;
    if (rung.kind === "questionEnd" && rung.question === rows.top) continue;
    // A question whose row is cut by the top edge has its start above the
    // reader — the tail landing that brought them here already showed it, so
    // the press climbs on instead of re-aligning the same row.
    if (rung.kind === "question" && rung.question === rows.top && rows.topClipped) continue;
    return rung;
  }
  return null;
}

/** The rung ↓ lands on: the closest rung below the reader. The rung at the
 *  top edge is excluded (its start is not below), which is what makes ↓ the
 *  inverse of ↑ after a jump instead of re-selecting the landed rung. A
 *  questionEnd stands on its question like a replyTail stands on its tail:
 *  both still belong to the reader until the top edge walks past that row. */
export function nextRung(
  rungs: readonly QuestionRung[],
  rows: ViewportRows,
): QuestionRung | null {
  if (rows.top === null) return null;
  const limit = rows.top - 1;
  for (let position = rungs.length - 1; position >= 0; position -= 1) {
    const rung = rungs[position]!;
    const index = rung.kind === "replyTail" ? rung.index : rung.question;
    if (index <= limit) return rung;
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
