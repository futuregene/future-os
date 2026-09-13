import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { TimelineCard } from "../TimelineCard";
import type { TimelineItem } from "../../remote/types";

jest.mock("../MarkdownText", () => ({ MarkdownText: "MarkdownText" }));
jest.mock("lucide-react-native", () => Object.fromEntries([
  "AlertTriangle", "Check", "ChevronDown", "ChevronUp", "CircleAlert", "Copy", "FileText", "Paperclip", "Pencil", "TerminalSquare", "TriangleAlert", "X",
].map(name => [name, name])));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key, i18n: { language: "en" } }) }));

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
