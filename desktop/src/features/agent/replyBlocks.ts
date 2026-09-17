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
