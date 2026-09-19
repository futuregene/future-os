import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
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
