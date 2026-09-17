import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Modal, ScrollView, StyleSheet, Text, View } from "react-native";
import { ConnectionBadge } from "../../components/ConnectionBadge";
import { Button } from "../../components/Button";
import { DialogSurface } from "../../components/DialogSurface";
import { SessionsScreen } from "../SessionsScreen";
import { SessionList } from "../SessionList";
import { ActionMenu } from "../../components/ActionMenu";
import { SettingsLink } from "../../features/settings/SettingsPrimitives";
import { checkForUpdate } from "../../update/update";

let mockDimensions = { width: 320, height: 640, scale: 1, fontScale: 1 };
const mockRemote = {
  desktops: [{ pairId: "p1", name: "A very long desktop name" }],
  credentials: { pairId: "p1" },
  workspaces: [] as { id: string; name: string }[],
  connectionPresentation: { level: "connected", titleKey: "connection.connected" },
  desktopOnline: true,
  capabilities: new Set(["desktop_settings_v1", "skill_management_v1"]),
  desktopSettingsRevision: 0,
  skillsRevision: 0,
  getDesktopSettings: jest.fn(async () => ({ autoUpgradeSkills: true, autoTitleFirstTurn: true, autoConnectRemote: true, hiddenModels: [] })),
  listSettingsModels: jest.fn(async () => []),
  listInstalledSkills: jest.fn(async () => []),
  listAvailableSkills: jest.fn(async () => []),
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
  ["ArrowLeft", "ChevronLeft", "ChevronRight", "ChevronDown", "Folder", "LogOut", "MessageCircle", "Monitor", "Plus", "Pin", "Pencil", "Trash2", "Settings", "Unplug", "X"].map(name => [name, name]),
));
jest.mock("react-native-safe-area-context", () => ({
  SafeAreaView: "SafeAreaView",
  useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }),
}));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key, i18n: { language: "en" } }) }));

