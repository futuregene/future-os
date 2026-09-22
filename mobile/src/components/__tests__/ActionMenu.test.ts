import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, Platform, ScrollView, StyleSheet, Text, TextInput } from "react-native";
import { ActionMenu } from "../ActionMenu";
jest.mock("lucide-react-native", () => ({ Search: "Search", X: "X" }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
let tree: ReactTestRenderer;
const action = jest.fn();
const close = jest.fn();
const button = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && node.props.onPress)[0]!;
beforeEach(() => {
  jest.clearAllMocks();
  act(() => { tree = create(createElement(ActionMenu, { title: "Actions", visible: true, onClose: close, actions: [{ label: "New", onPress: action }] })); });
});
afterEach(() => { act(() => tree.unmount()); Platform.OS = "ios"; jest.useRealTimers(); });
test("iOS waits for native dismissal and cannot fire an action twice", () => {
  act(() => { button("New").props.onPress(); button("New").props.onPress(); });
  expect(close).toHaveBeenCalledTimes(1);
  expect(action).not.toHaveBeenCalled();
  act(() => { tree.root.findByType(Modal).props.onDismiss(); tree.root.findByType(Modal).props.onDismiss(); });
  expect(action).toHaveBeenCalledTimes(1);
});
test("Android executes after closing without relying on iOS onDismiss", () => {
  Platform.OS = "android";
  jest.useFakeTimers();
  act(() => button("New").props.onPress());
  expect(action).not.toHaveBeenCalled();
  act(() => jest.runAllTimers());
  expect(action).toHaveBeenCalledTimes(1);
});
test("cancel and hardware back do not execute any action", () => {
  act(() => button("chat.cancel").props.onPress());
  act(() => tree.root.findByType(Modal).props.onRequestClose());
  act(() => tree.root.findByType(Modal).props.onDismiss());
  expect(action).not.toHaveBeenCalled();
});
test("actions have the same roomy touch target as other app sheets", () => {
  expect(StyleSheet.flatten(button("New").props.style({ pressed: false })).minHeight).toBeGreaterThanOrEqual(44);
});

test("long titles stay in one header line and expand only inside the scrolling list", () => {
  const title = "A long filename ".repeat(100) + ".pdf";
  act(() => tree.update(createElement(ActionMenu, { title, visible: true, onClose: close, actions: [{ label: "New", onPress: action }] })));
  const header = tree.root.findAllByType(Text).find(node => node.props.accessibilityRole === "header")!;
  expect(header.props.numberOfLines).toBe(1);
  expect(header.props.ellipsizeMode).toBe("middle");
  expect(button(title).props.accessibilityState.expanded).toBe(false);
  expect(StyleSheet.flatten(button(title).props.style)).toMatchObject({ minWidth: 0, minHeight: 44, flex: 1 });
  const fullTitles = () => tree.root.findByType(ScrollView).findAllByType(Text).filter(node => node.props.children === title);
  expect(fullTitles()).toHaveLength(0);
  act(() => button(title).props.onPress());
  expect(fullTitles()).toHaveLength(1);
  expect(fullTitles()[0]!.props.numberOfLines).toBeUndefined();
  expect(button(title).props.accessibilityState.expanded).toBe(true);
  expect(close).not.toHaveBeenCalled();
  expect(action).not.toHaveBeenCalled();
  act(() => button(title).props.onPress());
  expect(fullTitles()).toHaveLength(0);
});

test("group headings label what follows without being choices of their own", () => {
  act(() => tree.update(createElement(ActionMenu, {
    title: "Actions",
    visible: true,
    onClose: close,
    actions: [
      { label: "New", onPress: action },
      { label: "Project", heading: true },
      { label: "Task", nested: true, onPress: action },
    ],
  })));
  expect(button("Project")).toBeUndefined();
  const flat = (label: string) => StyleSheet.flatten(button(label).props.style({ pressed: false }));
  expect(flat("Task").paddingLeft).toBeGreaterThan(flat("New").paddingHorizontal);
  act(() => button("Task").props.onPress());
  act(() => tree.root.findByType(Modal).props.onDismiss());
  expect(action).toHaveBeenCalledTimes(1);
});

test("a long list can be filtered from the sheet's own search field", () => {
  const onChangeText = jest.fn();
  const props = { title: "Actions", visible: true, onClose: close, actions: [{ label: "New", onPress: action }] };
  const search = { value: "", label: "share.search", placeholder: "share.search", onChangeText };
  act(() => tree.update(createElement(ActionMenu, { ...props, search })));
  expect(tree.root.findByType(TextInput).props.placeholder).toBe("share.search");
  act(() => tree.root.findByType(TextInput).props.onChangeText("task"));
  expect(onChangeText).toHaveBeenCalledWith("task");
  act(() => tree.update(createElement(ActionMenu, { ...props, search: { ...search, value: "task" } })));
  act(() => button("sessions.clearSearch").props.onPress());
  expect(onChangeText).toHaveBeenLastCalledWith("");
  expect(close).not.toHaveBeenCalled();
});

test("expanded titles reset when the menu closes or switches to a different target", () => {
  act(() => button("Actions").props.onPress());
  act(() => button("chat.cancel").props.onPress());
  expect(button("Actions").props.accessibilityState.expanded).toBe(false);
  act(() => button("Actions").props.onPress());
  const props = { title: "Actions", visible: true, onClose: close, actions: [] };
  act(() => tree.update(createElement(ActionMenu, { ...props, visible: false })));
  act(() => tree.update(createElement(ActionMenu, props)));
  expect(button("Actions").props.accessibilityState.expanded).toBe(false);
  act(() => button("Actions").props.onPress());
  act(() => tree.update(createElement(ActionMenu, { ...props, title: "Another file" })));
  expect(button("Another file").props.accessibilityState.expanded).toBe(false);
});
