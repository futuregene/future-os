import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Text } from "react-native";
import { SessionPicker, sessionPickerHeight } from "../components/SessionPicker";
import { sessionMentionGroups } from "../sessionMention";
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

const groups = sessionMentionGroups(
  [
    session("c1", "Loose chat", { mode: "chat" }),
    session("s1", "Clean the data", { workspaceId: "a" }),
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

test("shows the conversations under their workspace, chats first", () => {
  act(() => { tree = create(createElement(SessionPicker, { groups, onSelect, onClose, maxHeight: 240 })); });
  // The heading names the workspace its conversations came from, and the
  // workspace-less ones are labelled rather than left under a blank heading.
  expect(texts()).toEqual(expect.arrayContaining(["sessions.reference", "sessions.conversations", "Dopamine", "Loose chat", "Clean the data"]));
});

test("selecting a row hands back the session id the reference needs", () => {
  act(() => { tree = create(createElement(SessionPicker, { groups, onSelect, onClose, maxHeight: 240 })); });
  act(() => row("Clean the data").props.onPress());
  expect(onSelect).toHaveBeenCalledWith({ sessionId: "s1", title: "Clean the data" });
});

test("reports no matches instead of an empty panel", () => {
  act(() => { tree = create(createElement(SessionPicker, { groups: [], onSelect, onClose, maxHeight: 240 })); });
  expect(texts()).toEqual(expect.arrayContaining(["sessions.noResults"]));
});

test("names an unnamed conversation and an unnamed workspace", () => {
  const orphan = sessionMentionGroups([session("s9", "   ", { workspaceId: "ghost" })], []);
  act(() => { tree = create(createElement(SessionPicker, { groups: orphan, onSelect, onClose, maxHeight: 240 })); });
  expect(texts()).toEqual(expect.arrayContaining(["sessions.unnamed", "sessions.workspace"]));
  act(() => row("sessions.unnamed").props.onPress());
  // The reference must not carry a blank title into the message.
  expect(onSelect).toHaveBeenCalledWith({ sessionId: "s9", title: "sessions.unnamed" });
});

describe("sessionPickerHeight", () => {
  test("grows with the rows and headings it has to show", () => {
    const one = sessionPickerHeight(844, 300, 1, 1);
    const more = sessionPickerHeight(844, 300, 6, 3);
    expect(more).toBeGreaterThan(one);
  });

  test("never exceeds the room the keyboard leaves, and keeps a usable floor", () => {
    // A keyboard that leaves almost nothing: still a tappable panel, not a sliver.
    expect(sessionPickerHeight(400, 380, 8, 4)).toBe(120);
    expect(sessionPickerHeight(844, 0, 8, 4)).toBeLessThanOrEqual(844);
  });
});
