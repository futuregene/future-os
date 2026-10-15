import { createElement } from "react";
import { AppState, StyleSheet, type AppStateStatus } from "react-native";
import { layout } from "../../theme/tokens";
import * as Clipboard from "expo-clipboard";
import { act, create, type ReactTestInstance, type ReactTestRenderer } from "react-test-renderer";
import { PausedTimeline } from "../PausedTimeline";
import { TimelineCard } from "../TimelineCard";
import { useStyles } from "../../features/chat/messages/messageStyles";
import { lightTheme } from "../../theme/theme";
import type { HistoryAttachment, TimelineItem, TimelineSegment } from "../../remote/types";
import i18n from "../../i18n";

jest.mock("../MarkdownText", () => ({ MarkdownText: "MarkdownText" }));
/** The sheet the rendered components use, in the default (light) theme. */
const messageStyles = useStyles.forTheme(lightTheme);
jest.mock("lucide-react-native", () => Object.fromEntries(
  ["AlertTriangle", "Brain", "Check", "ChevronDown", "ChevronUp", "CircleAlert", "Copy",
    "FileText", "GitBranch", "Paperclip", "Pencil", "TerminalSquare", "TriangleAlert", "Wrench", "X"]
    .map(name => [name, name]),
));
jest.mock("../appAlerts", () => ({ AppAlert: { alert: jest.fn() } }));
jest.mock("expo-clipboard", () => ({ setStringAsync: jest.fn(async () => true) }));

let tree: ReactTestRenderer;
afterEach(() => {
  if (tree) act(() => tree.unmount());
  jest.useRealTimers();
  jest.restoreAllMocks();
  // Module-level jest.fn()s (clipboard, alerts, AppState) keep their call
  // history across tests unless it is cleared here.
  jest.clearAllMocks();
});

function render(item: TimelineItem, props: Partial<Parameters<typeof TimelineCard>[0]> = {}) {
  act(() => { tree = create(createElement(TimelineCard, { item, ...props })); });
}
/** Render an arbitrary element (a provider subtree) as the tree under test. */
function renderElement(element: Parameters<typeof create>[0]) {
  act(() => { tree = create(element); });
}
function hasText(text: string) {
  return tree.root.findAll(
    node => typeof node.type === "string" && node.props.children === text,
  ).length > 0;
}
/** The innermost tappable node whose subtree paints `text`. */
function press(text: string) {
  const node = tree.root.findAll(candidate =>
    typeof candidate.props.onPress === "function"
    && candidate.findAll(inner => inner.props.children === text).length > 0).at(-1);
  expect(node).toBeDefined();
  act(() => node!.props.onPress());
}
/** Host nodes rendered for a mocked lucide glyph name. */
function glyphs(name: string) {
  return tree.root.findAll(node => node.type === name);
}
/** The tappable node carrying a screen-reader label (its glyphs paint no text). */
function pressLabelled(label: string) {
  const node = tree.root.findAll(candidate =>
    typeof candidate.props.onPress === "function"
    && candidate.props.accessibilityLabel === label).at(-1);
  expect(node).toBeDefined();
  act(() => node!.props.onPress());
}
const message = (fields: Partial<Extract<TimelineItem, { kind: "message" }>>): TimelineItem => ({
  id: "m", kind: "message", role: "assistant", text: "reply", ...fields,
});
const attachment = (fields: Partial<HistoryAttachment> & { name: string }): HistoryAttachment =>
  ({ path: `/files/${fields.name}`, ...fields });

