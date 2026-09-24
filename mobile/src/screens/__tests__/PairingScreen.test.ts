import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { CameraView } from "expo-camera";
import { BackHandler, Modal, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
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

test("pairing is scrollable on short screens and back shares the title row", () => {
  const scroll = tree.root.findByType(ScrollView);
  expect(StyleSheet.flatten(scroll.props.contentContainerStyle).flexGrow).toBe(1);
  expect(
    scroll.findAll(node =>
      node.props.accessibilityLabel === "common.back" && typeof node.props.onPress === "function",
    ),
  ).toHaveLength(1);
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
  expect(onBack).not.toHaveBeenCalled();
});

test("system back from pairing returns to its parent", () => {
  act(() => tree.unmount());
  const subscribe = jest.spyOn(BackHandler, "addEventListener").mockReturnValue({ remove: jest.fn() });
  try {
    act(() => { tree = create(createElement(PairingScreen, { onBack, onPaired })); });
    const handler = subscribe.mock.calls.at(-1)![1];
    act(() => { expect(handler({ type: "hardwareBackPress", timeStamp: 1 })).toBe(true); });
    expect(onBack).toHaveBeenCalledTimes(1);
    expect(onPaired).not.toHaveBeenCalled();
  } finally { subscribe.mockRestore(); }
});

test("manual entry submits from the keyboard without adding another confirmation step", async () => {
  act(() => button("pairing.manual").props.onPress());
  act(() => tree.root.findByType(TextInput).props.onChangeText("  valid-code  "));
  await act(async () => tree.root.findByType(TextInput).props.onSubmitEditing());
  expect(mockPair).toHaveBeenCalledWith("valid-code");
  expect(onPaired).toHaveBeenCalledTimes(1);
});

// The desktop folds every handshake failure into one opaque reply, so the phone
// is the only place a user (or a support log) can be told what to actually do.
// These three outcomes need three different actions, and used to share one
// "update both apps" message.
test("a pairing failure names the action the user can take", async () => {
  const toast = () => tree.root.findAllByType(Text).map(node => node.props.children).join("|");
  const pairFailing = async (error: Error) => {
    mockPair.mockRejectedValueOnce(error);
    act(() => button("pairing.manual").props.onPress());
    act(() => tree.root.findByType(TextInput).props.onChangeText("some-code"));
    await act(async () => tree.root.findByType(TextInput).props.onSubmitEditing());
    return toast();
  };

  // The desktop never answered: a network problem, not a version problem.
  expect(await pairFailing(new Error("desktop_handshake_failed: TIMEOUT"))).toContain(
    "pairing.network",
  );
  // The desktop refused the handshake — the code is spent, used, or describes an
  // identity the bridge no longer serves. Recovery is a fresh code.
  expect(
    await pairFailing(
      new Error(
        "desktop_handshake_failed: pairing_signature_invalid:remote_secure_channel_invalid (invitation_expired)",
      ),
    ),
  ).toContain("pairing.invalid");
  // The confirmation did not verify: only this one means the builds disagree.
  expect(await pairFailing(new Error("desktop_handshake_failed: pairing_confirmation_mismatch"))).toContain(
    "pairing.verification",
  );
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
