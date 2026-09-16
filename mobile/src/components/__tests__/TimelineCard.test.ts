import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { TimelineCard } from "../TimelineCard";
import type { TimelineItem, TimelineSegment, TimelineToolRow } from "../../remote/types";

jest.mock("../MarkdownText", () => ({ MarkdownText: "MarkdownText" }));
jest.mock("lucide-react-native", () => Object.fromEntries([
  "AlertTriangle", "Brain", "Check", "ChevronDown", "ChevronUp", "CircleAlert", "Copy", "FileText", "Paperclip", "Pencil", "TerminalSquare", "TriangleAlert", "X",
].map(name => [name, name])));
// Interpolate the two options the folded step-run summary passes (count/action)
// so its aggregated line is assertable verbatim; every other key stays bare.
jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: Record<string, unknown>) =>
      options ? `${key}(${options.action ?? ""})×${options.count ?? ""}` : key,
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

const prose = (id: string, text = "answer"): TimelineSegment => ({ id, kind: "text", text });
const thinking = (id: string, text = id): TimelineSegment => ({ id, kind: "thinking", text });
const tool = (id: string, fields: Partial<TimelineToolRow> = {}): TimelineSegment => ({
  id,
  kind: "tool",
  tool: { name: "shell", complete: true, status: "completed", detail: `cmd ${id}`, ...fields },
});

const STEPS_SUMMARY =
  "chat.runCount(chat.thoughtCompleted)×1 · chat.runCount(chat.runCompleted)×1";

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
  expect(countText(STEPS_SUMMARY)).toBe(1);
  expect(countText("chat.runCount(chat.thoughtCompleted)×2 · chat.runCount(chat.runCompleted)×2")).toBe(1);
  expect(hasText("chat.thoughtCompleted")).toBe(false);
  expect(hasText("chat.runCompleted")).toBe(false);
});

test("the folded run opens into its rows, each still opening its own detail", () => {
  render(reply({
    segments: [thinking("k1", "why it broke"), tool("c1", { detail: "ls -la" }), thinking("k2"), tool("c2")],
  }));
  act(() => rowButton("chat.runCount(chat.thoughtCompleted)×2 · chat.runCount(chat.runCompleted)×2").props.onPress());
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

// A failed call is the one row the user must not have to dig for.
test("a failed call stays on screen instead of folding away", () => {
  render(reply({
    segments: [thinking("k1"), tool("c1", { status: "failed" }), tool("c2")],
  }));
  expect(hasText("chat.runFailed")).toBe(true);
  expect(countText(STEPS_SUMMARY)).toBe(0);
});

// The tail of an in-flight reply is the slice being written right now: folding it
// would hide live progress behind the summary.
test("the slice a streaming reply is still on stays visible", () => {
  render(reply({ streaming: true, segments: [thinking("k1"), tool("c1"), thinking("k2")] }));
  expect(countText(STEPS_SUMMARY)).toBe(1);
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
