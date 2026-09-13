import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, StyleSheet, View } from "react-native";
import { ConnectionBadge } from "../../components/ConnectionBadge";
import { Button } from "../../components/Button";
import { DialogSurface } from "../../components/DialogSurface";
import { SessionsScreen } from "../SessionsScreen";
import { SessionList } from "../SessionList";
import { ActionMenu } from "../../components/ActionMenu";

let mockDimensions = { width: 320, height: 640, scale: 1, fontScale: 1 };
const mockRemote = {
  desktops: [{ pairId: "p1", name: "A very long desktop name" }],
  credentials: { pairId: "p1" },
  workspaces: [],
  connectionPresentation: { level: "connected", titleKey: "connection.connected" },
  desktopOnline: true,
  sandboxAvailable: true,
  approvalTier: "manual",
  newConversation: jest.fn(),
  reconnect: jest.fn(),
  setSessionPinned: jest.fn(async () => {}),
};
jest.mock("react-native", () => {
  const actual = jest.requireActual("react-native");
  return new Proxy(actual, {
    get: (target, key) => key === "useWindowDimensions" ? () => mockDimensions : Reflect.get(target, key),
  });
});
jest.mock("../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote }));
jest.mock("../SessionList", () => ({ SessionList: () => null }));
jest.mock("../../i18n/LanguageSettings", () => ({ LanguageSettings: () => null }));
jest.mock("../../update/prompt", () => ({ promptUpgrade: jest.fn() }));
jest.mock("../../update/update", () => ({ checkForUpdate: jest.fn() }));
jest.mock("lucide-react-native", () => Object.fromEntries(
  ["ChevronDown", "Folder", "LogOut", "MessageCircle", "Monitor", "Plus", "Pin", "Pencil", "Trash2", "Settings", "Unplug", "X"].map(name => [name, name]),
));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }),
}));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));

let tree: ReactTestRenderer;
const onManageDesktops = jest.fn();
const button = (label: string) => tree.root.findAll(node =>
  node.props.accessibilityLabel === label && typeof node.props.onPress === "function",
)[0]!;
beforeEach(() => {
  jest.clearAllMocks();
  mockDimensions = { width: 320, height: 640, scale: 1, fontScale: 1 };
  act(() => { tree = create(createElement(SessionsScreen, { onManageDesktops })); });
});
afterEach(() => act(() => tree.unmount()));

test("device header has real gutters and a non-shrinking touch target on a small phone", () => {
  const selector = button("desktops.title");
  const style = StyleSheet.flatten(selector.props.style({ pressed: false }));
  expect(style.minHeight).toBeGreaterThanOrEqual(44);
  expect(style.minWidth).toBe(0);
  const header = selector.parent!;
  expect(StyleSheet.flatten(header.props.style).paddingHorizontal).toBe(16);
  expect(tree.root.findByType(ConnectionBadge).props.compact).toBe(true);
  act(() => selector.props.onPress());
  expect(onManageDesktops).toHaveBeenCalledTimes(1);
});

test("category switching is delegated to the list toolbar, without a separate tab row", () => {
  expect(tree.root.findAll(node => node.props.accessibilityRole === "tab")).toHaveLength(0);
  act(() => tree.root.findByType(SessionList).props.onTabChange("workspace"));
  expect(tree.root.findByType(SessionList).props.tab).toBe("workspace");
  act(() => tree.root.findByType(SessionList).props.onTabChange("chat"));
  expect(tree.root.findByType(SessionList).props.tab).toBe("chat");
});

test("tablet layout is centered and large text keeps the device badge compact", () => {
  mockDimensions = { width: 820, height: 1180, scale: 1, fontScale: 1 };
  act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
  expect(tree.root.findByType(ConnectionBadge).props.compact).toBe(false);
  expect(tree.root.findAllByType(View).some(node => {
    const style = StyleSheet.flatten(node.props.style);
    return style?.maxWidth === 760 && style.alignSelf === "center";
  })).toBe(true);
  mockDimensions = { ...mockDimensions, fontScale: 1.6 };
  act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
  expect(tree.root.findByType(ConnectionBadge).props.compact).toBe(true);
});

test("settings uses the keyboard-safe scrollable dialog and can be dismissed", () => {
  act(() => button("sessions.settings").props.onPress());
  const modal = tree.root.findAllByType(Modal).find(node => node.props.visible)!;
  expect(modal.findAllByType(DialogSurface)).toHaveLength(1);
  act(() => modal.props.onRequestClose());
  expect(tree.root.findAllByType(Modal).some(node => node.props.visible)).toBe(false);
});

test("iOS device navigation waits until the settings modal has dismissed", () => {
  act(() => button("sessions.settings").props.onPress());
  const modal = tree.root.findAllByType(Modal).find(node => node.props.visible)!;
  const manage = modal.findAllByType(Button).find(node => node.props.label === "desktops.title")!;
  act(() => manage.props.onPress());
  expect(onManageDesktops).not.toHaveBeenCalled();
  act(() => modal.props.onDismiss());
  expect(onManageDesktops).toHaveBeenCalledTimes(1);
  act(() => modal.props.onDismiss());
  expect(onManageDesktops).toHaveBeenCalledTimes(1);
});

test("session ellipsis and long-press callback use the shared app action sheet", async () => {
  const session = { sessionId: "s", threadId: "t", title: "Reply", streaming: false };
  act(() => tree.root.findByType(SessionList).props.onMenu(session));
  const menu = tree.root.findByType(ActionMenu);
  expect(menu.props.visible).toBe(true);
  expect(menu.props.actions.map((action: { label: string }) => action.label)).toEqual(["sessions.pin", "chat.rename", "sessions.delete"]);
  const modal = menu.findByType(Modal);
  act(() => button("sessions.pin").props.onPress());
  expect(mockRemote.setSessionPinned).not.toHaveBeenCalled();
  await act(async () => { modal.props.onDismiss(); });
  expect(mockRemote.setSessionPinned).toHaveBeenCalledWith("s", "t", true);
});

test("new chat opens immediately, without an extra creation dialog", () => {
  act(() => tree.root.findByType(SessionList).props.onTabChange("chat"));
  act(() => button("sessions.new").props.onPress());
  expect(mockRemote.newConversation).toHaveBeenCalledWith("chat");
});