describe("a user message is the user's own markdown, plus their attachments", () => {
  /** The markdown rows the row rendered (a host stub in this suite). */
  function markdownNodes() {
    return tree.root.findAll(node => String(node.type) === "MarkdownText");
  }

  test("the prompt is handed to the markdown renderer on the bubble's own tone", () => {
    // A prompt renders through the same component as a reply — its own markdown
    // included — and the tone is what tells that component it is painting the
    // user's accent-filled bubble rather than the app surface. Links, file
    // mentions and `#` references are that renderer's business.
    const onOpenFile = jest.fn();
    const onOpenSession = jest.fn();
    render(message({ role: "user", text: "see [poem.txt](./poem.txt) and **loud**" }), { onOpenFile, onOpenSession });

    const rendered = markdownNodes();
    expect(rendered).toHaveLength(1);
    expect(rendered[0]!.props).toMatchObject({
      onOpenFile,
      onOpenSession,
      text: "see [poem.txt](./poem.txt) and **loud**",
      tone: "onAccent",
    });
  });

  test("a reply renders the same component without a tone of its own", () => {
    // boundary: the other side of the tone — the app's own surface — plus the
    // streaming flag only a reply can carry.
    render(message({ id: "a1", streaming: true, text: "half an answer" }));
    const rendered = markdownNodes();
    expect(rendered).toHaveLength(1);
    expect(rendered[0]!.props).toMatchObject({ streaming: true });
    expect(rendered[0]!.props.tone).toBeUndefined();
  });

  test("each attachment is a chip that opens its own file, and an unknown kind still renders", () => {
    const onOpenAttachment = jest.fn();
    const attachments = [
      attachment({ name: "报告最终版.docx", path: "/files/报告最终版.docx", kind: "file" }),
      attachment({ name: "shot.png", path: "/files/shot.png", kind: "image" }),
      attachment({ name: "legacy.bin" }),
    ];
    render(message({ role: "user", text: "here", attachments }), { onOpenAttachment });
    expect(tree.root.findAll(node => node.props.accessibilityLabel === "报告最终版.docx")).not.toHaveLength(0);
    press("报告最终版.docx");
    expect(onOpenAttachment).toHaveBeenCalledWith(attachments[0]);
    press("shot.png");
    expect(onOpenAttachment).toHaveBeenCalledWith(attachments[1]);
    expect(hasText("legacy.bin")).toBe(true);
  });

  test("attachments without an opener are disabled chips", () => {
    render(message({ role: "user", text: "here", attachments: [attachment({ name: "a.txt", kind: "file" })] }));
    const chip = tree.root.findAll(node => node.props.accessibilityLabel === "a.txt")[0]!;
    expect(chip.props.disabled).toBe(true);
  });

  test("a message with only attachments shows no empty bubble, and an empty message draws nothing", () => {
    render(message({ role: "user", text: "   ", attachments: [attachment({ name: "a.txt", kind: "file" })] }));
    expect(hasText("a.txt")).toBe(true);
    expect(tree.root.findAll(node => node.props.accessibilityLabel === "Copy response")).toHaveLength(0);
    act(() => tree.unmount());
    render(message({ role: "user", id: "empty", text: "" }));
    expect(tree.root.findAll(node =>
      typeof node.type === "string" && node.props.children === "")).toHaveLength(0);
  });
});

