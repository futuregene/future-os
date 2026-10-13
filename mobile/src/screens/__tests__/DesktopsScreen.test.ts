import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Alert, BackHandler, Modal, Text } from "react-native";
import { Button } from "../../components/Button";
import { PEER_ICONS } from "../../remote/peerIcons";
import type { PairedDesktop } from "../../remote/types";
import { DesktopsScreen } from "../DesktopsScreen";

const mockRemote: {
  credentials: { pairId: string };
  desktops: PairedDesktop[];
  switchDesktop: jest.Mock;
  removeDesktop: jest.Mock;
} = {
  credentials: { pairId: "pair-1" },
  desktops: [
    { desktopId: "desktop-1", pairId: "pair-1" },
    { desktopId: "desktop-2", pairId: "pair-2" },
  ],
  switchDesktop: jest.fn(),
  removeDesktop: jest.fn(),
};
jest.mock("../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote }));
// The device page is a stack of its own with its own tests; here it only has to
// be reachable, for the right machine, and closable.
jest.mock("../../features/settings/SettingsScreen", () => ({ SettingsScreen: "SettingsScreen" }));
jest.mock("lucide-react-native", () => ({
  ArrowLeft: "ArrowLeft",
  Plus: "Plus",
  Check: "Check",
  Settings2: "Settings2",
  Trash2: "Trash2",
}));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }),
}));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

let tree: ReactTestRenderer;
const onBack = jest.fn();
const onAdd = jest.fn();
const onOpenConversation = jest.fn();

async function render(): Promise<void> {
  await act(async () => {
    tree = create(createElement(DesktopsScreen, { onBack, onAdd, onOpenConversation }));
  });
}

function pressables(accessibilityLabel: string) {
  return tree.root.findAll((node) =>
    typeof node.props.onPress === "function" && node.props.accessibilityLabel === accessibilityLabel,
  );
}

const selectors = () =>
  tree.root.findAll((node) =>
    typeof node.props.onPress === "function" && node.props.accessibilityState?.selected !== undefined,
  );

const button = (label: string) =>
  tree.root.findAllByType(Button).find((node) => node.props.label === label)!;

const texts = () => tree.root.findAllByType(Text).map((node) => node.props.children).flat().filter(
  (child): child is string => typeof child === "string",
);

/** The device page the list opened, if any. */
const devicePage = () => tree.root.findAllByType("SettingsScreen" as never)[0];

beforeEach(async () => {
  jest.clearAllMocks();
  mockRemote.credentials = { pairId: "pair-1" };
  mockRemote.desktops = [
    { desktopId: "desktop-1", pairId: "pair-1" },
    { desktopId: "desktop-2", pairId: "pair-2" },
  ];
  mockRemote.switchDesktop.mockResolvedValue(undefined);
  mockRemote.removeDesktop.mockResolvedValue(undefined);
  jest.spyOn(Alert, "alert").mockImplementation(() => {});
  await render();
});
afterEach(() => act(() => tree.unmount()));

test("lists both desktops and marks the current one", () => {
  expect(selectors()).toHaveLength(2);
  expect(selectors()[0]!.props.accessibilityState.selected).toBe(true);
  expect(selectors()[1]!.props.accessibilityState.selected).toBe(false);
});

test("selects a saved desktop without scanning again", async () => {
  await act(async () => selectors()[1]!.props.onPress());
  expect(mockRemote.switchDesktop).toHaveBeenCalledWith("desktop-2");
  expect(onBack).toHaveBeenCalled();
});

/**
 * The list has one action per machine that opens everything about it — what to
 * call it here, and every setting that is read from or written to it. It is the
 * only way in: the phone's own settings screen has no device rows at all.
 */
test("the configure action opens that desktop's own page, and only when asked", async () => {
  expect(devicePage()).toBeUndefined();
  act(() => pressables("desktops.settings")[1]!.props.onPress());
  const page = devicePage()!;
  expect(page.props.deviceId).toBe("desktop-2");
  expect(page.props.onOpenConversation).toBe(onOpenConversation);
  // Nothing was written just by opening it: the name and icon live in the page.
  act(() => page.props.onClose());
  expect(devicePage()).toBeUndefined();
  expect(mockRemote.switchDesktop).not.toHaveBeenCalled();
});

test("a whole card tap still connects, even with the device page shut", async () => {
  // Two controls per row, one choice: opening the page must not have made the
  // row's own action (connect this machine) ambiguous or unreachable.
  await act(async () => selectors()[1]!.props.onPress());
  expect(mockRemote.switchDesktop).toHaveBeenCalledWith("desktop-2");
  expect(devicePage()).toBeUndefined();
});

