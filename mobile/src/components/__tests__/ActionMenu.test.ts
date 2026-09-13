import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, Platform, StyleSheet } from "react-native";
import { ActionMenu } from "../ActionMenu";
jest.mock("lucide-react-native", () => ({ X: "X" }));
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