describe("the settled footer", () => {
  test("a long run reads in minutes and seconds beside its token count", () => {
    render(message({ durationMs: 85_000, outputTokens: 1_234 }));
    expect(hasText("1m 25s · 1,234 tokens")).toBe(true);
  });

  test("a run of exactly a minute keeps the zero seconds", () => {
    render(message({ durationMs: 60_000 }));
    expect(hasText("1m 0s")).toBe(true);
  });

  test("a clock skew that reports a negative duration cannot render a negative time", () => {
    render(message({ durationMs: -5_000 }));
    expect(hasText("0s")).toBe(true);
  });

  test("zero tokens drop the count instead of printing a zero", () => {
    render(message({ durationMs: 2_000, outputTokens: 0 }));
    expect(hasText("2s")).toBe(true);
  });

  test.each([
    [{ stopped: true }, "You stopped this response"],
    [{ failed: true }, "Something went wrong."],
    [{ truncated: true }, "Something went wrong."],
    [{}, "Completed"],
  ])("a settled reply with %o reports its own ending", (fields, expected) => {
    render(message({ ...fields }));
    expect(hasText(expected)).toBe(true);
  });

  test("copying puts the reply on the clipboard and confirms it, then settles back", async () => {
    jest.useFakeTimers();
    const setStringAsync = Clipboard.setStringAsync as jest.Mock;
    render(message({ text: "the answer" }));
    expect(glyphs("Copy")).not.toHaveLength(0);
    pressLabelled("Copy response");
    await act(async () => { await Promise.resolve(); });
    expect(setStringAsync).toHaveBeenCalledWith("the answer");
    expect(glyphs("Check")).not.toHaveLength(0);
    act(() => { jest.advanceTimersByTime(1_400); });
    expect(glyphs("Check")).toHaveLength(0);
    expect(glyphs("Copy")).not.toHaveLength(0);
  });

  test("a clipboard the OS refuses must not claim the reply was copied", async () => {
    const setStringAsync = Clipboard.setStringAsync as jest.Mock;
    setStringAsync.mockRejectedValueOnce(new Error("clipboard unavailable"));
    render(message({ text: "the answer" }));
    pressLabelled("Copy response");
    await act(async () => { await Promise.resolve(); });
    expect(setStringAsync).toHaveBeenCalledTimes(1);
    expect(glyphs("Check")).toHaveLength(0);
    expect(glyphs("Copy")).not.toHaveLength(0);
  });

  test("a reply with no text to copy offers no copy button", () => {
    render(message({ text: "   ", durationMs: 1_000 }));
    expect(tree.root.findAll(node => node.props.accessibilityLabel === "Copy response")).toHaveLength(0);
    expect(hasText("1s")).toBe(true);
  });

  test("a failed latest run offers retry and continue, and hands back its own item", () => {
    const onRetry = jest.fn();
    const onContinue = jest.fn();
    const item = message({ id: "latest", text: "half an answer", failed: true, runId: "run-1" });
    render(item, { isLatestAssistant: true, onContinue, onRetry });
    press("Retry");
    expect(onRetry).toHaveBeenCalledWith(item);
    press("Continue");
    expect(onContinue).toHaveBeenCalledWith(item);
  });

  test("a failed run the user stopped is not offered as recoverable", () => {
    render(message({ id: "latest", failed: true, stopped: true, runId: "run-1" }), { isLatestAssistant: true });
    expect(hasText("Retry")).toBe(false);
  });

  test("a settled reply offers Fork and hands back its own item", () => {
    const onFork = jest.fn();
    const item = message({ id: "a-fork", text: "an answer" });
    render(item, { onFork });
    expect(glyphs("GitBranch")).not.toHaveLength(0);
    pressLabelled("Fork");
    expect(onFork).toHaveBeenCalledWith(item);
  });

  test("a host without fork support offers no Fork, and a streaming reply hides it", () => {
    render(message({ id: "a", text: "an answer" }));
    expect(glyphs("GitBranch")).toHaveLength(0);

    act(() => tree.unmount());
    render(message({ id: "a", text: "an answer", streaming: true }), { onFork: jest.fn() });
    expect(glyphs("GitBranch")).toHaveLength(0);
  });

  // Copy and Fork are one group of small glyphs: each hugs its own icon instead
  // of sitting at the left edge of a full-width box (which left them 43pt apart,
  // wider than the gap to the stats text), and `hitSlop` keeps the tap area.
  test("the footer's actions sit together and still take a 44pt tap", () => {
    render(message({ id: "a-spacing", text: "an answer" }), { onFork: jest.fn() });
    for (const label of ["Copy response", "Fork"]) {
      const control = tree.root.findAll(node =>
        node.props.accessibilityLabel === label && typeof node.props.onPress === "function")[0]!;
      expect(StyleSheet.flatten(control.props.style))
        .toMatchObject({ minWidth: 28, minHeight: layout.touchTarget, justifyContent: "center" });
      // The box hugs the glyph; this much `hitSlop` per side restores 44pt.
      expect(control.props.hitSlop).toBe((layout.touchTarget - 28) / 2);
    }
  });
});

