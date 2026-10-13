import type { TimelineItem } from "../../remote/types";

/**
 * The chat list is rendered inverted (`newestFirst`), so a row's *view index* is
 * its distance from the newest message: index 0 is the newest row and larger
 * indices are older. Everything here works in that space, because those are the
 * indices `FlatList.scrollToIndex` takes.
 *
 * The two directions are a ladder over question turns: ↑ walks the ladder one
 * step older, ↓ one step newer, and each turn contributes two rungs — the end of
 * its reply, and the question that started it. So consecutive presses cover, in
 * order: the previous answer's last slice, the previous question, the answer
 * before that, its question…
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
 * A rung on the question ladder — everything a jump can land on, in reading
 * order (newest first). A turn whose reply never arrives (or whose prose folded
 * entirely into thinking) contributes only its question.
 */
export type QuestionRung =
  | { kind: "question"; question: number }
  | { kind: "replyTail"; index: number };

/**
 * Where a rung lands: the row that takes the reading position, and which of its
 * edges takes it.
 *
 * The list is inverted, so `viewPosition: 1` puts a row's *start* just under
 * the viewport's top edge while `viewPosition: 0` puts its *end* just above the
 * bottom edge: the mirror swaps which edge `scrollToIndex` aligns. A question
 * lands by its **start** (the turn is read forward from there, the answer
 * filling the screen below it), and a reply's last slice lands by its **end**,
 * so the prose the reader came for is the last thing above the composer rather
 * than scrolled off the top.
 *
 * Landing a reply by the row *below* it — what this used to do, aligning that
 * row to the top edge — put the answer above the viewport instead: the reader
 * saw the next question at the top and the answer they asked for was gone.
 */
export interface RungLanding {
  index: number;
  edge: "top" | "bottom";
}

export function rungLanding(rung: QuestionRung): RungLanding {
  return rung.kind === "question"
    ? { index: rung.question, edge: "top" }
    : { index: rung.index, edge: "bottom" };
}

/** Whether two rungs are the same step of the ladder, across rebuilds. */
export function sameRung(left: QuestionRung, right: QuestionRung): boolean {
  if (left.kind !== right.kind) return false;
  if (left.kind === "question" && right.kind === "question")
    return left.question === right.question;
  if (left.kind === "replyTail" && right.kind === "replyTail")
    return left.index === right.index;
  return false;
}

/**
 * The rung's own position in the ladder, or null once it is not one of its
 * steps any more (a rebuilt ladder — a new turn, a page of history, a
 * compaction that moved the divider).
 */
export function rungPosition(
  rungs: readonly QuestionRung[],
  rung: QuestionRung | null,
): number | null {
  if (rung === null) return null;
  const position = rungs.findIndex(candidate => sameRung(candidate, rung));
  return position < 0 ? null : position;
}

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
 * Build the ladder for a transcript. The list is one question per turn plus its
 * (possibly streaming) reply, so the turn above a question is the slice of
 * items between it and the next question down: its reply tail is the oldest
 * assistant message in that slice that still has output on screen (prose, or a
 * tool call the reply ends on), or — when the reply's prose was compacted away
 * — the oldest one that still ends on the divider.
 */
export function buildLadder(
  items: readonly TimelineItem[],
  questions: readonly number[],
): QuestionRung[] {
  const rungs: QuestionRung[] = [];
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
    if (replyTail !== null) rungs.push({ kind: "replyTail", index: replyTail });
    rungs.push({ kind: "question", question });
  });
  return rungs;
}

/**
 * The rung ↑ lands on: the first stop whose row is entirely above the reader.
 *
 * `top` is the topmost row with any part on screen, so every row above it is
 * off screen — a destination whichever edge it lands by. The row *at* the top
 * edge is not one: its start is at the reader (or just above, once a drag has
 * moved a few pixels into it), which is "where I am", not "the previous one".
 * Skipping it is what makes the second press after landing on an answer reach
 * that answer's question instead of re-aligning what is already on screen, and
 * it is why a press never looks dead.
 */
export function previousRung(
  rungs: readonly QuestionRung[],
  top: number | null,
): QuestionRung | null {
  if (top === null) return null;
  for (const rung of rungs) {
    if (rungLanding(rung).index > top) return rung;
  }
  return null;
}

/** The rung ↓ lands on: the closest stop below the reader. The stop at the top
 *  edge is excluded (it is not below), which is what makes ↓ the inverse of ↑
 *  after a jump instead of re-selecting the landed rung. */
export function nextRung(
  rungs: readonly QuestionRung[],
  top: number | null,
): QuestionRung | null {
  if (top === null) return null;
  for (let position = rungs.length - 1; position >= 0; position -= 1) {
    const rung = rungs[position]!;
    if (rungLanding(rung).index <= top - 1) return rung;
  }
  return null;
}
