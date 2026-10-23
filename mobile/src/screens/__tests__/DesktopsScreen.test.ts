import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Alert, BackHandler, Modal, StyleSheet, Text } from "react-native";
import { Button } from "../../components/Button";
import { peerIconId, PEER_ICONS } from "../../remote/peerIcons";
import type { PairedDesktop } from "../../remote/types";
import { lightColors } from "../../theme/tokens";
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
/** The machine each device page was scoped to, in render order. */
const mockScopes: string[] = [];
jest.mock("../../remote/RemoteContext", () => ({
  useRemoteControls: () => mockRemote,
  // A device page reads the machine it names; the scope itself is the
  // provider's, with its own tests. Here it only has to be reachable, for the
  // right machine, and render what it wraps.
  DesktopScope: ({ desktopId, children }: { desktopId: string; children: unknown }) => {
    mockScopes.push(desktopId);
    return children;
  },
}));
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
// The machine mark is drawn from the shared peer-icon set; these tests are
// about which id each row gets, so the glyph itself is one named stub.
jest.mock("../../remote/PeerIconGlyph", () => ({ PeerIconGlyph: "PeerIconGlyph" }));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }),
}));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

let tree: ReactTestRenderer;
const onBack = jest.fn();
const onAdd = jest.fn();
const onOpenConversation = jest.fn();

/** The device page, as the session list opens it. */
async function render(): Promise<void> {
  await act(async () => {
    tree = create(createElement(DesktopsScreen, { onBack, onAdd, onOpenConversation }));
  });
}

/** The startup picker: the same list with no back action and nothing behind it. */
async function renderPicker(): Promise<void> {
  await act(async () => {
    tree = create(createElement(DesktopsScreen, { onAdd }));
  });
}

function pressables(accessibilityLabel: string) {
  return tree.root.findAll((node) =>
    typeof node.props.onPress === "function" && node.props.accessibilityLabel === accessibilityLabel,
  );
}

/** A machine's row, by the name drawn on it. A plain `View` draws its props
 * through to a host view, so a row can match twice — the outer match, first in
 * the tree, is the one carrying the state. */
const row = (name: string) =>
  tree.root.findAll((node) => node.props.accessibilityLabel === name)[0]!;

/** The rows that are a control: the startup picker's, and only those. */
const rowControls = (name: string) =>
  tree.root.findAll((node) =>
    typeof node.props.onPress === "function" && node.props.accessibilityLabel === name,
  );

const button = (label: string) =>
  tree.root.findAllByType(Button).find((node) => node.props.label === label)!;

const texts = () => tree.root.findAllByType(Text).map((node) => node.props.children).flat().filter(
  (child): child is string => typeof child === "string",
);

/**
 * The machine mark each card draws, in row order.
 *
 * Resolved through `peerIconId`, because that is what the glyph draws: a card
 * with no icon of its own is handed `undefined` and the default is what appears.
 */
const glyphIds = () => tree.root.findAll(node => String(node.type) === "PeerIconGlyph").map(node => peerIconId(node.props.id));

/** The device page the list opened, if any. */
const devicePage = () => tree.root.findAllByType("SettingsScreen" as never)[0];

