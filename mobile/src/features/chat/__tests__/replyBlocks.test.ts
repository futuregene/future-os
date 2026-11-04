import type { TimelineSegment } from "../../../remote/types";
import { buildReplyBlocks, splitFinalAnswer } from "../messages/ReplyBlocks";

// The fold is pure, but it lives with the bubble's renderer, which draws
// glyphs: stubbed the way the other message-row suites stub them.
jest.mock("lucide-react-native", () => ({
  Brain: "Brain",
  ChevronDown: "ChevronDown",
  ChevronUp: "ChevronUp",
  ListTree: "ListTree",
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

describe("folding a settled reply onto its final answer", () => {
  const named = (id: string): TimelineSegment => ({ id, kind: "text", text: "…" });
  const ids = (blocks: ReturnType<typeof splitFinalAnswer>["answer"]) =>
    blocks.map(block => (block.kind === "steps" ? `steps:${block.segments[0]!.id}` : block.segment.id));

  test("the last prose is the answer; everything before it is the work", () => {
    const result = splitFinalAnswer(buildReplyBlocks([named("early"), thinking("h1"), tool("w1"), named("answer")]));
    expect(ids(result.answer)).toEqual(["answer"]);
    expect(result.process).toHaveLength(2);
  });

  test("a reply whose answer already comes first has no work to fold", () => {
    const result = splitFinalAnswer(buildReplyBlocks([text("t1"), thinking("h1"), tool("w1")]));
    expect(ids(result.answer)).toEqual(["t1", "steps:h1"]);
    expect(result.process).toEqual([]);
  });

  test("a reply with no prose has no answer to isolate", () => {
    expect(splitFinalAnswer(buildReplyBlocks([thinking("h1"), tool("w1")])).process).toEqual([]);
    expect(splitFinalAnswer(buildReplyBlocks([])).process).toEqual([]);
  });

  test("blank prose is not an answer", () => {
    const blank: TimelineSegment = { id: "blank", kind: "text", text: " \n" };
    expect(splitFinalAnswer(buildReplyBlocks([text("t1"), blank])).process).toEqual([]);
  });

  test("a trailing pause stays with the answer", () => {
    const result = splitFinalAnswer(buildReplyBlocks([thinking("h1"), tool("w1"), text("t1"), thinking("h2")]));
    expect(ids(result.answer)).toEqual(["t1", "h2"]);
    expect(result.process).toHaveLength(1);
  });
});

describe("folding work that did not report back", () => {
  const completed = (id: string): TimelineSegment => ({
    id,
    kind: "tool",
    tool: { name: "read", complete: true, status: "completed" },
  });
  const running = (id: string): TimelineSegment => ({
    id,
    kind: "tool",
    tool: { name: "shell", complete: false, status: "running" },
  });
  const interrupted = (id: string): TimelineSegment => ({
    id,
    kind: "tool",
    tool: { name: "shell", complete: true, status: "interrupted" },
  });
  const blocks = (segments: TimelineSegment[]) =>
    buildReplyBlocks(segments).map(block =>
      block.kind === "steps" ? `steps:${block.segments.map(segment => segment.id).join(",")}` : block.segment.id);

  test("a call still running keeps its own row", () => {
    expect(blocks([completed("a"), running("b"), completed("c"), completed("d")]))
      .toEqual(["a", "b", "steps:c,d"]);
  });

  test("an interrupted call keeps its own row instead of joining a summary", () => {
    // A folded run is summarised as work that ran ("Ran 3 commands"), which is
    // exactly the claim an interrupted call cannot back up.
    expect(blocks([completed("a"), completed("b"), interrupted("c"), completed("d")]))
      .toEqual(["steps:a,b", "c", "d"]);
  });
});
