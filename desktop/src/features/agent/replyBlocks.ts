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
 * The plain text a reply copies as: its prose slices, joined with a blank line.
 *
 * One rule, two callers. The hover button beside a reply copies this text, and
 * `thread.copyReply` (`⌘⌥C`) copies it too — a keyboard path that copied
 * something else (raw markdown including the activity lines, say) would be a
 * second, wrong answer to "what does this reply say".
 *
 * `fold` matters for the same reason: while the reading setting folds a settled
 * reply onto its answer, the visible answer is the trailing prose only, and the
 * notes the model wrote between its calls are not part of what the button beside
 * it offers.
 */
export function replyCopyText(
  message: { content?: string; segments?: readonly MessageSegment[]; status?: string },
  detail: { fold: boolean },
): string {
  const streaming = message.status === "streaming";
  const segments = message.segments && message.segments.length > 0 ? message.segments : null;
  const blocks = segments ? buildReplyBlocks([...segments], streaming) : null;
  const folded = blocks && detail.fold && !streaming ? splitFinalAnswer(blocks) : null;
  if (folded)
    return proseText(folded.answer);
  if (segments)
    return segments.flatMap(segment => (segment.kind === "text" ? [segment.text] : [])).join("\n\n");
  return message.content ?? "";
}

/** The prose a set of rendered blocks carries, joined the way the copy button wants it. */
export function proseText(blocks: readonly ReplyBlock[]): string {
  return blocks
    .flatMap(block => (block.kind === "segment" && block.segment.kind === "text" ? [block.segment.text] : []))
    .join("\n\n");
}
