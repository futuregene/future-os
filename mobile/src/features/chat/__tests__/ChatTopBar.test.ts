import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ChatTopBar } from "../components/ChatTopBar";

jest.mock("lucide-react-native", () => ({ ArrowLeft: "ArrowLeft", FolderOpen: "FolderOpen" }));
let tree: ReactTestRenderer;
const props = {
  title: "Session", contextLabel: "Non-workspace conversation", draft: false, backLabel: "Back",
  usageLabel: "Token usage and amount", usageText: "¥0.0234",
  filesLabel: "Files", filesOpen: false,
  onBack: jest.fn(), onUsage: jest.fn(), onFiles: jest.fn(),
};
const button = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && node.props.onPress);
/** The single Text node rendering `value`, however deeply it is wrapped. */
const texts = (value: string) => tree.root.findAll(node => node.children.includes(value));
beforeEach(() => jest.clearAllMocks());
afterEach(() => act(() => tree.unmount()));

test("existing sessions expose the file toggle, the running amount, and back", () => {
  act(() => { tree = create(createElement(ChatTopBar, props)); });
  expect(button("Files")[0]!.props.accessibilityState.expanded).toBe(false);
  act(() => button("Files")[0]!.props.onPress());
  expect(props.onFiles).toHaveBeenCalledTimes(1);
  act(() => { tree.update(createElement(ChatTopBar, { ...props, filesOpen: true })); });
  expect(button("Files")[0]!.props.accessibilityState.expanded).toBe(true);
  act(() => button("Back")[0]!.props.onPress());
  // The amount is the amount *and* the entry point to the breakdown.
  expect(texts("¥0.0234").length).toBeGreaterThan(0);
  act(() => button("Token usage and amount")[0]!.props.onPress());
  expect(props.onBack).toHaveBeenCalledTimes(1);
  expect(props.onUsage).toHaveBeenCalledTimes(1);
});

test.each(["Non-workspace conversation", "Workspace · Research", "Workspace conversation"])("unsent drafts show %s before the first message", contextLabel => {
  act(() => { tree = create(createElement(ChatTopBar, { ...props, draft: true, contextLabel })); });
  expect(button("Files")).toHaveLength(0);
  // A draft has no session yet, so there is nothing to account for.
  expect(button("Token usage and amount")).toHaveLength(0);
  expect(tree.root.findAll(node => node.props.accessibilityLabel === contextLabel).length).toBeGreaterThan(0);
});
