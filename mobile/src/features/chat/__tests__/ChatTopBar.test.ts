import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ChatTopBar } from "../components/ChatTopBar";

jest.mock("lucide-react-native", () => ({ ArrowLeft: "ArrowLeft", FolderOpen: "FolderOpen", Pencil: "Pencil" }));
let tree: ReactTestRenderer;
const props = {
  title: "Session", draft: false, backLabel: "Back", renameLabel: "Rename",
  filesLabel: "Files", filesOpen: false,
  onBack: jest.fn(), onRename: jest.fn(), onFiles: jest.fn(),
};
const button = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && node.props.onPress);
beforeEach(() => jest.clearAllMocks());
afterEach(() => act(() => tree.unmount()));

test("existing sessions expose an accessible file toggle and retain back/rename actions", () => {
  act(() => { tree = create(createElement(ChatTopBar, props)); });
  expect(button("Files")[0]!.props.accessibilityState.expanded).toBe(false);
  act(() => button("Files")[0]!.props.onPress());
  expect(props.onFiles).toHaveBeenCalledTimes(1);
  act(() => { tree.update(createElement(ChatTopBar, { ...props, filesOpen: true })); });
  expect(button("Files")[0]!.props.accessibilityState.expanded).toBe(true);
  act(() => button("Back")[0]!.props.onPress());
  act(() => button("Rename")[0]!.props.onPress());
  expect(props.onBack).toHaveBeenCalledTimes(1);
  expect(props.onRename).toHaveBeenCalledTimes(1);
});

test("unsent drafts do not expose a file directory", () => {
  act(() => { tree = create(createElement(ChatTopBar, { ...props, draft: true })); });
  expect(button("Files")).toHaveLength(0);
});