describe("the live elapsed timer", () => {
  beforeEach(() => {
    (AppState.addEventListener as jest.Mock).mockReturnValue({ remove: jest.fn() });
    // A foregrounded phone: the timer only runs while the app is active.
    Object.defineProperty(AppState, "currentState", {
      configurable: true, value: "active", writable: true,
    });
  });

  test("a run older than a minute ticks once a second", () => {
    jest.useFakeTimers();
    const startedAt = Date.now() - 85_000;
    render(message({ streaming: true, startedAt }));
    expect(hasText("1m 25s")).toBe(true);
    act(() => { jest.advanceTimersByTime(1_000); });
    expect(hasText("1m 26s")).toBe(true);
    expect(hasText("1m 25s")).toBe(false);
  });

  test("a run with no start anchor says it is generating rather than counting", () => {
    render(message({ streaming: true }));
    expect(hasText("Generating…")).toBe(true);
  });

  test("a transcript covered by a file surface stops ticking", () => {
    jest.useFakeTimers();
    renderElement(createElement(
      PausedTimeline, { paused: true },
      createElement(TimelineCard, {
        item: message({ streaming: true, startedAt: Date.now() - 1_000 }),
      }),
    ));
    expect(hasText("1s")).toBe(true);
    act(() => { jest.advanceTimersByTime(3_000); });
    // Covered: the label is still the first frame, because no interval was armed.
    expect(hasText("1s")).toBe(true);
    expect(hasText("4s")).toBe(false);
  });

  test("a backgrounded app stops the timer instead of waking every second", () => {
    jest.useFakeTimers();
    const startedAt = Date.now() - 1_000;
    render(message({ streaming: true, startedAt }));
    expect(hasText("1s")).toBe(true);
    const onChange = (AppState.addEventListener as jest.Mock).mock.calls[0]![1] as
      (state: AppStateStatus) => void;
    act(() => onChange("background"));
    act(() => { jest.advanceTimersByTime(3_000); });
    expect(hasText("1s")).toBe(true);
    act(() => onChange("active"));
    act(() => { jest.advanceTimersByTime(1_000); });
    expect(hasText("5s")).toBe(true);
  });

  test("the visible context reaches a nested transcript", () => {
    jest.useFakeTimers();
    renderElement(createElement(
      PausedTimeline, { paused: false },
      createElement(TimelineCard, {
        item: message({ streaming: true, startedAt: Date.now() - 1_000 }),
      }),
    ));
    act(() => { jest.advanceTimersByTime(1_000); });
    // Uncovered (unpaused) transcript keeps counting.
    expect(hasText("2s")).toBe(true);
  });
});

describe("notices", () => {
  test("a warning shows the agent's own words", () => {
    render({ id: "w", kind: "notice", tone: "warning", text: "Workspace is read-only" });
    expect(hasText("Workspace is read-only")).toBe(true);
  });

  test("a danger notice classifies the raw error instead of dumping it at the user", () => {
    render({ id: "d", kind: "notice", tone: "danger", text: "[CTX_LIMIT] context too large" });
    expect(hasText(i18n.t("failure.contextLimit"))).toBe(true);
    expect(hasText("[CTX_LIMIT] context too large")).toBe(false);
  });

  test("the truncated marker reads as truncation, not as an error blob", () => {
    render({ id: "t", kind: "notice", tone: "danger", text: "truncated" });
    expect(hasText(i18n.t("chat.truncated"))).toBe(true);
  });

  test("an approval item draws nothing inline — the dock owns approval", () => {
    render({
      id: "appr-1",
      kind: "approval",
      payload: { approval_request_id: "req-1", kind: "file_write" },
    });
    expect(tree.toJSON()).toBeNull();
  });
});

