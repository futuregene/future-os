import { createElement } from "react";
import { AppState, StyleSheet, type AppStateStatus } from "react-native";
import { colors } from "../../theme/tokens";
import { act, create, type ReactTestInstance, type ReactTestRenderer } from "react-test-renderer";
import { TimelineCard } from "../TimelineCard";
import type { TimelineItem, TimelineSegment, TimelineToolRow } from "../../remote/types";

jest.mock("../MarkdownText", () => ({ MarkdownText: "MarkdownText" }));
jest.mock("lucide-react-native", () => Object.fromEntries([
  "AlertTriangle", "Brain", "Check", "ChevronDown", "ChevronUp", "CircleAlert", "Copy", "FileText", "Paperclip", "Pencil", "TerminalSquare", "TriangleAlert", "Wrench", "X",
].map(name => [name, name])));
// The mock mirrors the real templates for the keys the folded summary uses, and
// returns every other key bare — so the summary's rendered shape is assertable
// (a glyph and "×N") without tying the whole suite to the copy deck.
jest.mock("react-i18next", () => {
  const templates: Record<string, string> = {
    "chat.stepCount": "×{{count}}",
    "chat.stepSummary": "{{action}} {{count}}×",
    "chat.stepTool": "Tool calls",
    "chat.stepThink": "Thought",
    "chat.stepsFailed": "{{count}} failed",
    // The compaction divider's copy, so the rendered label is assertable instead
    // of being the bare i18n key.
    "chat.compacted": "Context compacted",
    "chat.compactedTokens": "Context compacted · {{formattedCount}} tokens",
    "chat.compactedTokensDelta": "Context compacted · {{before}} → {{after}} tokens (estimated)",
    "chat.manuallyCompactedTokensDelta": "Manually compacted · {{before}} → {{after}} tokens (estimated)",
  };
  return {
    useTranslation: () => ({
      t: (key: string, options?: Record<string, unknown>) => {
        const template = templates[key];
        if (!template) return key;
        return template.replace(/\{\{(\w+)\}\}/g, (_match, name: string) => String(options?.[name] ?? ""));
      },
      i18n: { language: "en" },
    }),
  };
});

let tree: ReactTestRenderer;
beforeEach(() => {
  (AppState.addEventListener as jest.Mock).mockReturnValue({ remove: jest.fn() });
});
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

/**
 * The folded summary row, found by the label a screen reader reads — the counts
 * are the row's whole semantics, and the label spells out what the glyphs mean.
 * Tool calls come first, then reasoning (the fixed order), zeros dropped.
 */
function summaryRow(counts: { tools?: number; thinking?: number; failed?: number }) {
  const label = [
    counts.tools ? `Tool calls ${counts.tools}×` : null,
    counts.thinking ? `Thought ${counts.thinking}×` : null,
    counts.failed ? `${counts.failed} failed` : null,
  ].filter(Boolean).join(" · ");
  return tree.root.findAll(node =>
    typeof node.props.onPress === "function" && node.props.accessibilityLabel === label)[0]!;
}

/** Every string the given row actually paints (its glyphs carry no text). */
function paintedStrings(host: ReactTestInstance) {
  return host.findAll(node => typeof node.type === "string" && typeof node.props.children === "string")
    .map(node => node.props.children as string);
}
/** How many 14px glyphs a row renders: its own icon plus any chevron/disclosure. */
function countIconsIn(host: ReactTestInstance) {
  return host.findAll(node => node.props.size === 14 && typeof node.props.color === "string").length;
}
/** Count a specific glyph in a row (the lucide mock renders each name verbatim). */
function countIconsNamed(host: ReactTestInstance, name: string) {
  return host.findAll(node => node.type === name).length;
}

/**
 * The rail a row sits on: walk up from its label to the nearest container that
 * declares an `alignSelf` other than `stretch` — the default every layer inherits,
 * which says nothing about which side the row is on. The first real value is the
 * row's own block, and asserting on it does not depend on how many composite
 * layers the row happens to render through.
 */
function railOf(host: ReactTestInstance): unknown {
  for (let node: ReactTestInstance | null = host; node; node = node.parent) {
    const alignSelf = (StyleSheet.flatten(node.props.style) as { alignSelf?: string } | undefined)?.alignSelf;
    if (alignSelf && alignSelf !== "stretch") return alignSelf;
  }
  return undefined;
}

