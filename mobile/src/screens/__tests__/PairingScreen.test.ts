import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { CameraView } from "expo-camera";
import { Modal, ScrollView, StyleSheet, TextInput, View } from "react-native";
import { Button } from "../../components/Button";
import { DialogSurface } from "../../components/DialogSurface";
import { PairingScreen } from "../PairingScreen";

let mockPermission = { granted: true, canAskAgain: true };
const mockPair = jest.fn();
const mockRequest = jest.fn();
const mockGetPermission = jest.fn();
jest.mock("expo-camera", () => ({
  CameraView: "CameraView",
  useCameraPermissions: () => [mockPermission, mockRequest, mockGetPermission],
}));
jest.mock("../../remote/RemoteContext", () => ({ useRemote: () => ({ pair: mockPair }) }));
jest.mock("../../remote/codec", () => ({ pairingCodeFromQr: (code: string) => code }));
jest.mock("lucide-react-native", () => ({ ArrowLeft: "ArrowLeft", Clipboard: "Clipboard", ScanLine: "ScanLine" }));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }),
}));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
jest.mock("../../i18n/LanguageSettings", () => ({ LanguageSettings: () => null }));

let tree: ReactTestRenderer;
const onBack = jest.fn();
const onPaired = jest.fn();
const button = (label: string) => tree.root.findAll(node =>
  node.props.accessibilityLabel === label && typeof node.props.onPress === "function",
)[0]!;
beforeEach(() => {
  jest.clearAllMocks();
  mockPermission = { granted: true, canAskAgain: true };
  mockPair.mockResolvedValue(undefined);
  act(() => { tree = create(createElement(PairingScreen, { onBack, onPaired })); });
});
afterEach(() => act(() => tree.unmount()));

test("pairing is scrollable on short screens and back remains outside the scroll area", () => {
  const scroll = tree.root.findByType(ScrollView);
  expect(StyleSheet.flatten(scroll.props.contentContainerStyle).flexGrow).toBe(1);
  expect(scroll.findAll(node => node.props.accessibilityLabel === "common.back")).toHaveLength(0);
  act(() => button("common.back").props.onPress());
  expect(onBack).toHaveBeenCalledTimes(1);
});

test("manual entry pauses QR scanning and resumes it after dismissing", () => {
  expect(tree.root.findByType(CameraView).props.onBarcodeScanned).toEqual(expect.any(Function));
  act(() => button("pairing.manual").props.onPress());
  expect(tree.root.findByType(CameraView).props.onBarcodeScanned).toBeUndefined();
  expect(tree.root.findAllByType(DialogSurface)).toHaveLength(1);
  act(() => tree.root.findByType(Modal).props.onRequestClose());
  expect(tree.root.findByType(CameraView).props.onBarcodeScanned).toEqual(expect.any(Function));
});

test("manual entry submits from the keyboard without adding another confirmation step", async () => {
  act(() => button("pairing.manual").props.onPress());
  act(() => tree.root.findByType(TextInput).props.onChangeText("  valid-code  "));
  await act(async () => tree.root.findByType(TextInput).props.onSubmitEditing());
  expect(mockPair).toHaveBeenCalledWith("valid-code");
  expect(onPaired).toHaveBeenCalledTimes(1);
});

test("permission instructions can grow instead of being clipped inside a square camera frame", () => {
  mockPermission = { granted: false, canAskAgain: true };
  act(() => tree.update(createElement(PairingScreen, { onBack, onPaired })));
  const scanner = tree.root.findAllByType(View).find(node =>
    StyleSheet.flatten(node.props.style)?.minHeight === 280,
  )!;
  expect(StyleSheet.flatten(scanner.props.style).aspectRatio).toBeUndefined();
  const permissionButton = tree.root.findAllByType(Button).find(node => node.props.label === "pairing.continue")!;
  act(() => permissionButton.props.onPress());
  expect(mockRequest).toHaveBeenCalledTimes(1);
});
