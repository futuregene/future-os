import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, StyleSheet, View } from "react-native";
import { ConnectionBadge } from "../../components/ConnectionBadge";
import { Button } from "../../components/Button";
import { DialogSurface } from "../../components/DialogSurface";
import { SessionsScreen } from "../SessionsScreen";

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
};
jest.mock("react-native", () => {
  const actual = jest.requireActual("react-native");
  return new Proxy(actual, {
    get: (target, key) => key === "useWindowDimensions" ? () => mockDimensions : Reflect.get(target, key),
  });
});
jest.mock("../../remote/RemoteContext", () => ({ useRemote: () => mockRemote }));
jest.mock("../SessionList", () => ({ SessionList: () => null }));
jest.mock("../../update/prompt", () => ({ promptUpgrade: jest.fn() }));
jest.mock("../../update/update", () => ({ checkForUpdate: jest.fn() }));
jest.mock("lucide-react-native", () => Object.fromEntries(
  ["ChevronDown", "Folder", "LogOut", "MessageCircle", "Monitor", "Plus", "Settings", "Unplug", "X"].map(name => [name, name]),
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

test("tabs get their own full-width row instead of competing with header actions", () => {
  const tabs = tree.root.findAll(node => node.props.accessibilityRole === "tab" && node.props.onPress);
  expect(tabs).toHaveLength(2);
  for (const tab of tabs) {
    expect(StyleSheet.flatten(tab.props.style)).toMatchObject({ flex: 1, minHeight: 44, minWidth: 0 });
  }
  expect(tabs[0]!.parent!.findAllByType(ConnectionBadge)).toHaveLength(0);
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

test("new chat opens immediately, without an extra creation dialog", () => {
  const chatTab = tree.root.findAll(node => node.props.accessibilityRole === "tab" && node.props.onPress)[1]!;
  act(() => chatTab.props.onPress());
  act(() => button("sessions.new").props.onPress());
  expect(mockRemote.newConversation).toHaveBeenCalledWith("chat");
});
