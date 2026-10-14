import type { TimelineItem } from "../../remote/types";

/**
 * The chat list is rendered inverted (`newestFirst`), so a row's *view index* is
 * its distance from the newest message: index 0 is the newest row and larger
 * indices are older. Everything here works in that space, because those are the
 * indices `FlatList.scrollToIndex` takes.
 *
 * The two directions share one ladder over question turns and read it
 * differently. Each turn contributes two rungs — the question that started it,
 * and the end of the reply it got — listed newest first, so a turn's reply rung
 * sits *before* its own question. ↓ takes the very next rung below, which walks
 * the transcript as it was written: input, output, input, output… ↑ takes only
 * the questions, so a press of it always reaches the reader's own previous
 * prompt instead of stopping on the answer in between.
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
 * The view index of the row a rung lands on.
 *
 * A landing always takes the row's **start** — the top edge of the viewport,
 * where a reader enters a row and reads it down. (Landing a reply by the row
 * *below* it — what this used to do — put the answer above the viewport
 * entirely: the reader saw the next question at the top and the answer they
 * asked for was gone.)
 *
 * A reply's *start* is not quite where its landing stops: an exchange opens with
 * the work it ran and closes with the output it produced, so the walk lifts the
 * row to the reply's last output slice — which only the bubble can measure (see
 * `replyTailBlock`). This function still answers for the row; `useQuestionNav`
 * applies the lift, and a reply nobody has measured yet keeps the plain start
 * landing.
 *
 * The transcript's newest row is no exception: it lands like everything else,
 * with the list's actual tail a scroll below it and the composer's own "back to
 * latest" the way there.
 */
export function rungIndex(rung: QuestionRung): number {
  return rung.kind === "question" ? rung.question : rung.index;
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
 * The rung ↑ lands on: the first question whose row is entirely above the
 * reader.
 *
 * `top` is the topmost row with any part on screen, so every row above it is
 * off screen. The row *at* the top edge is not a destination: its start is at
 * the reader (or just above, once a drag has moved a few pixels into it), which
 * is "where I am", not "the previous one". Skipping it is what makes the press
 * after a landing reach the prompt above instead of re-aligning what is already
 * on screen, and it is why a press never looks dead.
 *
 * Replies are not destinations here: ↑ is the reader's own prompts, and the
 * answers between two of them are what ↓ walks through.
 */
export function previousQuestion(
  rungs: readonly QuestionRung[],
  top: number | null,
): QuestionRung | null {
  if (top === null) return null;
  for (const rung of rungs) {
    if (rung.kind !== "question") continue;
    if (rungIndex(rung) > top) return rung;
  }
  return null;
}

/** The rung ↓ lands on: the closest stop below the reader, of either kind — so
 *  each press of it moves one item through the transcript as written. The stop
 *  at the top edge is excluded (it is not below), which is what makes ↓ land on
 *  something new after a jump instead of re-selecting the landed rung. */
export function nextRung(
  rungs: readonly QuestionRung[],
  top: number | null,
): QuestionRung | null {
  if (top === null) return null;
  for (let position = rungs.length - 1; position >= 0; position -= 1) {
    const rung = rungs[position]!;
    if (rungIndex(rung) <= top - 1) return rung;
  }
  return null;
}