describe("tool rows name the failure that happened", () => {
  const row = (name: string, status: "failed" | "completed"): TimelineSegment => ({
    id: `${name}-1`, kind: "tool",
    tool: { name, complete: status === "completed", status, detail: `target-${name}` },
  });

  test.each([
    ["read", "Read failed"],
    ["write", "Write failed"],
    ["edit", "Edit failed"],
    ["shell", "Command failed"],
  ])("a failed %s says so instead of the neutral completed label", (name, expected) => {
    render(message({ id: `fail-${name}`, segments: [row(name, "failed")] }));
    expect(hasText(expected)).toBe(true);
  });

  test("an unknown tool name falls back to the shell treatment", () => {
    render(message({ id: "unknown-tool", segments: [row("teleport", "completed")] }));
    expect(hasText("Ran a command")).toBe(true);
  });
});

describe("a reply's tail anchor is measured inside its own row", () => {
  const prose = (id: string, text: string): TimelineSegment => ({ id, kind: "text", text });
  const reasoning = (id: string): TimelineSegment => ({ id, kind: "thinking", text: "..." });

  /** The reply's block list: the host view laid out with the list's own style. */
  function blockList(): ReactTestInstance | undefined {
    return tree.root.findAll(
      node => typeof node.type === "string" && node.props.style === messageStyles.segmentList,
    )[0];
  }
  /**
   * The wrapper the bubble measures the anchor through: the block list's only
   * child carrying a layout handler and no style of its own, because the anchor
   * block is wrapped and the blocks around it are not.
   */
  function anchorWrapper(list: ReactTestInstance): ReactTestInstance | undefined {
    return list.children.filter(
      (child): child is ReactTestInstance =>
        typeof child !== "string"
        && typeof child.props.onLayout === "function"
        && child.props.style === undefined,
    )[0];
  }

  test("the block list and the anchor block chain into the row's own offset", () => {
    const onReplyTailAnchor = jest.fn();
    render(message({
      id: "a1",
      segments: [reasoning("h1"), prose("t1", "first"), prose("t2", "the answer")],
    }), { onReplyTailAnchor });

    const list = blockList()!;
    const anchor = anchorWrapper(list)!;
    // The last output block is the one measured, so the wrapper holds the last
    // piece of prose and the earlier one is left alone.
    expect(
      anchor.findAll(node => String(node.type) === "MarkdownText").map(node => node.props.text),
    ).toEqual(["the answer"]);

    act(() => list.props.onLayout({ nativeEvent: { layout: { y: 4 } } }));
    // Half of a position is not a position: nothing is reported until the
    // anchor itself has been laid out too.
    expect(onReplyTailAnchor).not.toHaveBeenCalled();

    act(() => anchor.props.onLayout({ nativeEvent: { layout: { y: 260 } } }));
    expect(onReplyTailAnchor).toHaveBeenCalledWith("a1", 264);
  });

  test("a reply with no output block reports nothing to aim at", () => {
    const onReplyTailAnchor = jest.fn();
    render(message({ id: "a2", segments: [reasoning("h1")] }), { onReplyTailAnchor });

    const list = blockList()!;
    act(() => list.props.onLayout({ nativeEvent: { layout: { y: 4 } } }));

    expect(anchorWrapper(list)).toBeUndefined();
    expect(onReplyTailAnchor).not.toHaveBeenCalled();
  });

  test("a reply with no segments at all has no block list to measure", () => {
    const onReplyTailAnchor = jest.fn();
    render(message({ id: "a3", text: "the whole reply" }), { onReplyTailAnchor });

    expect(blockList()).toBeUndefined();
    expect(onReplyTailAnchor).not.toHaveBeenCalled();
  });
});