let tree: ReactTestRenderer;
const onManageDesktops = jest.fn();
const button = (label: string) => tree.root.findAll(node =>
  node.props.accessibilityLabel === label && typeof node.props.onPress === "function",
)[0]!;
beforeEach(() => {
  jest.clearAllMocks();
  mockDimensions = { width: 320, height: 640, scale: 1, fontScale: 1 };
  mockRemote.workspaces = [];
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
  const badge = tree.root.findByType(ConnectionBadge);
  expect(badge.findAllByType(Text)).toHaveLength(0);
  expect(badge.props.active).toBe(true);
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

test("tablet and large-text layouts keep the status dot-only", () => {
  mockDimensions = { width: 820, height: 1180, scale: 1, fontScale: 1 };
  act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
  expect(tree.root.findByType(ConnectionBadge).findAllByType(Text)).toHaveLength(0);
  expect(tree.root.findAllByType(View).some(node => {
    const style = StyleSheet.flatten(node.props.style);
    return style?.maxWidth === 760 && style.alignSelf === "center";
  })).toBe(true);
  mockDimensions = { ...mockDimensions, fontScale: 1.6 };
  act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
  expect(tree.root.findByType(ConnectionBadge).findAllByType(Text)).toHaveLength(0);
});

test("settings uses a full-screen scrollable page and can be dismissed", () => {
  act(() => button("sessions.settings").props.onPress());
  const modal = tree.root.findAllByType(Modal).find(node => node.props.visible)!;
  expect(modal.props.presentationStyle).toBe("fullScreen");
  expect(modal.findAllByType(ScrollView).length).toBeGreaterThan(0);
  act(() => modal.props.onRequestClose());
  expect(tree.root.findAllByType(Modal).some(node => node.props.visible)).toBe(false);
});

test.each(["desktopSettings.preferences", "desktopSettings.models", "desktopSettings.skills", "language.title"])(
  "system back from %s returns to settings before closing the modal", async label => {
    await act(async () => button("sessions.settings").props.onPress());
    const modal = () => tree.root.findAllByType(Modal).find(node => node.props.visible)!;
    const link = () => modal().findAllByType(SettingsLink).find(node => node.props.label === label);
    await act(async () => link()!.props.onPress());
    expect(link()).toBeUndefined();
    await act(async () => modal().props.onRequestClose());
    expect(link()).toBeDefined();
    expect(onManageDesktops).not.toHaveBeenCalled();
    act(() => modal().props.onRequestClose());
    expect(tree.root.findAllByType(Modal).some(node => node.props.visible)).toBe(false);
    await act(async () => button("sessions.settings").props.onPress());
    expect(link()).toBeDefined();
  },
);

test("iOS update checking waits until the settings modal has dismissed", async () => {
  jest.mocked(checkForUpdate).mockRejectedValueOnce(new Error("offline"));
  act(() => button("sessions.settings").props.onPress());
  const modal = tree.root.findAllByType(Modal).find(node => node.props.visible)!;
  const links = modal.findAllByType(SettingsLink);
  expect(links.some(node => node.props.label === "desktops.title" || node.props.label === "sessions.unpair")).toBe(false);
  const update = links.find(node => node.props.label === "update.check")!;
  act(() => update.props.onPress());
  expect(checkForUpdate).not.toHaveBeenCalled();
  await act(async () => modal.props.onDismiss());
  expect(checkForUpdate).toHaveBeenCalledTimes(1);
  act(() => modal.props.onDismiss());
  expect(checkForUpdate).toHaveBeenCalledTimes(1);
  expect(onManageDesktops).not.toHaveBeenCalled();
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

test.each([
  { width: 320, height: 480, scale: 1, fontScale: 1.6 },
  { width: 820, height: 1180, scale: 1, fontScale: 1 },
])("new workspace list uses the dialog viewport instead of a nested 180-point scroll (%s)", dimensions => {
  mockDimensions = dimensions;
  mockRemote.workspaces = Array.from({ length: 25 }, (_, index) => ({ id: `w${index}`, name: `Workspace ${index}` }));
  act(() => tree.root.findByType(SessionList).props.onTabChange("workspace"));
  act(() => button("sessions.new").props.onPress());
  const modal = tree.root.findAllByType(Modal).find(node => node.props.visible)!;
  const surface = modal.findByType(DialogSurface);
  expect(surface.props.footer).toBeDefined();
  expect(surface.findAllByType(ScrollView)).toHaveLength(1);
  const scroll = surface.findByType(ScrollView);
  expect(scroll.findAllByType(Button)).toHaveLength(0);
  expect(surface.findAllByType(View).some(node => StyleSheet.flatten(node.props.style)?.maxHeight === 180)).toBe(false);
  const names = scroll.findAllByType(Text).filter(node => typeof node.props.children === "string" && node.props.children.startsWith("Workspace "));
  expect(names).toHaveLength(25);
  expect(names.every(node => node.props.numberOfLines === 1)).toBe(true);
  const radio = scroll.findAll(node => node.props.accessibilityRole === "radio" && node.props.onPress);
  act(() => radio[radio.length - 1]!.props.onPress());
  act(() => surface.findByType(Button).props.onPress());
  expect(mockRemote.newConversation).not.toHaveBeenCalled();
  act(() => modal.props.onDismiss());
  expect(mockRemote.newConversation).toHaveBeenCalledWith("workspace", "w24");
});

test("empty workspace list keeps creation disabled", () => {
  act(() => tree.root.findByType(SessionList).props.onTabChange("workspace"));
  act(() => button("sessions.new").props.onPress());
  const modal = tree.root.findAllByType(Modal).find(node => node.props.visible)!;
  expect(modal.findByType(Button).props.disabled).toBe(true);
  expect(modal.findAllByType(Text).some(node => node.props.children === "sessions.noWorkspaces")).toBe(true);
});

test("new chat opens immediately, without an extra creation dialog", () => {
  act(() => tree.root.findByType(SessionList).props.onTabChange("chat"));
  act(() => button("sessions.new").props.onPress());
  expect(mockRemote.newConversation).toHaveBeenCalledWith("chat");
});
