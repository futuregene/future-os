import { createElement } from "react";
import { AppState, Dimensions, Image, StyleSheet, type AppStateStatus } from "react-native";
import { layout, lightColors } from "../../theme/tokens";
import * as Clipboard from "expo-clipboard";
import { act, create, type ReactTestInstance, type ReactTestRenderer } from "react-test-renderer";
import { PausedTimeline } from "../PausedTimeline";
import { TimelineCard } from "../TimelineCard";
import { useStyles } from "../../features/chat/messages/messageStyles";
import { collapsedPromptHeight, PROMPT_COLLAPSE_LINES } from "../../features/chat/messages/promptCollapse";
import { lightTheme } from "../../theme/theme";
import type { HistoryAttachment, TimelineItem, TimelineSegment } from "../../remote/types";
import i18n from "../../i18n";
import { ImageViewerContext } from "../imageViewer";
import { setAppearance } from "../../theme/preferences";

jest.mock("../MarkdownText", () => ({ MarkdownText: "MarkdownText" }));
// The reader is its own surface with its own suite; here it is a host element,
// so this file can assert which prompt (and whether) the row hands it.
jest.mock("../../features/chat/components/PromptReader", () => ({ PromptReader: "PromptReader" }));
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
beforeEach(async () => {
  // These cases describe a reply's blocks as a whole. The shipped default folds
  // a settled reply onto its answer, which has its own suite
  // (`TimelineCardFinalAnswer.test.ts`); here the reply stays unfolded.
  await setAppearance({ replyDetail: "detailed" });
});
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

  test("a picture the user sent is drawn as itself, and opens the full-screen viewer", () => {
    // Desktop parity: an image in the conversation is the image, not a file name.
    // A just-sent bubble names this phone's own copy, so the bytes are already
    // here and the thumbnail opens the viewer without another transfer.
    const openViewer = jest.fn();
    const shot = attachment({ kind: "image", name: "shot.png", path: "file:///phone/shot.png" });
    renderElement(createElement(ImageViewerContext, { value: openViewer },
      createElement(TimelineCard, { item: message({ role: "user", text: "here", attachments: [shot] }), onOpenAttachment: jest.fn() })));
    expect(glyphs("Paperclip")).toHaveLength(0);
    expect(tree.root.findByType(Image).props.source).toEqual({ uri: "file:///phone/shot.png" });
    pressLabelled("shot.png");
    expect(openViewer).toHaveBeenCalledWith({ name: "shot.png", uri: "file:///phone/shot.png" });
  });

  test("attachments without an opener are disabled chips", () => {
    render(message({ role: "user", text: "here", attachments: [attachment({ name: "a.txt", kind: "file" })] }));
    const chip = tree.root.findAll(node => node.props.accessibilityLabel === "a.txt")[0]!;
    expect(chip.props.disabled).toBe(true);
  });

  // A chip draws as one 26px line, so the name alone left a target under the 44px
  // floor; the slop is what makes it hittable, and nothing on screen moves.
  test("an attachment chip extends its touch area past its drawn line", () => {
    render(message({ role: "user", text: "here", attachments: [attachment({ name: "a.txt", kind: "file" })] }), { onOpenAttachment: jest.fn() });
    const chip = tree.root.findAll(node => node.props.accessibilityLabel === "a.txt")[0]!;
    // 26px drawn + this much a side reaches `layout.touchTarget`, the arithmetic
    // the transcript's copy button does from its 28px box.
    expect(chip.props.hitSlop).toEqual({ top: (layout.touchTarget - 26) / 2, bottom: (layout.touchTarget - 26) / 2 });
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

describe("a prompt taller than the phone is read in a reader, never drawn whole", () => {
  /** A prompt of the size the phone reported: goal-sized prose that wraps to far
   * more lines than a viewport holds. */
  const giantPrompt = Array.from({ length: 40 }, (_, index) =>
    `TODO todo_${index}: ${'Coverage residual lines for this subtree — measured, classified and recorded. '.repeat(6)}`,
  ).join("\n");
  /** The prompt bubble: the only box painting the accent fill. */
  const bubble = () => tree.root.findAll(node =>
    typeof node.type === "string"
    && StyleSheet.flatten(node.props.style)?.backgroundColor === lightColors.accent)[0];
  const bubbleStyle = () => StyleSheet.flatten(bubble()?.props.style);
  const toggles = () => tree.root.findAll(node =>
    typeof node.props.onPress === "function"
    && node.props.accessibilityLabel === "Show full prompt");
  /** The markdown rows this row rendered (a host stub in this suite). */
  const markdownNodes = () => tree.root.findAll(node => String(node.type) === "MarkdownText");
  const readers = () => tree.root.findAll(node => String(node.type) === "PromptReader");

  test("the bubble draws only a preview, capped at the shared wrapped-line budget", () => {
    render(message({ role: "user", text: giantPrompt }));
    // The cap is a real height, derived from the reader's own line height and
    // window rather than a hard-coded dp: it has to grow with the text size and
    // stay inside the viewport on a short phone.
    const expectedCap = collapsedPromptHeight(
      lightTheme.chat.lineHeight,
      Dimensions.get("window").height,
    );
    expect(bubbleStyle()).toMatchObject({ maxHeight: expectedCap, overflow: "hidden" });
    expect(expectedCap).toBeGreaterThan(lightTheme.chat.lineHeight * (PROMPT_COLLAPSE_LINES - 1));
    expect(expectedCap).toBeLessThanOrEqual(Dimensions.get("window").height / 2);
    // The bubble renders a head of the source, not the whole of it: what made
    // the row blank was its height, so the row must not carry the tail at all.
    const drawn = markdownNodes()[0]!.props.text as string;
    expect(drawn.length).toBeLessThan(giantPrompt.length);
    expect(giantPrompt.startsWith(drawn)).toBe(true);
    expect(drawn.length).toBeLessThanOrEqual(800);
  });

  test("the control sits after the bubble and opens the whole prompt in a reader", () => {
    render(message({ role: "user", text: giantPrompt }));
    // Structure, not presence: a control painted above the bubble would be a
    // different screen even though both strings are on it. The DFS order of the
    // rendered tree is the document order.
    const ordered = tree.root.findAll(() => true);
    const bubbleAt = ordered.findIndex(node =>
      StyleSheet.flatten(node.props.style)?.backgroundColor === lightColors.accent);
    const controlAt = ordered.findIndex(node => node.props.accessibilityLabel === "Show full prompt");
    expect(bubbleAt).toBeGreaterThanOrEqual(0);
    expect(controlAt).toBeGreaterThan(bubbleAt);

    // Closed by default: opening a modal has to be the reader's choice, and the
    // bubble never grows into the shape that fails to draw.
    expect(readers()[0]!.props).toMatchObject({ text: giantPrompt, visible: false });
    press("Show full prompt");
    expect(readers()[0]!.props.visible).toBe(true);
    expect(bubbleStyle()?.maxHeight).toBe(collapsedPromptHeight(
      lightTheme.chat.lineHeight,
      Dimensions.get("window").height,
    ));
    // The reader closes itself, and the bubble is left exactly as it was.
    act(() => readers()[0]!.props.onClose());
    expect(readers()[0]!.props.visible).toBe(false);
  });

  test("a prompt within the budget keeps its bubble and offers no control", () => {
    // 16 lines is the code block's own threshold: at it, a prompt is short.
    const atBudget = Array.from({ length: PROMPT_COLLAPSE_LINES }, () => "one line of prompt").join("\n");
    render(message({ role: "user", text: atBudget }));
    expect(bubbleStyle()?.maxHeight).toBeUndefined();
    expect(toggles()).toHaveLength(0);
    expect(readers()).toHaveLength(0);
    expect(markdownNodes()[0]!.props.text).toBe(atBudget);
  });

  test("a reply is never capped, however long it is", () => {
    // The reply side has to stay fully readable: it is the answer, not the
    // question, and it already folds its own steps.
    render(message({ id: "a1", role: "assistant", text: giantPrompt }));
    expect(tree.root.findAll(node =>
      typeof node.type === "string"
      && StyleSheet.flatten(node.props.style)?.maxHeight !== undefined)).toHaveLength(0);
    expect(toggles()).toHaveLength(0);
    expect(readers()).toHaveLength(0);
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

  test("a run the user stopped is offered for recovery like any other", () => {
    // The reason a reply stopped is not a gate: a stop, a failure, a truncated
    // stream and the cancelled row a restart leaves behind all end the reply in
    // a place the reader may want to pick it up from.
    render(message({ id: "latest", failed: true, stopped: true, runId: "run-1" }), { isLatestAssistant: true });
    expect(hasText("Retry")).toBe(true);
    expect(hasText("Continue")).toBe(true);
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
