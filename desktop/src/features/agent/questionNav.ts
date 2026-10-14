import type { MessageSegment } from "@future-os/thread-projection";

/** Minimal shape needed to identify a row and read a reply's output. */
export interface TranscriptMessage {
  id: string;
  role: "assistant" | "system" | "tool" | "user";
  content?: string | null;
  segments?: readonly MessageSegment[] | null;
}

/**
 * A row index in the rendered message list, from 0 (oldest) to length - 1
 * (newest). The desktop transcript is drawn in chronological order — unlike
 * mobile's inverted list — so a *larger* index is further down the page.
 */
export type MessageIndex = number;

/**
 * A rung of the ↑/↓ ladder: something a press can come to rest on.
 *
 * A turn contributes up to two: the prompt that started it, and the end of the
 * reply it got. They are listed in transcript order, so a turn's tail rung sits
 * just *below* its own question — which is what lets ↓ read the conversation as
 * it was written.
 */
export type NavRung
  = | { kind: "question"; index: MessageIndex; messageId: string }
    | { kind: "replyTail"; index: MessageIndex; messageId: string };

/** A slice of a reply that counts as output: prose, or a tool call. */
function isOutputSegment(segment: MessageSegment): boolean {
  return segment.kind === "text" || segment.kind === "activity";
}

/**
 * Whether a reply has output on screen. A reply that is so far only reasoning
 * is a pause rather than an answer, and one whose prose the compaction replaced
 * still deserves a rung when it *ends* on the divider — the same rules mobile's
 * ladder applies, so both clients stop in the same places.
 */
function hasOutput(message: TranscriptMessage): boolean {
  const segments = message.segments ?? [];
  if (segments.length > 0)
    return segments.some(isOutputSegment);
  return (message.content ?? "").trim().length > 0;
}

/** Whether a reply with no output of its own ends on a compaction divider. */
function endsOnDivider(message: TranscriptMessage): boolean {
  const segments = message.segments ?? [];
  return segments.length > 0 && segments[segments.length - 1]!.kind === "compaction";
}

/**
 * Build the ladder for a transcript, in transcript order: for each user
 * message, its question rung and — when the reply that follows it has output —
 * the reply's tail rung.
 *
 * The turn above a question is the slice of items between it and the next
 * question down, and its tail is the *first* assistant message in that slice
 * with output (the projection writes one reply per turn, so this only matters
 * for a turn the store split across several assistant rows), or — when the
 * reply's prose was compacted away — the one that still ends on the divider.
 */
export function buildNavRungs(messages: readonly TranscriptMessage[]): NavRung[] {
  const rungs: NavRung[] = [];
  messages.forEach((message, index) => {
    if (message.role !== "user")
      return;
    rungs.push({ kind: "question", index, messageId: message.id });
    let replyIndex: MessageIndex | null = null;
    let fallbackIndex: MessageIndex | null = null;
    for (let cursor = index + 1; cursor < messages.length; cursor += 1) {
      const next = messages[cursor]!;
      if (next.role === "user")
        break;
      if (next.role !== "assistant")
        continue;
      if (hasOutput(next)) {
        replyIndex = cursor;
        break;
      }
      if (fallbackIndex === null && endsOnDivider(next))
        fallbackIndex = cursor;
    }
    const tail = replyIndex ?? fallbackIndex;
    if (tail !== null)
      rungs.push({ kind: "replyTail", index: tail, messageId: messages[tail]!.id });
  });
  return rungs;
}

/**
 * The rung ↑ lands on: the last question above the reader.
 *
 * `top` is the row the reader is reading (see `useQuestionNav`), so a question
 * at that row is not a destination — it is where they already are, and skipping
 * it is what makes the press after a landing move instead of re-aligning what is
 * already on screen.
 *
 * Replies are not destinations here at all: ↑ is the reader's own prompts, so a
 * press never stops on the answer between two of them. ↓ is what walks through
 * those.
 */
export function previousQuestion(
  rungs: readonly NavRung[],
  top: MessageIndex | null,
): NavRung | null {
  if (top === null)
    return null;
  for (let position = rungs.length - 1; position >= 0; position -= 1) {
    const rung = rungs[position]!;
    if (rung.kind === "question" && rung.index < top)
      return rung;
  }
  return null;
}

/**
 * The rung ↓ lands on: the closest stop below the reader, of either kind — so
 * each press moves one item through the transcript as written, prompt then
 * reply, prompt then reply…
 */
export function nextRung(
  rungs: readonly NavRung[],
  top: MessageIndex | null,
): NavRung | null {
  if (top === null)
    return null;
  for (const rung of rungs) {
    if (rung.index > top)
      return rung;
  }
  return null;
}

/**
 * Whether two rungs are the same step. Identity is what a rung *is* — its kind
 * and the row it stands on — so a rebuilt ladder (a new turn, a page of history,
 * a compaction that moved the divider) compares equal when it still holds the
 * same stop.
 */
export function sameRung(left: NavRung | null, right: NavRung | null): boolean {
  if (left === null || right === null)
    return left === right;
  return left.kind === right.kind && left.index === right.index;
}
