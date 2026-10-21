import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import * as Clipboard from "expo-clipboard";
import { TimelineCard } from "../TimelineCard";
import type { TimelineItem, TimelineSegment } from "../../remote/types";
import { setAppearance } from "../../theme/preferences";
import "../../i18n";

/**
 * The phone's own reading setting: a settled reply keeps only the answer it
 * ends on, with the work that produced it folded into one row above it.
 *
 * The fold is a render concern of this client — the desktop sends the same
 * ordered slices either way — so it is exercised here on a plain segment list,
 * the shape both a local and any paired desktop's transcript arrives in.
 */
jest.mock("@react-native-async-storage/async-storage", () => ({
  getItem: jest.fn(async () => null),
  setItem: jest.fn(async () => {}),
}));
jest.mock("../MarkdownText", () => ({ MarkdownText: "MarkdownText" }));
jest.mock("../../features/chat/components/PromptReader", () => ({ PromptReader: "PromptReader" }));
jest.mock("../appAlerts", () => ({ AppAlert: { alert: jest.fn() } }));
jest.mock("expo-clipboard", () => ({ setStringAsync: jest.fn(async () => true) }));
jest.mock("lucide-react-native", () => Object.fromEntries(
  ["Brain", "Check", "ChevronDown", "ChevronUp", "Copy", "GitBranch", "ListTree", "Wrench"]
    .map(name => [name, name]),
));

let tree: ReactTestRenderer;
afterEach(() => {
  if (tree) act(() => tree.unmount());
  jest.clearAllMocks();
});

const note = (id: string, text: string): TimelineSegment => ({ id, kind: "text", text });
const thought = (id: string): TimelineSegment => ({ id, kind: "thinking", text: "weighing it" });
const call = (id: string): TimelineSegment => ({ id, kind: "tool", tool: { name: "read", complete: true } });

// The shape the setting exists for: notes and calls through the task, the
// answer last.
const workedReply: TimelineSegment[] = [
  note("note", "I will read the relay implementation first."),
  thought("thought"),
  call("call"),
  note("answer", "The plaintext relay works end to end."),
];

/** Set the setting the way the appearance page does. */
async function setFinalAnswerOnly(value: boolean) {
  await act(async () => { await setAppearance({ finalAnswerOnly: value }); });
}

function render(item: TimelineItem) {
  act(() => { tree = create(createElement(TimelineCard, { item })); });
}

const message = (segments: TimelineSegment[], over: Partial<Extract<TimelineItem, { kind: "message" }>> = {}): TimelineItem => ({
  id: "m",
  kind: "message",
  role: "assistant",
  segments,
  text: segments.flatMap(segment => (segment.kind === "text" ? [segment.text] : [])).join("\n\n"),
  ...over,
});

/** The markdown rows this row rendered, in order (the renderer is a stub here). */
function prose() {
  return tree.root.findAll(node => String(node.type) === "MarkdownText").map(node => node.props.text as string);
}

/** The tappable row whose accessibility label starts with `prefix`. */
function pressable(prefix: string) {
  return tree.root.findAll(candidate =>
    typeof candidate.props.onPress === "function"
    && typeof candidate.props.accessibilityLabel === "string"
    && candidate.props.accessibilityLabel.startsWith(prefix)).at(-1);
}

beforeEach(async () => {
  await setFinalAnswerOnly(false);
});

it("shows the whole reply when the setting is off", () => {
  render(message(workedReply));

  expect(prose()).toEqual(["I will read the relay implementation first.", "The plaintext relay works end to end."]);
  expect(pressable("Show process")).toBeUndefined();
});

it("folds the work into one counted row above the answer", async () => {
  await setFinalAnswerOnly(true);
  render(message(workedReply));

  expect(prose()).toEqual(["The plaintext relay works end to end."]);
  const toggle = pressable("Show process")!;
  expect(toggle).toBeDefined();
  expect(toggle.props.accessibilityState.expanded).toBe(false);
  // Counts describe work, not rows, and the wording is spoken rather than
  // painted: the line has to survive a phone's margin.
  expect(toggle.props.accessibilityLabel).toBe("Show process · Tool calls 1× · Thought 1×");
});

it("opens to the same blocks the setting hides, and closes again", async () => {
  await setFinalAnswerOnly(true);
  render(message(workedReply));

  act(() => pressable("Show process")!.props.onPress());
  expect(prose()).toEqual(["I will read the relay implementation first.", "The plaintext relay works end to end."]);
  expect(pressable("Hide process")).toBeDefined();

  act(() => pressable("Hide process")!.props.onPress());
  expect(prose()).toEqual(["The plaintext relay works end to end."]);
});

it("keeps a streaming reply whole, and folds it once it settles", async () => {
  await setFinalAnswerOnly(true);
  render(message(workedReply, { streaming: true }));
  expect(prose()).toEqual(["I will read the relay implementation first.", "The plaintext relay works end to end."]);
  expect(pressable("Show process")).toBeUndefined();

  render(message(workedReply));
  expect(prose()).toEqual(["The plaintext relay works end to end."]);
});

it("never folds a reply with no answer to isolate", async () => {
  const worked: TimelineSegment[] = [thought("thought"), call("call")];
  render(message(worked));
  const unfolded = prose();

  await setFinalAnswerOnly(true);
  render(message(worked));
  expect(pressable("Show process")).toBeUndefined();
  expect(prose()).toEqual(unfolded);
});

it("copies the answer the row shows, not the prose the fold hides", async () => {
  await setFinalAnswerOnly(true);
  render(message(workedReply));

  const copy = tree.root.findAll(candidate => typeof candidate.props.onPress === "function"
    && candidate.props.accessibilityLabel === "Copy response").at(-1)!;
  await act(async () => { copy.props.onPress(); });
  expect(Clipboard.setStringAsync).toHaveBeenCalledWith("The plaintext relay works end to end.");
});