beforeEach(async () => {
  jest.clearAllMocks();
  mockScopes.length = 0;
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

/**
 * Which machine the phone is reading is not drawn on this list. The mark read
 * as a third control on a row whose two controls are the actions beside it, and
 * the list is a description of the machines (see the screen's own note). Which
 * machine a new conversation belongs to is asked for where one is created, and
 * assistive tech still gets the machine in use here.
 */
test("draws the machine in use plainly, saying which it is only to assistive tech", () => {
  expect(row("desktop-1").props.accessibilityState.selected).toBe(true);
  expect(row("desktop-2").props.accessibilityState.selected).toBe(false);
  expect(tree.root.findAllByType("Check" as never)).toHaveLength(0);
  // The highlight is gone with the mark: no row is painted as the current one.
  expect(tree.root.findAll(
    (node) => StyleSheet.flatten(node.props.style)?.backgroundColor === lightColors.accentSoft,
  )).toHaveLength(0);
});

/**
 * The page is about the machines, not a way to switch between them. A tap on a
 * row used to switch the active desktop and leave for the session list: a move
 * nobody asked for on a page opened to look at the machines, and one that threw
 * the page away with it. Switching is the session list's device picker.
 */
test("a machine's row is not a control on the device page", () => {
  expect(rowControls("desktop-1")).toHaveLength(0);
  expect(rowControls("desktop-2")).toHaveLength(0);
  expect(row("desktop-2").props.onPress).toBeUndefined();
  expect(mockRemote.switchDesktop).not.toHaveBeenCalled();
  expect(onBack).not.toHaveBeenCalled();
});

/**
 * The list has one action per machine that opens everything about it — what to
 * call it here, and every setting that is read from or written to it. It is the
 * only way in: the phone's own settings screen has no device rows at all.
 */
test("the configure action opens that desktop's own page, and only when asked", async () => {
  expect(devicePage()).toBeUndefined();
  // Nothing on this list reads a machine's settings, and no page exists to read
  // them for.
  expect(mockScopes).toHaveLength(0);
  act(() => pressables("desktops.settings")[1]!.props.onPress());
  const page = devicePage()!;
  expect(page.props.deviceId).toBe("desktop-2");
  expect(page.props.onOpenConversation).toBe(onOpenConversation);
  // The page reads the machine it was opened for — not the one in use.
  expect(mockScopes).toContain("desktop-2");
  expect(mockScopes).not.toContain("desktop-1");
  // Nothing was written just by opening it: the name and icon live in the page.
  act(() => page.props.onClose());
  expect(devicePage()).toBeUndefined();
  expect(mockRemote.switchDesktop).not.toHaveBeenCalled();
});

test("the configure action is the row's only tap, and it stays reachable", async () => {
  // The gear sits on the same row as a control that no longer exists: what is
  // left has to still be findable and still be the only thing that opens it.
  expect(pressables("desktops.settings")).toHaveLength(2);
  act(() => pressables("desktops.settings")[0]!.props.onPress());
  expect(devicePage()!.props.deviceId).toBe("desktop-1");
  expect(mockRemote.switchDesktop).not.toHaveBeenCalled();
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

/**
 * The startup picker is the same list with nothing behind it, and there the row
 * *is* the choice: with no session list to go back to, a device page that only
 * described the machines would leave the phone with no way to pick one.
 */
test("a startup picker still lets the row choose the machine", async () => {
  act(() => tree.unmount());
  await renderPicker();
  expect(pressables("common.back")).toHaveLength(0);
  await act(async () => rowControls("desktop-2")[0]!.props.onPress());
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

test("keeps the startup picker open and displays a failed switch", async () => {
  act(() => tree.unmount());
  await renderPicker();
  mockRemote.switchDesktop.mockRejectedValueOnce(new Error("storage unavailable"));
  await act(async () => rowControls("desktop-2")[0]!.props.onPress());
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
  // The mark is drawn, not typed: the ids say which picture each card got.
  expect(glyphIds()).toEqual(["rocket", PEER_ICONS[0]!.id]);
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
    await renderPicker();
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

/**
 * The device page is a full-screen surface, so the system's own two ways out of
 * it are the dismissal and a back request. Both have to be routed: a page left
 * mounted behind a list that thinks it is closed keeps writing to the machine.
 */
test("the device page closes on a dismissal, and back leaves it to the page", async () => {
  const surface = () =>
    tree.root.findAllByType(Modal).find(node => node.props.presentationStyle === "fullScreen")!;
  expect(surface().props.visible).toBe(false);

  act(() => pressables("desktops.settings")[1]!.props.onPress());
  expect(surface().props.visible).toBe(true);
  expect(devicePage()).toBeDefined();

  // A back request is the page's own stack to answer — it has levels of its own,
  // and only the surface closes when it runs out of them.
  act(() => surface().props.onRequestClose());

  act(() => surface().props.onDismiss());
  expect(surface().props.visible).toBe(false);
  expect(devicePage()).toBeUndefined();
});
