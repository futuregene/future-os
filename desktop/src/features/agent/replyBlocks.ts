import type { MessageSegment } from "@future-os/thread-projection";

export type StepSegment = Extract<MessageSegment, { kind: "thinking" | "activity" }>;
export type ReplyBlock
  = | { kind: "segment"; segment: MessageSegment }
    | { kind: "steps"; segments: StepSegment[] };

function foldableStep(segment: MessageSegment): segment is StepSegment {
  return segment.kind === "thinking"
    || (segment.kind === "activity" && segment.item.status !== "running");
}

/**
 * Match mobile's buildReplyBlocks: fold consecutive settled steps, including
 * failures, but never cross prose, compaction, running tools or the live tail.
 * This is presentation only; the shared chronological projection stays intact.
 */
export function buildReplyBlocks(segments: MessageSegment[], streaming = false): ReplyBlock[] {
  const blocks: ReplyBlock[] = [];
  const liveTail = streaming ? segments.length - 1 : -1;
  let index = 0;
  while (index < segments.length) {
    const segment = segments[index]!;
    if (index !== liveTail && foldableStep(segment)) {
      const run: StepSegment[] = [segment];
      let cursor = index + 1;
      while (cursor < segments.length && cursor !== liveTail) {
        const next = segments[cursor]!;
        if (!foldableStep(next))
          break;
        run.push(next);
        cursor += 1;
      }
      if (run.length > 1) {
        blocks.push({ kind: "steps", segments: run });
        index = cursor;
        continue;
      }
    }
    blocks.push({ kind: "segment", segment });
    index += 1;
  }
  return blocks;
}

/**
 * Split a settled reply into the work that led to its answer and the answer
 * itself — the whole reply minus everything before its last piece of prose.
 *
 * "The final answer" is the reply's last non-empty prose block: an agent that
 * works through a task writes notes between its calls, and the block after the
 * last call is what it is actually telling the reader. Everything after that
 * prose stays with the answer, so a trailing pause (reasoning, a divider) never
 * changes which block the reply opens on. A reply with no prose at all — one
 * that is still only work, or one whose answer the compaction replaced — has
 * no answer to isolate, so nothing is folded.
 */
export function splitFinalAnswer(blocks: readonly ReplyBlock[]): { process: ReplyBlock[]; answer: ReplyBlock[] } {
  for (let index = blocks.length - 1; index >= 0; index -= 1) {
    const block = blocks[index]!;
    if (block.kind === "segment" && block.segment.kind === "text" && block.segment.text.trim().length > 0)
      return { process: blocks.slice(0, index), answer: blocks.slice(index) };
  }
  return { process: [], answer: [...blocks] };
}

/**
 * Whether a rendered block still carries output a landing can stand on: prose,
 * or a tool call — the same two slice kinds the question ladder counts as a
 * reply's output. A folded run counts when any of its slices is a call; folded
 * reasoning alone is a pause, not output.
 */
function isOutputBlock(block: ReplyBlock): boolean {
  return block.kind === "segment"
    ? block.segment.kind === "text" || block.segment.kind === "activity"
    : block.segments.some(segment => segment.kind === "activity");
}

/**
 * The block a reply's tail rung lands on: the reply's *last* block that still
 * carries output, or null when nothing on screen is output (a reply that is so
 * far only reasoning, or one whose prose the compaction replaced).
 *
 * A press that reaches an answer is the reader asking for the answer, and a
 * reply opens with the work it ran: activity lines and reasoning first, the
 * prose they led to after. Landing on the row's start therefore puts the
 * reasoning the reader did not ask for at the reading edge and pushes the
 * output itself down the page. The anchor is that output — the last of it in
 * the reply — and whatever follows it (folded reasoning, a divider, the status
 * footer) stays below the reading edge. A reply that ends on a pause thus
 * anchors on the output before the pause, the same way the ladder skips a reply
 * that folded entirely into thinking.
 *
 * Mobile's twin of this function decides the same block for its bubbles; the
 * two must agree, because both clients promise the same thing about "the last
 * slice of the previous answer".
 */
export function replyTailBlock(blocks: readonly ReplyBlock[]): number | null {
  for (let index = blocks.length - 1; index >= 0; index -= 1) {
    if (isOutputBlock(blocks[index]!))
      return index;
  }
  return null;
}