const THINK_RUN = { tools: 1, thinking: 1 };

// Consecutive thinking/tool rows are one line on a phone (the desktop transcript
// spends a full line per activity); the individual rows stay one tap away, and
// each of those still opens its own detail.
test("a run of thinking and tool rows folds into one icon-count summary line", () => {
  render(reply({
    segments: [
      prose("p1"),
      thinking("k1"), tool("c1"), thinking("k2"), tool("c2"),
      prose("p2"),
      thinking("k3"), tool("c3"),
    ],
  }));
  // Prose splits the reply into one folded run per stretch of activity.
  expect(summaryRow(THINK_RUN)).toBeTruthy();
  expect(summaryRow({ tools: 2, thinking: 2 })).toBeTruthy();
  // The wording is gone from the line — it paints a count and nothing else.
  expect(paintedStrings(summaryRow(THINK_RUN))).toEqual(["×1", "·", "×1"]);
  expect(hasText("chat.thoughtCompleted")).toBe(false);
  expect(hasText("chat.runCompleted")).toBe(false);
});

// Nothing is lost to a screen reader: the row still says what the glyphs mean,
// including the count of anything that failed.
test("the icon summary keeps a spoken form", () => {
  render(reply({ segments: [thinking("k1"), tool("c1"), tool("c2", { status: "failed" })] }));
  const row = summaryRow({ tools: 2, thinking: 1, failed: 1 });
  expect(row.props.accessibilityLabel).toBe("Tool calls 2× · Thought 1× · 1 failed");
});

test("the folded run opens into its rows, each still opening its own detail", () => {
  render(reply({
    segments: [thinking("k1", "why it broke"), tool("c1", { detail: "ls -la" }), thinking("k2"), tool("c2")],
  }));
  act(() => summaryRow({ tools: 2, thinking: 2 }).props.onPress());
  // Expanded rows keep the words (unlike the folded line) and a glyph each.
  expect(countText("chat.thoughtCompleted")).toBe(2);
  expect(countText("chat.runCompleted")).toBe(2);
  expect(countIconsIn(rowButton("chat.thoughtCompleted"))).toBe(2);
  expect(countIconsIn(rowButton("chat.runCompleted"))).toBe(2);
  // Open children: still just labels — the detail waits for its own tap.
  expect(hasText("why it broke")).toBe(false);
  expect(hasText("ls -la")).toBe(false);
  act(() => rowButton("chat.thoughtCompleted").props.onPress());
  expect(hasText("why it broke")).toBe(true);
  act(() => rowButton("chat.runCompleted").props.onPress());
  expect(hasText("ls -la")).toBe(true);
});

// The tool count is a wrench, not the terminal it started as: the count covers
// every tool kind (run/read/write/edit), and a shell prompt claimed they were all
// commands.
test("the tool-call count carries a generic tool glyph", () => {
  render(reply({ segments: [tool("c1", { name: "read" }), tool("c2", { name: "edit" })] }));
  expect(summaryRow({ tools: 2 })).toBeTruthy();
  expect(countIconsNamed(summaryRow({ tools: 2 }), "Wrench")).toBe(1);
  expect(countIconsNamed(summaryRow({ tools: 2 }), "TerminalSquare")).toBe(0);
});

// Every tool kind is one "tool call" in the summary: shell/read/write/edit split
// into four counts read as a list of numbers nobody parses (the feedback that
// prompted this: "已运行5次已思考三次，已写入一次，这种文案太啰嗦，合并一下").
test("every tool kind merges into a single tool-call count", () => {
  render(reply({
    segments: [
      thinking("k1"), tool("c1"), tool("c2", { name: "read" }),
      tool("c3", { name: "write" }), tool("c4", { name: "edit" }),
    ],
  }));
  expect(summaryRow({ tools: 4, thinking: 1 })).toBeTruthy();
});

test("a run of tool calls alone omits the zero thinking count", () => {
  render(reply({ segments: [tool("c1"), tool("c2")] }));
  expect(summaryRow({ tools: 2 })).toBeTruthy();
  expect(paintedStrings(summaryRow({ tools: 2 }))).toEqual(["×2"]);
});

test("a run of reasoning alone omits the zero tool count", () => {
  render(reply({ segments: [thinking("k1"), thinking("k2")] }));
  expect(summaryRow({ thinking: 2 })).toBeTruthy();
  expect(paintedStrings(summaryRow({ thinking: 2 }))).toEqual(["×2"]);
});

