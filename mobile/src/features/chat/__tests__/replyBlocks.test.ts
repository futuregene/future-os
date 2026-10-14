import type { TimelineSegment } from "../../../remote/types";
import { buildReplyBlocks, replyTailBlock } from "../messages/ReplyBlocks";

// The anchor rule is pure, but it lives with the bubble's renderer, which draws
// glyphs: stubbed the way the other message-row suites stub them.
jest.mock("lucide-react-native", () => ({
  Brain: "Brain",
  ChevronDown: "ChevronDown",
  ChevronUp: "ChevronUp",
  Wrench: "Wrench",
}));
jest.mock("../../../components/MarkdownText", () => ({ MarkdownText: "MarkdownText" }));

const text = (id: string): TimelineSegment => ({ id, kind: "text", text: "…" });
const thinking = (id: string): TimelineSegment => ({ id, kind: "thinking", text: "…" });
const tool = (id: string): TimelineSegment => ({
  id,
  kind: "tool",
  tool: { name: "read", complete: true },
});
const compaction = (id: string): TimelineSegment => ({
  id,
  kind: "compaction",
  status: "completed",
});

/** The block index a reply's tail rung anchors on. */
const anchorOf = (segments: TimelineSegment[], streaming?: boolean) =>
  replyTailBlock(buildReplyBlocks(segments, streaming));

describe("the block a reply's tail rung lands on", () => {
  test("the last output block wins, not the first", () => {
    // The shape a long exchange has: work, prose, more work, the answer. A
    // press that reaches this reply is asking for the answer.
    const segments = [
      thinking("h1"),
      tool("w1"),
      text("t1"),
      thinking("h2"),
      text("t2"),
    ];
    const blocks = buildReplyBlocks(segments);
    expect(blocks.map(block => (block.kind === "steps" ? "steps" : block.segment.kind)))
      .toEqual(["steps", "text", "thinking", "text"]);
    expect(anchorOf(segments)).toBe(3);
  });

  test("a folded burst of tool calls is output", () => {
    // Consecutive stopped step rows collapse into one summary line; a run that
    // contains a call still stands for work the reader can land on.
    const segments = [text("t1"), thinking("h1"), tool("w1"), thinking("h2")];
    const blocks = buildReplyBlocks(segments);
    expect(blocks.map(block => (block.kind === "steps" ? "steps" : block.segment.kind)))
      .toEqual(["text", "steps"]);
    expect(anchorOf(segments)).toBe(1);
  });

  test("a folded burst of reasoning alone is a pause", () => {
    // The same fold without a call behind it: nothing to land on.
    expect(anchorOf([text("t1"), thinking("h1"), thinking("h2")])).toBe(0);
  });

  test("reasoning after the last output is a pause, not the anchor", () => {
    // The reply ends on folded reasoning: the output before it is where the
    // press lands, and the pause stays below the reading edge.
    expect(anchorOf([text("t1"), thinking("h1")])).toBe(0);
  });

  test("a divider after the last output is not output", () => {
    // A compaction divider sits below the reading edge like a pause does, with
    // a folded reasoning row behind it for good measure.
    expect(anchorOf([text("t1"), compaction("c1")])).toBe(0);
    expect(anchorOf([text("t1"), compaction("c1"), thinking("h1")])).toBe(0);
  });

  test("a reply with nothing on screen has no anchor at all", () => {
    // Still reasoning, or only a divider: the landing keeps the row's start.
    expect(anchorOf([thinking("h1")])).toBeNull();
    expect(anchorOf([compaction("c1")])).toBeNull();
    expect(anchorOf([])).toBeNull();
  });

  test("the slice being written right now is not folded into a run", () => {
    // A streaming reply keeps its last slice out of the fold, so the anchor
    // follows the blocks the row really renders — and it is the last output
    // block either way, which is what keeps the landing stable across the
    // moment the run settles.
    const segments = [text("t1"), tool("w1"), tool("w2")];
    expect(buildReplyBlocks(segments, true)).toHaveLength(3);
    expect(anchorOf(segments, true)).toBe(2);
    expect(buildReplyBlocks(segments, false)).toHaveLength(2);
    expect(anchorOf(segments, false)).toBe(1);
  });
});
