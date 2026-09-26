import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { CameraView } from "expo-camera";
import { BackHandler, Modal, ScrollView, StyleSheet, Text, TextInput, View } from "react-native";
import { Button } from "../../components/Button";
import { DialogSurface } from "../../components/DialogSurface";
import { PairingScreen } from "../PairingScreen";
import { pairingCodeFromQr } from "../../remote/codec";
import { RemoteApiError } from "../../remote/connectionState";

let mockDimensions = { width: 402, height: 874, scale: 3, fontScale: 1 };
const mockAppStateHandlers = new Set<(state: string) => void>();
jest.mock("react-native", () => {
  const actual = jest.requireActual("react-native");
  return new Proxy(actual, {
    get: (target, key) => {
      if (key === "useWindowDimensions") return () => mockDimensions;
      // A real AppState only emits from the native side; the fake lets a test
      // drive the foreground transition without spying on (and restoring) the
      // library's own object.
      if (key === "AppState") {
        return {
          addEventListener: (_type: string, callback: (state: string) => void) => {
            mockAppStateHandlers.add(callback);
            return { remove: () => { mockAppStateHandlers.delete(callback); } };
          },
        };
      }
      return Reflect.get(target, key);
    },
  });
});

let mockPermission = { granted: true, canAskAgain: true };
const mockPair = jest.fn();
const mockRequest = jest.fn();
const mockGetPermission = jest.fn();
jest.mock("expo-camera", () => ({
  CameraView: "CameraView",
  useCameraPermissions: () => [mockPermission, mockRequest, mockGetPermission],
}));
jest.mock("../../remote/RemoteContext", () => ({ useRemote: () => ({ pair: mockPair }) }));
jest.mock("../../remote/codec", () => ({ pairingCodeFromQr: jest.fn((code: string) => code) }));
jest.mock("lucide-react-native", () => ({ ArrowLeft: "ArrowLeft", Clipboard: "Clipboard", ScanLine: "ScanLine" }));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }),
}));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
jest.mock("../../i18n/LanguageSettings", () => ({ LanguageSettings: () => null }));

