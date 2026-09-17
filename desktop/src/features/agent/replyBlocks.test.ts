import type { MessageSegment } from "@future-os/thread-projection";
import { expect, it } from "vitest";
import { buildReplyBlocks } from "./replyBlocks";

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