test("system back returns from device management", async () => {
  act(() => tree.unmount());
  const subscribe = jest.spyOn(BackHandler, "addEventListener").mockReturnValue({ remove: jest.fn() });
  try {
    await render();
    const handler = subscribe.mock.calls.at(-1)![1];
    act(() => { expect(handler({ type: "hardwareBackPress", timeStamp: 1 })).toBe(true); });
    expect(onBack).toHaveBeenCalledTimes(1);
  } finally { subscribe.mockRestore(); }
});

test("supports a startup picker without a back action", async () => {
  act(() => tree.unmount());
  await act(async () => {
    tree = create(createElement(DesktopsScreen, { onAdd }));
  });
  expect(pressables("common.back")).toHaveLength(0);
  await act(async () => selectors()[1]!.props.onPress());
  expect(mockRemote.switchDesktop).toHaveBeenCalledWith("desktop-2");
  expect(onBack).not.toHaveBeenCalled();
});

test("keeps the list in a centered column with side padding", () => {
  const column = tree.root.findAll(
    (node) => node.props.style?.alignSelf === "center" && node.props.style?.maxWidth > 0,
  )[0]!;
  expect(column.props.style.alignSelf).toBe("center");
  expect(column.props.style.paddingHorizontal).toBeGreaterThan(0);
});

test("opens the add-by-QR flow", () => {
  act(() => button("desktops.add").props.onPress());
  expect(onAdd).toHaveBeenCalled();
});

test("keeps the picker open and displays a failed switch", async () => {
  mockRemote.switchDesktop.mockRejectedValueOnce(new Error("storage unavailable"));
  await act(async () => selectors()[1]!.props.onPress());
  expect(onBack).not.toHaveBeenCalled();
  expect(tree.root.findAll((node) => node.props.accessibilityRole === "alert").length).toBeGreaterThan(0);
});

test("falls back to the desktop id until the user names it", async () => {
  expect(texts()).toEqual(expect.arrayContaining(["desktop-1", "desktop-2"]));
});

/**
 * The icon is picked on the device page and drawn on this list, and those are
 * two different files: storing an id that nothing reads back looks exactly like
 * a picker that works. Every card used to draw the same fixed monitor glyph, so
 * this screen was the one place the choice had no visible effect.
 */
test("draws each desktop with the icon the user gave it, and the default without one", async () => {
  act(() => tree.unmount());
  mockRemote.desktops = [
    { desktopId: "desktop-1", pairId: "pair-1", icon: "rocket" },
    { desktopId: "desktop-2", pairId: "pair-2" },
  ];
  await render();
  expect(texts()).toEqual(expect.arrayContaining(["🚀", PEER_ICONS[0]!.glyph]));
});

test("shows the chosen name above the id", async () => {
  mockRemote.desktops = [
    { desktopId: "desktop-1", pairId: "pair-1", name: "Studio Mac" },
    { desktopId: "desktop-2", pairId: "pair-2" },
  ];
  await render();
  expect(texts()).toEqual(expect.arrayContaining(["Studio Mac", "desktop-1", "desktop-2"]));
});

test("a startup picker lets the system own the back gesture", async () => {
  act(() => tree.unmount());
  const subscribe = jest.spyOn(BackHandler, "addEventListener").mockReturnValue({ remove: jest.fn() });
  try {
    await act(async () => {
      tree = create(createElement(DesktopsScreen, { onAdd }));
    });
    const handler = subscribe.mock.calls.at(-1)![1];
    // Nothing to go back to: reporting "handled" would swallow the OS back
    // gesture and leave the user stuck on the picker.
    expect(handler({ type: "hardwareBackPress", timeStamp: 1 })).toBe(false);
    expect(onBack).not.toHaveBeenCalled();
  } finally { subscribe.mockRestore(); }
});

test("confirms removal of only the chosen desktop", async () => {
  act(() => pressables("sessions.unpair")[1]!.props.onPress());
  const modal = tree.root.findAllByType(Modal).find(node => node.props.visible)!;
  act(() => modal.findAllByType(Button).find(node => node.props.variant === "danger")!.props.onPress());
  expect(mockRemote.removeDesktop).not.toHaveBeenCalled();
  await act(async () => { modal.props.onDismiss(); });
  expect(mockRemote.removeDesktop).toHaveBeenCalledWith("desktop-2");
});

test("a refused removal is reported instead of silently leaving the desktop paired", async () => {
  mockRemote.removeDesktop.mockRejectedValueOnce(new Error("bridge unreachable"));
  act(() => pressables("sessions.unpair")[1]!.props.onPress());
  const modal = tree.root.findAllByType(Modal).find(node => node.props.visible)!;
  act(() => modal.findAllByType(Button).find(node => node.props.variant === "danger")!.props.onPress());
  await act(async () => { modal.props.onDismiss(); });
  expect(mockRemote.removeDesktop).toHaveBeenCalledWith("desktop-2");
  expect(tree.root.findAll(node => node.props.accessibilityRole === "alert").length).toBeGreaterThan(0);
  expect(texts()).toContain("desktops.actionFailed");
});
