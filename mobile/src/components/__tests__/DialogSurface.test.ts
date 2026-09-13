import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Keyboard, KeyboardAvoidingView, Platform, ScrollView, StyleSheet, Text, View } from "react-native";
import { DialogSurface } from "../DialogSurface";
import { Button } from "../Button";

jest.mock("react-native-safe-area-context", () => ({
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 20, right: 10 }),
}));

let tree: ReactTestRenderer;
const originalOS = Platform.OS;
afterEach(() => {
  if (tree) act(() => tree.unmount());
  Platform.OS = originalOS;
  jest.restoreAllMocks();
});

test("dialog scrolls within safe-area gutters and keeps actions tappable with the keyboard open", () => {
  Platform.OS = "ios";
  act(() => { tree = create(createElement(DialogSurface, null, createElement(Text, null, "Dialog"))); });
  expect(tree.root.findByType(KeyboardAvoidingView).props.behavior).toBe("padding");
  const scroll = tree.root.findByType(ScrollView);
  expect(scroll.props.keyboardShouldPersistTaps).toBe("handled");
  expect(StyleSheet.flatten(scroll.props.contentContainerStyle)).toMatchObject({ flexGrow: 1, padding: 16 });
  const viewport = tree.root.findAllByType(View).find(node =>
    StyleSheet.flatten(node.props.style)?.paddingLeft === 20,
  )!;
  expect(StyleSheet.flatten(viewport.props.style)).toMatchObject({ paddingTop: 24, paddingBottom: 34, paddingRight: 10 });
});

test("Android edge-to-edge dialog follows the IME and releases keyboard listeners", () => {
  Platform.OS = "android";
  const listeners = new Map<string, Parameters<typeof Keyboard.addListener>[1]>();
  const event = (height: number) => ({
    duration: 200,
    easing: "keyboard" as const,
    endCoordinates: { height, width: 390, screenX: 0, screenY: 844 - height },
  });
  const remove = jest.fn();
  jest.spyOn(Keyboard, "addListener").mockImplementation((name, callback) => {
    listeners.set(name, callback);
    return { remove } as unknown as ReturnType<typeof Keyboard.addListener>;
  });
  act(() => { tree = create(createElement(DialogSurface)); });
  const viewportStyle = () => StyleSheet.flatten(tree.root.findAllByType(View).find(node =>
    StyleSheet.flatten(node.props.style)?.paddingLeft === 20,
  )!.props.style);
  expect(viewportStyle().paddingBottom).toBe(34);
  act(() => listeners.get("keyboardDidShow")!(event(300)));
  expect(viewportStyle().paddingBottom).toBe(300);
  act(() => listeners.get("keyboardDidHide")!(event(0)));
  expect(viewportStyle().paddingBottom).toBe(34);
  act(() => tree.unmount());
  expect(remove).toHaveBeenCalledTimes(jest.mocked(Keyboard.addListener).mock.calls.length);
});

test("compact buttons retain a 44-point hit target and wrap long labels", () => {
  act(() => { tree = create(createElement(Button, { compact: true, label: "A long localized action", onPress: jest.fn() })); });
  const button = tree.root.findAll(node => node.props.accessibilityRole === "button" && typeof node.props.style === "function")[0]!;
  expect(StyleSheet.flatten(button.props.style({ pressed: false })).minHeight).toBe(44);
  expect(StyleSheet.flatten(tree.root.findByType(Text).props.style)).toMatchObject({ flexShrink: 1, textAlign: "center" });
});
