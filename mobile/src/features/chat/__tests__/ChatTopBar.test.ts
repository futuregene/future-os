import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { PeerIconGlyph } from "../../../remote/PeerIconGlyph";
import { ChatTopBar } from "../components/ChatTopBar";

jest.mock("lucide-react-native", () => ({
  ArrowLeft: "ArrowLeft",
  FolderOpen: "FolderOpen",
  ReceiptText: "ReceiptText",
}));
let tree: ReactTestRenderer;
const props = {
  title: "Session", contextLabel: "Non-workspace conversation", draft: false, backLabel: "Back",
  usageLabel: "Token usage and amount",
  filesLabel: "Files", filesOpen: false,
  onBack: jest.fn(), onUsage: jest.fn(), onFiles: jest.fn(),
};
const button = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && node.props.onPress);
// The rendered order, so a bar that grows an extra trailing control fails here.
const labels = () => tree.root
  .findAll(node => node.props.accessibilityRole === "button" && node.props.onPress)
  .map(node => node.props.accessibilityLabel);

beforeEach(() => jest.clearAllMocks());
afterEach(() => act(() => tree.unmount()));

test("a session shows files and the spend breakdown, plus back — and nothing else", () => {
  act(() => { tree = create(createElement(ChatTopBar, props)); });
  expect(button("Files")[0]!.props.accessibilityState.expanded).toBe(false);
  act(() => button("Files")[0]!.props.onPress());
  expect(props.onFiles).toHaveBeenCalledTimes(1);
  act(() => { tree.update(createElement(ChatTopBar, { ...props, filesOpen: true })); });
  expect(button("Files")[0]!.props.accessibilityState.expanded).toBe(true);
  // The spend entry is an icon, not the amount: the bar must not carry a
  // number that changes every turn.
  const usageButton = button("Token usage and amount")[0]!;
  expect(usageButton.findAll(node => String(node.type) === "ReceiptText")).toHaveLength(1);
  expect(tree.root.findAll(node => typeof node.children[0] === "string" && node.children[0].startsWith("¥"))).toHaveLength(0);
  act(() => usageButton.props.onPress());
  act(() => button("Back")[0]!.props.onPress());
  expect(props.onUsage).toHaveBeenCalledTimes(1);
  expect(props.onBack).toHaveBeenCalledTimes(1);
  // Renaming lives in the session list; the conversation bar must not repeat it.
  expect(labels()).toEqual(["Back", "Files", "Token usage and amount"]);
});

test.each(["Non-workspace conversation", "Workspace · Research", "Workspace conversation"])("unsent drafts show %s before the first message", contextLabel => {
  act(() => { tree = create(createElement(ChatTopBar, { ...props, draft: true, contextLabel })); });
  // Neither action applies to a session-less draft; the spacers that replace
  // them carry no label, which is what keeps the title centered.
  expect(labels()).toEqual(["Back"]);
  expect(tree.root.findAll(node => node.props.accessibilityLabel === contextLabel).length).toBeGreaterThan(0);
});

/**
 * The bar names the desktop the conversation is on, on the line that already
 * says where it is.
 *
 * The phone has no agent of its own, so every conversation here lives on a paired
 * desktop; the machine goes on the *context* line (with the workspace, or the chat
 * section) rather than in a row of its own — the layout keeps to one line, and the
 * machine is a qualifier of the conversation, not a second title.
 */
test("the context line carries the machine, and both halves stay addressable", () => {
  act(() => {
    tree = create(createElement(ChatTopBar, {
      ...props,
      contextLabel: "Workspace · Research",
      machine: { icon: "laptop", label: "Lab MacBook Pro" },
    }));
  });

  // The host node, not its wrapper: a testID is on both. The machine is now a
  // row — the shared set's mark, then the name — so the name is what the Text
  // holds, and the mark is a drawn glyph rather than a character in it.
  const hostRow = tree.root.findAll(node =>
    node.props.testID === "chat-machine" && String(node.type) === "View");
  expect(hostRow).toHaveLength(1);
  expect(hostRow[0]!.props.accessibilityLabel).toBe("Lab MacBook Pro");
  const hostText = hostRow[0]!.findAll(node => String(node.type) === "Text");
  expect(hostText.map(node => node.children.join(""))).toEqual(["Lab MacBook Pro"]);
  // The mark is the shared set's, handed the machine's own icon id: this file
  // stubs the icon module, so what can be checked here is that the mark is
  // drawn from that set rather than typed into the name (the drawing itself is
  // `remote/__tests__/peerIcons.test.ts`).
  expect(hostRow[0]!.findAllByType(PeerIconGlyph).map(node => node.props.id)).toEqual(["laptop"]);
  // The workspace it sits beside is still named, and the bar's controls are
  // untouched by the extra element.
  expect(tree.root.findAll(node => node.props.accessibilityLabel === "Workspace · Research").length).toBeGreaterThan(0);
  expect(labels()).toEqual(["Back", "Files", "Token usage and amount"]);

  // A bar built without a machine (the tests, and any caller that has none)
  // renders exactly what it did before.
  act(() => { tree.update(createElement(ChatTopBar, props)); });
  expect(tree.root.findAll(node => node.props.testID === "chat-machine")).toHaveLength(0);
  expect(tree.root.findAll(node => node.props.accessibilityLabel === props.contextLabel).length).toBeGreaterThan(0);
});