// The glyph sits flush against its own count (no gap), while the kinds stay
// spaced apart by the row's own gap — tight enough that the `·` is a separator
// rather than a hole in the line.
test("each glyph sits tight against its count", () => {
  render(reply({ segments: [thinking("k1"), tool("c1")] }));
  const row = summaryRow(THINK_RUN);
  expect(StyleSheet.flatten(row.props.style).gap).toBe(4);
  const groups = row.findAll(node =>
    typeof node.type !== "string" && StyleSheet.flatten(node.props.style)?.gap === 0);
  // One per kind (the tool group and the reasoning group).
  expect(groups).toHaveLength(2);
});

// A failed call folds like any other — and counts like any other, as a tool call
// — so the run must still say that something in it failed.
test("a failed call folds into the run, counts as a tool call, and is flagged", () => {
  render(reply({
    segments: [thinking("k1"), tool("c1", { status: "failed" }), tool("c2"), tool("c3")],
  }));
  // Three tool calls ran, one of them badly: folded into one line, counted 3.
  expect(summaryRow({ tools: 3, thinking: 1, failed: 1 })).toBeTruthy();
  expect(hasText("chat.runFailed")).toBe(false);
  // Flagged, not silent: the alert glyph rides along with the counts.
  expect(countIconsIn(summaryRow({ tools: 3, thinking: 1, failed: 1 }))).toBe(4);
  // The failed row keeps its own label once the run is open.
  act(() => summaryRow({ tools: 3, thinking: 1, failed: 1 }).props.onPress());
  expect(hasText("chat.runFailed")).toBe(true);
});

// A failure is marked by the alert glyph's shape alone — colour would shout
// inside the very line that was folded to quieten the transcript down.
test("a failed run is not tinted", () => {
  render(reply({ segments: [thinking("k1"), tool("c1", { status: "failed" }), tool("c2")] }));
  const counts = tree.root.findAll(node => node.props.children === "×2" || node.props.children === "×1");
  expect(counts).not.toHaveLength(0);
  for (const text of counts)
    expect(StyleSheet.flatten(text.props.style).color).toBe(colors.inkMuted);
});

// The folded line is apparatus, not prose: it sits against the right edge so the
// reader's eye line stays with the reply text, and its tap target stays clipped
// to the glyphs rather than spanning the bubble.
test("the folded summary hugs the right edge", () => {
  render(reply({ segments: [thinking("k1"), tool("c1")] }));
  const row = summaryRow(THINK_RUN);
  const style = StyleSheet.flatten(row.props.style);
  expect(style.alignSelf).toBe("flex-end");
  expect(style.justifyContent).toBe("flex-end");
});

// A step row that is NOT folded — a lone call, a lone burst, the slice still
// running — must sit on the same rail as a folded run. It did not, while the
// rail was a per-call-site `align` prop with a left-aligning default: the reply
// then alternated left/right down one screen, and a streaming row jumped sides
// the moment it got folded. The prop is gone so this cannot come back.
test("an unfolded step row sits on the same right rail as a folded run", () => {
  // One lone tool call before the prose, one lone reasoning slice after it:
  // neither reaches the two-slice minimum for folding.
  render(reply({
    segments: [tool("c1", { detail: "ls -la" }), prose("p1"), thinking("k1", "why")],
  }));
  for (const label of ["chat.runCompleted", "chat.thoughtCompleted"])
    expect(railOf(rowButton(label))).toBe("flex-end");
});

// A step row is the only way into the call behind it, so it carries a real touch
// target even though it draws as a single 20px line.
test("every step row extends its touch area beyond its drawn line", () => {
  render(reply({ segments: [tool("c1"), tool("c2"), prose("p1"), thinking("k1", "why")] }));
  expect(summaryRow({ tools: 2 }).props.hitSlop).toEqual({ top: 8, bottom: 8 });
  // Expanded rows and the lone reasoning slice carry it too.
  act(() => summaryRow({ tools: 2 }).props.onPress());
  expect(rowButton("chat.runCompleted").props.hitSlop).toEqual({ top: 8, bottom: 8 });
  expect(rowButton("chat.thoughtCompleted").props.hitSlop).toEqual({ top: 8, bottom: 8 });
});

