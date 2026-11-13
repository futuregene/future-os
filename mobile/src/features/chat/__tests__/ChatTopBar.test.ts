import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { PeerIconGlyph } from "../../../remote/PeerIconGlyph";
import { ChatTopBar } from "../components/ChatTopBar";

jest.mock("lucide-react-native", () => ({
  ArrowLeft: "ArrowLeft",
  Ellipsis: "Ellipsis",
}));
let tree: ReactTestRenderer;
const props = {
  title: "Session", contextLabel: "Non-workspace conversation", draft: false, backLabel: "Back",
  menuLabel: "Conversation actions", menuOpen: false,
  onBack: jest.fn(), onMenu: jest.fn(),
};
const button = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && node.props.onPress);
// The rendered order, so a bar that grows an extra trailing control fails here.
const labels = () => tree.root
  .findAll(node => node.props.accessibilityRole === "button" && node.props.onPress)
  .map(node => node.props.accessibilityLabel);

beforeEach(() => jest.clearAllMocks());
afterEach(() => act(() => tree.unmount()));

/**
 * The bar carries one trailing control, and it opens the conversation's menu.
 *
 * The session's files and its spend breakdown used to be two icons; they are
 * occasional actions on a whole conversation, and two of them spent the title's
 * width on things opened once a session. What is behind the dots is the caller's
 * (`ChatScreen`), so this file checks the button and the bar's shape, not the
 * list.
 */
test("a session shows one trailing menu button beside back, and nothing else", () => {
  act(() => { tree = create(createElement(ChatTopBar, props)); });
  expect(button("Conversation actions")[0]!.props.accessibilityState.expanded).toBe(false);
  act(() => button("Conversation actions")[0]!.props.onPress());
  expect(props.onMenu).toHaveBeenCalledTimes(1);
  act(() => { tree.update(createElement(ChatTopBar, { ...props, menuOpen: true })); });
  expect(button("Conversation actions")[0]!.props.accessibilityState.expanded).toBe(true);
  // The dot glyph itself, not a number: the bar must not carry a figure that
  // changes every turn.
  expect(button("Conversation actions")[0]!.findAll(node => String(node.type) === "Ellipsis")).toHaveLength(1);
  expect(tree.root.findAll(node => typeof node.children[0] === "string" && node.children[0].startsWith("¥"))).toHaveLength(0);
  act(() => button("Back")[0]!.props.onPress());
  expect(props.onBack).toHaveBeenCalledTimes(1);
  // Files and the spend breakdown are in the menu now; renaming lives in the
  // session list. Neither may come back as a bar control.
  expect(labels()).toEqual(["Back", "Conversation actions"]);
  expect(tree.root.findAll(node => node.props.accessibilityLabel === "Files")).toHaveLength(0);
  expect(tree.root.findAll(node => node.props.accessibilityLabel === "Token usage and amount")).toHaveLength(0);
});

test.each(["Non-workspace conversation", "Workspace · Research", "Workspace conversation"])("unsent drafts show %s before the first message", contextLabel => {
  act(() => { tree = create(createElement(ChatTopBar, { ...props, draft: true, contextLabel })); });
  // The menu does not apply to a session-less draft; the spacer that replaces it
  // carries no label, which is what keeps the title centered.
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
  expect(hostRow[0]!.findAllByType(PeerIconGlyph)).toHaveLength(1);
  expect(tree.root.findAll(node => node.props.accessibilityLabel === "Workspace · Research").length).toBeGreaterThan(0);
  expect(labels()).toEqual(["Back", "Conversation actions"]);

  // A bar built without a machine (the tests, and any caller that has none)
  // renders exactly what it did before.
  act(() => { tree.update(createElement(ChatTopBar, props)); });
  expect(tree.root.findAll(node => node.props.testID === "chat-machine")).toHaveLength(0);
  expect(tree.root.findAll(node => node.props.accessibilityLabel === props.contextLabel).length).toBeGreaterThan(0);
});

/**
 * The workspace half of the context line wraps rather than disappearing.
 *
 * The machine caps at 70% of the row and never shrinks (see `contextLine`), so on
 * a 390 pt phone the workspace side gets ~80 pt: one line of that printed
 * 「工作区 · 多巴…」 — the machine's name and nothing of the workspace the
 * reader needs to tell one of that machine's conversations from another. The
 * bar is a 60 pt minimum that the line was using half of, so the name is allowed
 * the second line it needs; the machine keeps one, because it is the half that
 * has to stay whole.
 */
test("the workspace wraps instead of truncating, and the machine stays on one line", () => {
  act(() => {
    tree = create(createElement(ChatTopBar, {
      ...props,
      contextLabel: "Workspace · 多巴胺与决策",
      machine: { icon: "laptop", label: "实验室的 MacBook Pro" },
    }));
  });

  const context = tree.root.findAll(node =>
    node.props.accessibilityLabel === "Workspace · 多巴胺与决策" && String(node.type) === "Text");
  expect(context).toHaveLength(1);
  expect(context[0]!.props.numberOfLines).toBe(2);
  const machineLabel = tree.root.findAll(node =>
    node.props.testID === "chat-machine" && String(node.type) === "View")[0]!
    .findAll(node => String(node.type) === "Text");
  expect(machineLabel.map(node => node.props.numberOfLines)).toEqual([1]);
});