let tree: ReactTestRenderer;
const mockPairingCodeFromQr = pairingCodeFromQr as jest.MockedFunction<typeof pairingCodeFromQr>;
const onBack = jest.fn();
const onPaired = jest.fn();
const button = (label: string) => tree.root.findAll(node =>
  node.props.accessibilityLabel === label && typeof node.props.onPress === "function",
)[0]!;
beforeEach(() => {
  jest.clearAllMocks();
  mockAppStateHandlers.clear();
  mockPermission = { granted: true, canAskAgain: true };
  mockPair.mockResolvedValue(undefined);
  mockDimensions = { width: 402, height: 874, scale: 3, fontScale: 1 };
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
  // The desktop refused the handshake 鈥?the code is spent, used, or describes an
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

const shownText = () =>
  tree.root.findAllByType(Text).map(node => node.props.children).join("|");
const submitManual = async (value: string) => {
  act(() => tree.root.findByType(TextInput).props.onChangeText(value));
  await act(async () => { await tree.root.findByType(TextInput).props.onSubmitEditing(); });
};

// The desktop folds a failure into {message, code, status}; the phone owns the
// mapping from each shape to the one action a user can take.
test.each([
  [new RemoteApiError("rejected", "invalid_pairing_code", 400), "pairing.invalid"],
  [new RemoteApiError("rejected", "invalid_jwt", 401), "pairing.invalid"],
  [new RemoteApiError("rejected", undefined, 403), "pairing.invalid"],
  [new RemoteApiError("rejected", undefined, 429), "pairing.service"],
  [new RemoteApiError("rejected", undefined, 503), "pairing.service"],
  [new RemoteApiError("rejected", undefined, 400), "pairing.failed"],
])("a structured API failure (%#) names the action the user can take", async (error, expected) => {
  mockPair.mockRejectedValueOnce(error);
  act(() => button("pairing.manual").props.onPress());
  await submitManual("some-code");
  expect(shownText()).toContain(expected);
});

test.each([
  ["unexpected_pairing_host", "pairing.host"],
  ["nats_ws_not_tls", "pairing.secureEndpoint"],
  ["desktop_handshake_failed: pairing_identity_mismatch", "pairing.invalid"],
  ["econnrefused", "pairing.network"],
  ["HTTP 502 from the bridge", "pairing.service"],
  // A handshake that fails for an unrecognised reason still means the two
  // builds disagree, which is checked before the transport vocabulary.
  ["desktop_handshake_failed: econnrefused", "pairing.verification"],
  ["who knows", "pairing.failed"],
  ["", "pairing.failed"],
])("a raw transport failure (%s) names the action the user can take", async (message, expected) => {
  mockPair.mockRejectedValueOnce(new Error(message));
  act(() => button("pairing.manual").props.onPress());
  await submitManual("some-code");
  expect(shownText()).toContain(expected);
});

test("scanning a code pairs, and a second frame cannot start a parallel pairing", async () => {
  jest.useFakeTimers();
  try {
    const scanned = () => tree.root.findByType(CameraView).props.onBarcodeScanned({ data: "scanned-code" });
    await act(async () => { await scanned(); });
    expect(mockPair).toHaveBeenCalledWith("scanned-code");
    expect(onPaired).toHaveBeenCalledTimes(1);
    // The camera keeps delivering frames while the request is in flight; the
    // lock must absorb them rather than fire a second pairing.
    await act(async () => { await scanned(); });
    expect(mockPair).toHaveBeenCalledTimes(1);
    act(() => { jest.advanceTimersByTime(1_300); });
    await act(async () => { await scanned(); });
    expect(mockPair).toHaveBeenCalledTimes(2);
  } finally {
    jest.useRealTimers();
  }
});

test("a frame the codec cannot parse is rejected without contacting the desktop", async () => {
  mockPairingCodeFromQr.mockReturnValueOnce("");
  await act(async () => {
    await tree.root.findByType(CameraView).props.onBarcodeScanned({ data: "not a pairing code" });
  });
  expect(mockPair).not.toHaveBeenCalled();
  expect(shownText()).toContain("pairing.invalid");
});

test("the pairing toast clears itself instead of sticking on screen", async () => {
  jest.useFakeTimers();
  try {
    await act(async () => {
      await tree.root.findByType(CameraView).props.onBarcodeScanned({ data: "" });
    });
    expect(shownText()).toContain("pairing.invalid");
    act(() => { jest.advanceTimersByTime(5_000); });
    expect(shownText()).not.toContain("pairing.invalid");
  } finally {
    jest.useRealTimers();
  }
});

test("manual entry ignores blank input and refuses an unparseable code", async () => {
  act(() => button("pairing.manual").props.onPress());
  await submitManual("   ");
  expect(mockPair).not.toHaveBeenCalled();
  mockPairingCodeFromQr.mockReturnValueOnce("");
  await submitManual("not-a-code");
  expect(mockPair).not.toHaveBeenCalled();
  expect(shownText()).toContain("pairing.invalid");
});

test("the manual dialog's own buttons submit and dismiss", async () => {
  act(() => button("pairing.manual").props.onPress());
  act(() => tree.root.findByType(TextInput).props.onChangeText("typed-code"));
  await act(async () => { button("pairing.manualSubmit").props.onPress(); });
  expect(mockPair).toHaveBeenCalledWith("typed-code");
  expect(tree.root.findAllByType(DialogSurface)).toHaveLength(0);
  act(() => button("pairing.manual").props.onPress());
  act(() => button("chat.cancel").props.onPress());
  expect(tree.root.findAllByType(DialogSurface)).toHaveLength(0);
});

test("a short screen keeps the compact manual affordance and its pressed state", () => {
  mockDimensions = { width: 320, height: 480, scale: 2, fontScale: 1.6 };
  act(() => tree.update(createElement(PairingScreen, { onBack, onPaired })));
  const compact = tree.root.findAll(node =>
    node.props.accessibilityLabel === "pairing.manual" && typeof node.props.style === "function");
  expect(compact).toHaveLength(1);
  const idle = StyleSheet.flatten(compact[0]!.props.style({ pressed: false }));
  const pressed = StyleSheet.flatten(compact[0]!.props.style({ pressed: true }));
  expect(idle.backgroundColor).toBeUndefined();
  expect(pressed.backgroundColor).toBeDefined();
});

const emitAppState = (state: string) => {
  act(() => { for (const handler of [...mockAppStateHandlers]) handler(state); });
};

test("returning to the foreground re-checks the camera permission", () => {
  expect(mockAppStateHandlers.size).toBe(1);
  mockGetPermission.mockClear();
  emitAppState("active");
  expect(mockGetPermission).toHaveBeenCalledTimes(1);
  emitAppState("background");
  expect(mockGetPermission).toHaveBeenCalledTimes(1);
});

test("a screen with no back handler registers no hardware-back listener", () => {
  const back = jest.spyOn(BackHandler, "addEventListener")
    .mockReturnValue({ remove: jest.fn() } as never);
  try {
    act(() => tree.update(createElement(PairingScreen, { onPaired })));
    expect(back).not.toHaveBeenCalled();
  } finally {
    back.mockRestore();
  }
});