// The live timer is part of the same apparatus and follows it onto the right
// rail, instead of hanging off the left edge of the reply it belongs to.
test("the live run indicator sits on the right rail", () => {
  render(reply({ streaming: true, startedAt: Date.now() - 1000 }));
  const indicator = tree.root.findAll(node => node.props.children === "1s"
    || node.props.children === "chat.generating")[0]!;
  const style = StyleSheet.flatten(indicator.parent!.props.style);
  expect(style.alignSelf).toBe("flex-end");
});

// The footer is the settled form of that same timer — duration and tokens, with
// the copy button beside them — so it lands on the same rail. It did not, so a
// reply jumped from the right edge to the left the instant its run finished.
test("the settled footer sits on the same rail as the live timer", () => {
  render(reply({ durationMs: 5100, outputTokens: 1234 }));
  const footer = tree.root.findAll(node => node.props.children === "5s · chat.tokens")[0]!;
  const style = StyleSheet.flatten(footer.parent!.props.style);
  expect(style.alignSelf).toBe("flex-end");
  expect(style.justifyContent).toBe("flex-end");
  // The copy button rides along in the same group, so its target stays clipped.
  const copy = tree.root.findAll(node => node.props.accessibilityLabel === "chat.copyResponse")[0]!;
  expect(copy).toBeDefined();
});

// The summary is the run's badge: tapping it must not move the line under the
// user's finger. The rows it opens are read, so they land in the reading column.
test("opening a run leaves the summary on the rail and reads its rows on the left", () => {
  render(reply({
    segments: [thinking("k1", "why it broke"), tool("c1", { detail: "ls -la" })],
  }));
  expect(StyleSheet.flatten(summaryRow(THINK_RUN).props.style).alignSelf).toBe("flex-end");
  act(() => summaryRow(THINK_RUN).props.onPress());
  // The line the user just tapped is still where it was.
  expect(StyleSheet.flatten(summaryRow(THINK_RUN).props.style).alignSelf).toBe("flex-end");

  // ...and everything it opened is reading content on the left.
  expect(railOf(rowButton("chat.thoughtCompleted"))).toBeUndefined();
  act(() => rowButton("chat.thoughtCompleted").props.onPress());
  const reasoning = tree.root.findAll(host => host.props.children === "why it broke")[0]!;
  expect(railOf(reasoning)).toBeUndefined();
  expect(StyleSheet.flatten(reasoning.props.style).borderLeftWidth).toBe(2);
  // The tool row's own expanded command is left-aligned too.
  act(() => rowButton("chat.runCompleted").props.onPress());
  const command = tree.root.findAll(host => host.props.children === "ls -la")[0]!;
  expect(railOf(command)).toBeUndefined();
});

// A file tool only becomes useful when its target is visible. The target takes
// its own line below the header: inline it claimed the row's leftover width with
// `flex: 1`, which resolves to zero inside the shrink-wrapped rail badge and hid
// the file name entirely.
test.each(["read", "write", "edit"])("opening a %s tool reveals its file name", kind => {
  render(reply({
    segments: [tool("c1", {
      name: kind,
      detail: "/w/agent/src/compaction/issue-audit-2026-09-16-with-a-long-filename.md",
      complete: true,
      status: "completed",
    })],
  }));
  expect(railOf(rowButton(`chat.${kind}Completed`))).toBe("flex-end");
  expect(hasText("issue-audit-2026-09-16-with-a-long-filename.md")).toBe(false);
  act(() => rowButton(`chat.${kind}Completed`).props.onPress());
  const name = "issue-audit-2026-09-16-with-a-long-filename.md";
  expect(hasText(name)).toBe(true);
  // Revealed content is reading content, so the row left the rail to make room.
  expect(railOf(rowButton(`chat.${kind}Completed`))).toBeUndefined();
  const target = tree.root.findAll(host => host.props.children === name)[0]!;
  // The name wraps: nothing clips it to one line of a rail badge.
  expect(target.props.numberOfLines).toBeUndefined();
  expect(StyleSheet.flatten(target.props.style).maxHeight).toBeUndefined();
  expect(StyleSheet.flatten(target.props.style).overflow).not.toBe("hidden");
  // …and it is a line of its own, not a cell squeezed in beside the label: the
  // header the user taps holds the glyph, the words and the chevron, nothing else.
  expect(rowButton(`chat.${kind}Completed`).findAll(inner => inner.props.children === name)).toHaveLength(0);
  expect(StyleSheet.flatten(target.props.style).flex).toBeUndefined();
});

