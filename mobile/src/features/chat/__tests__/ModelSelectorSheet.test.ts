import { createElement, type ComponentProps } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, ScrollView, StyleSheet, View } from "react-native";
import { ModelSelectorSheet } from "../components/ModelSelectorSheet";

jest.mock("lucide-react-native", () => ({ Check: "Check", X: "X" }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));
jest.mock("../../../remote/RemoteContext", () => ({ useRemote: jest.fn() }));

const remote = {
  modelId: "provider/model-a",
  models: [
    { id: "model-a", provider: "provider", label: "A long model name" },
    { id: "model-b", provider: "provider" },
  ],
  thinkingLevel: "high",
  setModel: jest.fn(),
  setThinkingLevel: jest.fn(),
};
const setSelector = jest.fn();
const props = {
  selector: "model", setSelector, remote, t: (key: string) => key,
} as unknown as ComponentProps<typeof ModelSelectorSheet>;
let tree: ReactTestRenderer;
beforeEach(() => {
  jest.clearAllMocks();
  act(() => { tree = create(createElement(ModelSelectorSheet, props)); });
});
afterEach(() => act(() => tree.unmount()));
const options = () => tree.root.findAll(node => node.props.accessibilityRole === "radio" && node.props.onPress);

test("bottom sheet is bounded, scrollable and has an explicit accessible close action", () => {
  expect(tree.root.findByType(Modal).props.animationType).toBe("slide");
  expect(tree.root.findAllByType(ScrollView)).toHaveLength(1);
  const surface = tree.root.findAllByType(View).find(node => node.props.accessibilityViewIsModal)!;
  expect(StyleSheet.flatten(surface.props.style)).toMatchObject({ maxWidth: 560, maxHeight: "85%" });
  const close = tree.root.findAll(node => node.props.accessibilityLabel === "common.close" && node.props.onPress)[0]!;
  act(() => close.props.onPress());
  expect(setSelector).toHaveBeenCalledWith(null);
});

test("model selection exposes checked state and preserves the provider-qualified identity", () => {
  expect(options()[0]!.props.accessibilityState.checked).toBe(true);
  expect(options()[1]!.props.accessibilityState.checked).toBe(false);
  act(() => options()[1]!.props.onPress());
  expect(remote.setModel).toHaveBeenCalledWith("provider/model-b");
  expect(setSelector).toHaveBeenCalledWith(null);
});

test("model and thinking sheets only expose their own choices", () => {
  expect(options()).toHaveLength(2);
  act(() => options()[1]!.props.onPress());
  expect(remote.setThinkingLevel).not.toHaveBeenCalled();
  remote.setModel.mockClear();
  act(() => tree.update(createElement(ModelSelectorSheet, { ...props, selector: "thinking" })));
  expect(options()).toHaveLength(6);
  act(() => options()[5]!.props.onPress());
  expect(remote.setThinkingLevel).toHaveBeenCalledWith("xhigh");
  expect(remote.setModel).not.toHaveBeenCalled();
});

test("thinking choices remain selectable after redesign", () => {
  act(() => tree.update(createElement(ModelSelectorSheet, { ...props, selector: "thinking" })));
  expect(options()).toHaveLength(6);
  expect(options()[4]!.props.accessibilityState.checked).toBe(true);
  act(() => options()[5]!.props.onPress());
  expect(remote.setThinkingLevel).toHaveBeenCalledWith("xhigh");
});
