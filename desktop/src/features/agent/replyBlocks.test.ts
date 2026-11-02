import type { MessageSegment } from "@future-os/thread-projection";
import { describe, expect, it } from "vitest";
import { buildReplyBlocks, replyCopyText, splitFinalAnswer } from "./replyBlocks";

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

describe("folding a settled reply onto its final answer", () => {
  const split = (segments: MessageSegment[]) => splitFinalAnswer(buildReplyBlocks(segments));
  const answerIds = (result: ReturnType<typeof split>) => result.answer.map(block => block.kind === "steps" ? `steps:${block.segments[0]!.id}` : block.segment.id);

  it("keeps the last prose and folds everything before it", () => {
    const early = { id: "early", kind: "text", text: "Looking around" } as MessageSegment;
    const answer = { id: "answer", kind: "text", text: "It works" } as MessageSegment;
    const result = split([early, thought, tool, answer]);
    expect(answerIds(result)).toEqual(["answer"]);
    expect(result.process.map(block => (block.kind === "steps" ? "steps" : block.segment.id))).toEqual(["early", "steps"]);
  });

  it("leaves a reply alone when its answer already comes first", () => {
    const result = split([prose, thought, tool]);
    expect(answerIds(result)).toEqual(["text", "steps:thought"]);
    expect(result.process).toEqual([]);
  });

  it("has no answer to isolate when the reply holds no prose", () => {
    expect(split([thought, tool]).process).toEqual([]);
    expect(answerIds(split([thought, tool]))).toEqual(["steps:thought"]);
    expect(split([thought]).process).toEqual([]);
    expect(split([]).process).toEqual([]);
  });

  it("ignores blank prose — an empty slice is not an answer", () => {
    const blank = { id: "blank", kind: "text", text: "  \n" } as MessageSegment;
    const result = split([prose, blank]);
    expect(answerIds(result)).toEqual(["text", "blank"]);
    expect(result.process).toEqual([]);
  });

  it("keeps a trailing pause with the answer", () => {
    const result = split([thought, tool, prose, thought]);
    expect(answerIds(result)).toEqual(["text", "thought"]);
    expect(result.process.map(block => (block.kind === "steps" ? "steps" : block.segment.id))).toEqual(["steps"]);
  });
});

describe("replyCopyText", () => {
  const note: MessageSegment = { id: "note", kind: "text", text: "Working on it" };

  it("joins a reply's prose and leaves the work out", () => {
    expect(replyCopyText({ segments: [note, tool, prose], status: "complete" }, { fold: false }))
      .toBe("Working on it\n\nResponse");
  });

  it("copies only the answer while the reading setting folds the work away", () => {
    // The same rule the copy button beside the reply follows: the button belongs
    // to the answer it is shown beside, so the note before the tool call is not
    // part of what the reader is quoting.
    expect(replyCopyText({ segments: [note, tool, prose], status: "complete" }, { fold: true }))
      .toBe("Response");
  });

  it("never folds a reply that is still streaming", () => {
    expect(replyCopyText({ segments: [note, tool, prose], status: "streaming" }, { fold: true }))
      .toBe("Working on it\n\nResponse");
  });

  it("falls back to the raw content when the reply has no slices", () => {
    expect(replyCopyText({ content: "plain", status: "complete" }, { fold: true })).toBe("plain");
    expect(replyCopyText({ status: "complete" }, { fold: true })).toBe("");
  });
});
