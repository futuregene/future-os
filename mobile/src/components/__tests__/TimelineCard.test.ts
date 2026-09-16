import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { TimelineCard } from "../TimelineCard";
import type { TimelineItem, TimelineSegment, TimelineToolRow } from "../../remote/types";

jest.mock("../MarkdownText", () => ({ MarkdownText: "MarkdownText" }));
jest.mock("lucide-react-native", () => Object.fromEntries([
  "AlertTriangle", "Brain", "Check", "ChevronDown", "ChevronUp", "CircleAlert", "Copy", "FileText", "Paperclip", "Pencil", "TerminalSquare", "TriangleAlert", "X",
].map(name => [name, name])));
// Interpolate the options a folded step-run summary passes so its aggregated
// line is assertable verbatim; every other key stays bare.
jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: Record<string, unknown>) => {
      if (!options) return key;
      const action = options.action ? `(${options.action})` : "";
      return `${key}${action}×${options.count ?? ""}`;
    },
    i18n: { language: "en" },
  }),
}));

let tree: ReactTestRenderer;
afterEach(() => { if (tree) act(() => tree.unmount()); });
const reply = (fields: Partial<Extract<TimelineItem, { kind: "message" }>>): TimelineItem => ({
  kind: "message", role: "assistant", id: "a", text: "reply", ...fields,
});
function render(item: TimelineItem) {
  act(() => { tree = create(createElement(TimelineCard, { item })); });
}
function hasText(text: string) {
  return tree.root.findAll(node => node.props.children === text).length > 0;
}
// One rendered row carries each label, but a React Native <Text> contributes a
// composite and a host instance — count host nodes so a row counts once.
function countText(text: string) {
  return tree.root.findAll(node => typeof node.type === "string" && node.props.children === text).length;
}
/** The tappable row whose own label is exactly `text` (never a reworded parent). */
function rowButton(text: string) {
  return tree.root.findAll(node =>
    typeof node.props.onPress === "function"
    && node.findAll(inner => inner.props.children === text).length > 0)[0]!;
}
// A `accessibilityLabel` on a Pressable propagates to every host view under it;
// count host nodes so the row is counted once.
function countLabel(label: string) {
  return tree.root.findAll(node =>
    typeof node.type === "string" && node.props.accessibilityLabel === label).length;
}
/** How many of a mocked glyph are rendered (the mock's components are name strings). */
function countGlyph(name: string) {
  return tree.root.findAll(node => (node.type as unknown as string) === name).length;
}

const prose = (id: string, text = "answer"): TimelineSegment => ({ id, kind: "text", text });
const thinking = (id: string, text = id): TimelineSegment => ({ id, kind: "thinking", text });
const tool = (id: string, fields: Partial<TimelineToolRow> = {}): TimelineSegment => ({
  id,
  kind: "tool",
  tool: { name: "shell", complete: true, status: "completed", detail: `cmd ${id}`, ...fields },
});

/** A folded summary line: the counts as `stepSummary(<verb>)×<count>`, joined. */
function stepsSummary(...counts: [key: string, count: number][]) {
  return counts
    .map(([kind, count]) => `chat.stepSummary(chat.step${kind})×${count}`)
    .join(" · ");
}

const THINK_RUN = stepsSummary(["Think", 1], ["Run", 1]);

// Consecutive thinking/tool rows are one line on a phone (the desktop transcript
// spends a full line per activity); the individual rows stay one tap away, and
// each of those still opens its own detail.
test("a run of thinking and tool rows folds into one summary line", () => {
  render(reply({
    segments: [
      prose("p1"),
      thinking("k1"), tool("c1"), thinking("k2"), tool("c2"),
      prose("p2"),
      thinking("k3"), tool("c3"),
    ],
  }));
  // Prose splits the reply into one folded run per stretch of activity.
  expect(countText(THINK_RUN)).toBe(1);
  expect(countText(stepsSummary(["Think", 2], ["Run", 2]))).toBe(1);
  expect(hasText("chat.thoughtCompleted")).toBe(false);
  expect(hasText("chat.runCompleted")).toBe(false);
});

