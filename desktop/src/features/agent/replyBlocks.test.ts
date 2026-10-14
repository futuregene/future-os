import type { MessageSegment } from "@future-os/thread-projection";
import { describe, expect, it } from "vitest";
import { buildReplyBlocks, replyTailBlock } from "./replyBlocks";

const thought: MessageSegment = { id: "thought", kind: "thinking", text: "Reasoning" };
const tool: MessageSegment = { id: "tool", kind: "activity", item: { id: "tool", kind: "read", status: "completed", target: "file.ts" } };
const failed: MessageSegment = { id: "failed", kind: "activity", item: { id: "failed", kind: "shell", status: "failed" } };
const running: MessageSegment = { id: "running", kind: "activity", item: { id: "running", kind: "shell", status: "running" } };
const prose: MessageSegment = { id: "text", kind: "text", text: "Response" };
const compaction: MessageSegment = { id: "compact", kind: "compaction" };

it("folds contiguous mixed settled steps, including failures, without changing their order", () => {
  const segments = [thought, tool, failed];
  expect(buildReplyBlocks(segments)).toEqual([{ kind: "steps", segments }]);
  expect(segments).toEqual([thought, tool, failed]);
});

it.each([prose, compaction, running])("does not fold across a $kind boundary", (boundary) => {
  expect(buildReplyBlocks([thought, tool, boundary, failed, thought])).toEqual([
    { kind: "steps", segments: [thought, tool] },
    { kind: "segment", segment: boundary },
    { kind: "steps", segments: [failed, thought] },
  ]);
});

it.each([thought, tool, failed])("keeps a lone $kind step as an individual row", (segment) => {
  expect(buildReplyBlocks([segment])).toEqual([{ kind: "segment", segment }]);
});

it("never folds the live tail even if it is a settled tool or a thinking slice", () => {
  for (const tail of [thought, tool, failed]) {
    expect(buildReplyBlocks([thought, tool, tail], true)).toEqual([
      { kind: "steps", segments: [thought, tool] },
      { kind: "segment", segment: tail },
    ]);
    expect(buildReplyBlocks([tool, tail], true).every(block => block.kind === "segment")).toBe(true);
  }
});

it("folds the former tail once another slice arrives or the reply settles", () => {
  expect(buildReplyBlocks([thought, tool, prose], true)).toEqual([
    { kind: "steps", segments: [thought, tool] },
    { kind: "segment", segment: prose },
  ]);
  expect(buildReplyBlocks([thought, tool], false)).toEqual([{ kind: "steps", segments: [thought, tool] }]);
});

it("folds tools-only and thoughts-only sequences and handles an empty reply", () => {
  expect(buildReplyBlocks([])).toEqual([]);
  expect(buildReplyBlocks([tool, failed])).toEqual([{ kind: "steps", segments: [tool, failed] }]);
  expect(buildReplyBlocks([thought, thought])).toEqual([{ kind: "steps", segments: [thought, thought] }]);
});

describe("the block a reply's tail rung lands on", () => {
  const anchorOf = (segments: MessageSegment[], streaming = false) =>
    replyTailBlock(buildReplyBlocks(segments, streaming));

  it("takes the last output block, not the first", () => {
    // The shape a long reply has: work, prose, more work, the answer. A press
    // that reaches this reply is asking for the answer.
    const segments = [thought, tool, prose, thought, { id: "text2", kind: "text", text: "Final" } as MessageSegment];
    expect(anchorOf(segments)).toBe(3);
  });

  it("counts a folded run that holds a call as output", () => {
    const segments = [prose, thought, tool];
    expect(buildReplyBlocks(segments).map(block => (block.kind === "steps" ? "steps" : block.segment.kind)))
      .toEqual(["text", "steps"]);
    expect(anchorOf(segments)).toBe(1);
  });

  it("treats reasoning after the last output as a pause", () => {
    expect(anchorOf([prose, thought])).toBe(0);
    expect(anchorOf([prose, compaction])).toBe(0);
  });

  it("has no anchor when nothing on screen is output", () => {
    // Still reasoning, or only a divider: the landing keeps the row's start.
    expect(anchorOf([thought])).toBeNull();
    expect(anchorOf([compaction])).toBeNull();
    expect(anchorOf([])).toBeNull();
    expect(anchorOf([thought, thought])).toBeNull();
  });

  it("anchors on the same block whether or not the reply is still streaming", () => {
    // The live tail is never folded, so the block indexes differ — but the
    // block the press lands on is the last output either way, which is what
    // keeps a landing stable across the moment a run settles.
    const segments = [prose, tool, tool];
    expect(anchorOf(segments, true)).toBe(2);
    expect(anchorOf(segments, false)).toBe(1);
    expect(anchorOf(segments, true)).not.toBeNull();
  });
});
