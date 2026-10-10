import { createElement, useEffect } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { useAppDialog } from "../useAppDialog";
import { DialogSurface } from "../DialogSurface";
import { Button } from "../Button";

jest.mock("react-native-safe-area-context", () => ({ useSafeAreaInsets: () => ({ top: 0, bottom: 0, left: 0, right: 0 }) }));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

let api: ReturnType<typeof useAppDialog>;
let tree: ReactTestRenderer;
function Harness() { const value = useAppDialog(); useEffect(() => { api = value; }); return value.dialog; }
beforeEach(() => { act(() => { tree = create(createElement(Harness)); }); });
afterEach(() => { act(() => tree.unmount()); Platform.OS = "ios"; jest.useRealTimers(); });

test.each(["ios", "android"] as const)("%s confirms once, only after dismissal", os => {
  Platform.OS = os;
  jest.useFakeTimers();
  const confirm = jest.fn();
  act(() => api.alert("Delete", "Cannot undo", [{ text: "Cancel", style: "cancel" }, { text: "Delete", style: "destructive", onPress: confirm }]));
  expect(tree.root.findAllByType(DialogSurface)).toHaveLength(1);
  const modal = tree.root.findByType(Modal);
  const button = tree.root.findAllByType(Button).find(node => node.props.variant === "danger")!;
  act(() => { button.props.onPress(); button.props.onPress(); });
  expect(confirm).not.toHaveBeenCalled();
  act(() => { if (os === "ios") modal.props.onDismiss(); else jest.runOnlyPendingTimers(); });
  act(() => modal.props.onDismiss());
  expect(confirm).toHaveBeenCalledTimes(1);
});

test.each(["ios", "android"] as const)("%s uses platform-safe selection without truncating notice text", os => {
  Platform.OS = os;
  act(() => api.alert("Update", "Current build\nAvailable build"));
  const message = tree.root.findAllByType(Text).find(node => node.props.children === "Current build\nAvailable build")!;
  expect(message).toBeDefined();
  expect(message.props.selectable).toBe(os === "ios");
  expect(message.props.numberOfLines).toBeUndefined();
});

test("long notice text scrolls without moving its single-row action footer", () => {
  const message = "A detailed error message\n".repeat(100);
  act(() => api.alert("Error", message, [{ text: "Cancel", style: "cancel" }, { text: "Retry" }]));
  const scroll = tree.root.findByType(ScrollView);
  expect(scroll.findAllByType(Text).some(node => node.props.children === message)).toBe(true);
  expect(scroll.findAllByType(Button)).toHaveLength(0);
  const buttons = tree.root.findAllByType(Button);
  expect(buttons.map(node => node.props.label)).toEqual(["Cancel", "Retry"]);
  const actions = tree.root.findAllByType(View).find(node => StyleSheet.flatten(node.props.style)?.flexDirection === "row" && node.findAllByType(Button).length === 2)!;
  expect(StyleSheet.flatten(actions.props.style).flexWrap).toBeUndefined();
  const dialog = tree.root.findByType(DialogSurface);
  expect(dialog.props.footer).toBeDefined();
});

test("title-only notices still have a pinned close action", () => {
  act(() => api.alert("Up to date"));
  expect(tree.root.findByType(ScrollView).findAllByType(Text).map(node => node.props.children)).toEqual(["Up to date"]);
  expect(tree.root.findByType(Button).props.label).toBe("common.close");
});

test("system back cancels without invoking the destructive action", () => {
  Platform.OS = "ios";
  const confirm = jest.fn();
  act(() => api.alert("Delete", "Cannot undo", [{ text: "Cancel", style: "cancel" }, { text: "Delete", style: "destructive", onPress: confirm }]));
  const modal = tree.root.findByType(Modal);
  act(() => modal.props.onRequestClose());
  act(() => modal.props.onDismiss());
  expect(confirm).not.toHaveBeenCalled();
});

test("a dialog left open when the surface goes away is dismissed, not left on screen", () => {
  function Surface({ active }: { active: boolean }) {
    const dialog = useAppDialog(active);
    useEffect(() => { api = dialog; });
    return dialog.dialog;
  }
  act(() => { tree = create(createElement(Surface, { active: true })); });
  act(() => api.alert("Delete", "Cannot undo", [{ text: "Delete", onPress: jest.fn() }]));
  expect(tree.root.findAllByType(DialogSurface)).toHaveLength(1);
  // The screen that owns the dialog is no longer active (a tab switch, a
  // navigation): its dialog must not survive over the surface that replaced it.
  act(() => { tree.update(createElement(Surface, { active: false })); });
  expect(tree.root.findAllByType(DialogSurface)).toHaveLength(0);
});