test("the folded run opens into its rows, each still opening its own detail", () => {
  render(reply({
    segments: [thinking("k1", "why it broke"), tool("c1", { detail: "ls -la" }), thinking("k2"), tool("c2")],
  }));
  act(() => rowButton(stepsSummary(["Think", 2], ["Run", 2])).props.onPress());
  expect(countText("chat.thoughtCompleted")).toBe(2);
  expect(countText("chat.runCompleted")).toBe(2);
  // Open children: still just labels — the detail waits for its own tap.
  expect(hasText("why it broke")).toBe(false);
  expect(hasText("ls -la")).toBe(false);
  act(() => rowButton("chat.thoughtCompleted").props.onPress());
  expect(hasText("why it broke")).toBe(true);
  act(() => rowButton("chat.runCompleted").props.onPress());
  expect(hasText("ls -la")).toBe(true);
});

// A failed call folds like any other — and counts like any other, under its own
// kind — so the run must still say that something in it failed.
test("a failed call folds into the run, counts under its kind, and is flagged", () => {
  render(reply({
    segments: [thinking("k1"), tool("c1", { status: "failed" }), tool("c2"), tool("c3")],
  }));
  // Three shell calls ran, one of them badly: folded into one line, counted 3.
  expect(countText(stepsSummary(["Think", 1], ["Run", 3]))).toBe(1);
  expect(hasText("chat.runFailed")).toBe(false);
  // Flagged, not silent: the alert glyph and the count a screen reader reads.
  expect(countGlyph("TriangleAlert")).toBe(1);
  expect(countLabel(`${stepsSummary(["Think", 1], ["Run", 3])} · chat.stepsFailed×1`)).toBe(1);
  // The failed row keeps its own danger label once the run is open.
  act(() => rowButton(stepsSummary(["Think", 1], ["Run", 3])).props.onPress());
  expect(hasText("chat.runFailed")).toBe(true);
});

test("a run with no failure carries no alert", () => {
  render(reply({ segments: [thinking("k1"), tool("c1")] }));
  expect(countGlyph("TriangleAlert")).toBe(0);
});

// The summary counts each kind under a short verb and merges the kinds into one
// line — a stack of "已运行 5 次 · 已思考 3 次" sentences is what made it unreadable.
test("every kind in the run is merged into one line of short counts", () => {
  render(reply({
    segments: [
      thinking("k1"), tool("c1"), tool("c2", { name: "read" }),
      tool("c3", { name: "write" }), tool("c4", { name: "edit" }),
    ],
  }));
  expect(countText(stepsSummary(["Think", 1], ["Run", 1], ["Read", 1], ["Write", 1], ["Edit", 1]))).toBe(1);
});

// The tail of an in-flight reply is the slice being written right now: folding it
// would hide live progress behind the summary.
test("the slice a streaming reply is still on stays visible", () => {
  render(reply({ streaming: true, segments: [thinking("k1"), tool("c1"), thinking("k2")] }));
  expect(countText(THINK_RUN)).toBe(1);
  expect(hasText("chat.thinking")).toBe(true);
  expect(hasText("chat.thoughtCompleted")).toBe(false);
});

test("a mid-run history preview has a generating footer even before the start timestamp arrives", () => {
  render(reply({ streaming: true }));
  expect(hasText("chat.generating")).toBe(true);
  expect(tree.root.findAll(node => node.props.accessibilityLabel === "chat.copyResponse")).toHaveLength(0);
});

test("terminal update replaces the timer with copy and authoritative duration", () => {
  render(reply({ streaming: true, startedAt: Date.now() - 1000 }));
  act(() => tree.update(createElement(TimelineCard, { item: reply({ streaming: false, durationMs: 5100 }) })));
  expect(hasText("5s")).toBe(true);
  expect(tree.root.findAll(node => node.props.accessibilityLabel === "chat.copyResponse").length).toBeGreaterThan(0);
});

test("legacy history without timing metadata shows completed instead of a blank footer", () => {
  render(reply({}));
  expect(hasText("chat.responseCompleted")).toBe(true);
});
