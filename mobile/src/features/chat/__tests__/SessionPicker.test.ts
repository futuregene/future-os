import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Text } from "react-native";
import { SessionPicker, sessionPickerHeight } from "../components/SessionPicker";
import { sessionMentionOptions } from "../sessionMention";
import type { RemoteSession, RemoteWorkspace } from "../../../remote/types";

jest.mock("lucide-react-native", () => ({ MessageCircle: () => null, X: () => null }));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

const session = (sessionId: string, title: string, extra: Partial<RemoteSession> = {}): RemoteSession => ({
  sessionId,
  threadId: `t_${sessionId}`,
  title,
  mode: "workspace",
  streaming: false,
  ...extra,
});
const workspace = (id: string, name: string): RemoteWorkspace => ({ id, name, path: `/p/${id}` });

const options = sessionMentionOptions(
  [
    session("c1", "Loose chat", { mode: "chat", lastMessageAt: 1 }),
    session("s1", "Clean the data", { workspaceId: "a", lastMessageAt: 2 }),
  ],
  [workspace("a", "Dopamine")],
);
const onSelect = jest.fn();
const onClose = jest.fn();
let tree: ReactTestRenderer;
const texts = () => tree.root.findAllByType(Text).map(node => node.props.children);
const row = (label: string) =>
  tree.root.findAll(node => node.props.accessibilityHint === "sessions.referenceHint"
    && node.props.accessibilityLabel === label && node.props.onPress)[0]!;

beforeEach(() => { jest.clearAllMocks(); });
afterEach(() => { if (tree) act(() => tree.unmount()); });

test("lists every conversation in the order it was given, naming its workspace", () => {
  act(() => { tree = create(createElement(SessionPicker, { sessions: options, onSelect, onClose, maxHeight: 240 })); });
  // One recency-ordered list: the workspace name is a row label, not a heading.
  expect(texts()).toEqual(expect.arrayContaining([
    "sessions.reference", "Dopamine", "Clean the data", "Loose chat",
  ]));
  const labels = tree.root
    .findAll(node => node.props.accessibilityHint === "sessions.referenceHint" && node.props.onPress)
    .map(node => node.props.accessibilityLabel);
  expect(labels).toEqual(["Clean the data", "Loose chat"]);
});

test("selecting a row hands back the session id the reference needs", () => {
  act(() => { tree = create(createElement(SessionPicker, { sessions: options, onSelect, onClose, maxHeight: 240 })); });
  act(() => row("Clean the data").props.onPress());
  expect(onSelect).toHaveBeenCalledWith({ sessionId: "s1", title: "Clean the data" });
});

test("reports no matches instead of an empty panel", () => {
  act(() => { tree = create(createElement(SessionPicker, { sessions: [], onSelect, onClose, maxHeight: 240 })); });
  expect(texts()).toEqual(expect.arrayContaining(["sessions.noResults"]));
});

test("names an unnamed conversation and an unnamed workspace", () => {
  const orphan = sessionMentionOptions([session("s9", "   ", { workspaceId: "ghost" })], []);
  act(() => { tree = create(createElement(SessionPicker, { sessions: orphan, onSelect, onClose, maxHeight: 240 })); });
  expect(texts()).toEqual(expect.arrayContaining(["sessions.unnamed", "sessions.workspace"]));
  act(() => row("sessions.unnamed").props.onPress());
  // The reference must not carry a blank title into the message.
  expect(onSelect).toHaveBeenCalledWith({ sessionId: "s9", title: "sessions.unnamed" });
});

test("labels no workspace on a chat, and names one on a workspace conversation", () => {
  act(() => { tree = create(createElement(SessionPicker, { sessions: options, onSelect, onClose, maxHeight: 240 })); });
  const labels = texts();
  expect(labels).toContain("Dopamine");
  // One occurrence only: the workspace's own row carries it, the chat's does not.
  expect(labels.filter(text => text === "Dopamine")).toHaveLength(1);
});

describe("sessionPickerHeight", () => {
  test("grows with the rows it has to show", () => {
    expect(sessionPickerHeight(844, 300, 6)).toBeGreaterThan(sessionPickerHeight(844, 300, 1));
  });

  test("reserves no room for headings, so every line of the panel is a row", () => {
    // Four rows plus the header and both borders, exactly: the old panel also
    // reserved a heading strip per group, which is why the same height now shows
    // more conversations.
    expect(sessionPickerHeight(844, 0, 4)).toBe(2 * 1 + 44 + 4 * 45);
  });

  test("never exceeds the room the keyboard leaves, and keeps a usable floor", () => {
    // A keyboard that leaves almost nothing: still a tappable panel, not a sliver.
    expect(sessionPickerHeight(400, 380, 8)).toBe(120);
    expect(sessionPickerHeight(844, 0, 8)).toBeLessThanOrEqual(844);
  });
});