// A burst of one file kind is the same story for every child it lists.
test("opening a file burst lists every child file name", () => {
  render(reply({
    segments: [tool("c1", {
      name: "edit",
      complete: true,
      status: "completed",
      count: 2,
      children: [
        { name: "edit", complete: true, status: "completed", detail: "/w/one.rs" },
        { name: "edit", complete: true, status: "completed", detail: "/w/issue-audit-2026-09-16-with-a-long-filename.md" },
      ],
    })],
  }));
  // The burst row reads "Edited 2×" (short verb + count), not the bare label.
  act(() => rowButton("chat.stepEdit 2×").props.onPress());
  expect(hasText("one.rs")).toBe(true);
  const name = "issue-audit-2026-09-16-with-a-long-filename.md";
  expect(hasText(name)).toBe(true);
  const target = tree.root.findAll(host => host.props.children === name)[0]!;
  expect(target.props.numberOfLines).toBeUndefined();
  expect(target.props.selectable).toBe(true);
});

test("a run with no failure carries no alert", () => {
  render(reply({ segments: [thinking("k1"), tool("c1")] }));
  // Two count glyphs and one chevron — no alert glyph riding along.
  expect(countIconsIn(summaryRow(THINK_RUN))).toBe(3);
});

// The tail of an in-flight reply is the slice being written right now: folding it
// would hide live progress behind the summary.
test("only the live tail subscribes to streaming Markdown presentation", () => {
  render(reply({ streaming: true, segments: [prose("first"), thinking("thought"), prose("tail")] }));
  const markdown = tree.root.findAll(node => (node.type as unknown) === "MarkdownText");
  expect(markdown.map(node => node.props.streaming)).toEqual([false, true]);
});

test("the elapsed-time label ticks once per second and sleeps in background", () => {
  jest.useFakeTimers();
  const originalActivity = Object.getOwnPropertyDescriptor(AppState, "currentState")!;
  Object.defineProperty(AppState, "currentState", { configurable: true, value: "active" });
  let change!: (state: AppStateStatus) => void;
  const remove = jest.fn();
  const listener = jest.spyOn(AppState, "addEventListener").mockImplementation((_type, callback) => {
    change = callback as typeof change;
    return { remove };
  });
  const interval = jest.spyOn(global, "setInterval");
  try {
    render(reply({ streaming: true, startedAt: Date.now() - 1000 }));
    expect(interval).toHaveBeenLastCalledWith(expect.any(Function), 1000);
    act(() => { jest.advanceTimersByTime(0); change("background"); });
    expect(jest.getTimerCount()).toBe(0);
    act(() => change("active"));
    expect(interval).toHaveBeenCalledTimes(2);
    act(() => tree.unmount());
    expect(remove).toHaveBeenCalledTimes(1);
    act(() => jest.advanceTimersByTime(0));
    expect(jest.getTimerCount()).toBe(0);
  } finally {
    listener.mockRestore(); interval.mockRestore();
    Object.defineProperty(AppState, "currentState", originalActivity);
    jest.useRealTimers();
  }
});

test("the slice a streaming reply is still on stays visible", () => {
  render(reply({ streaming: true, segments: [thinking("k1"), tool("c1"), thinking("k2")] }));
  expect(summaryRow(THINK_RUN)).toBeTruthy();
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

const compaction = (fields: Partial<Extract<TimelineSegment, { kind: "compaction" }>> = {}): TimelineSegment => ({
  id: "cp",
  kind: "compaction",
  status: "completed",
  ...fields,
});

test("a committed compaction divider reports both token counts", () => {
  render(reply({
    segments: [compaction({ tokensBefore: 190_000, tokensAfter: 20_000 })],
  }));
  expect(hasText("Context compacted · 190,000 → 20,000 tokens (estimated)")).toBe(true);
});

test("a compaction divider with no post-compaction estimate keeps the count it has", () => {
  // A released run journal's `compaction_end` carries only `tokens_before`.
  render(reply({ segments: [compaction({ tokensBefore: 190_000 })] }));
  expect(hasText("Context compacted · 190,000 tokens")).toBe(true);
});

test("a manual compaction divider reports both counts too", () => {
  render(reply({
    segments: [compaction({ tokensBefore: 33_064, tokensAfter: 9_250, trigger: "manual" })],
  }));
  expect(hasText("Manually compacted · 33,064 → 9,250 tokens (estimated)")).toBe(true);
});
