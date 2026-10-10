import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import {
  Modal,
  Platform,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { ConnectionBadge } from "../../components/ConnectionBadge";
import { Button } from "../../components/Button";
import { DialogSurface } from "../../components/DialogSurface";
import { SessionsScreen } from "../SessionsScreen";
import { SessionList } from "../SessionList";
import { DesktopPicker } from "../DesktopPicker";
import { ActionMenu } from "../../components/ActionMenu";
import { RenameModal } from "../../features/chat/components/RenameModal";
import { DisconnectedScreen } from "../DisconnectedScreen";
import { SettingsLink } from "../../features/settings/SettingsPrimitives";
import { checkForUpdate } from "../../update/update";
import { promptUpgrade } from "../../update/prompt";

let mockDimensions = { width: 320, height: 640, scale: 1, fontScale: 1 };
const mockRemote = {
  desktops: [
    { desktopId: "desktop_1", pairId: "p1", name: "A very long desktop name" },
    { desktopId: "desktop_2", pairId: "p2" },
  ],
  credentials: { pairId: "p1", expectedDesktopId: "desktop_1" },
  workspaces: [] as { id: string; name: string }[],
  connectionPresentation: { level: "connected", titleKey: "connection.connected", hintKey: "connection.offlineHint" },
  desktopOnline: true,
  capabilities: new Set(["desktop_settings_v1", "skill_management_v1", "workspace_create_v1"]),
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
  unpair: jest.fn(async () => {}),
  rename: jest.fn(async () => {}),
  generateTitle: jest.fn(async () => {}),
  deleteSession: jest.fn(async () => {}),
  createWorkspace: jest.fn(async (_path: string, _name: string) => ({ id: "w9", name: "New", path: "/host/new" })),
  clearError: jest.fn(),
  phase: "connected" as string,
  hasConnectedContent: true,
  error: null as string | null,
  presence: null as { reason?: string } | null,
};
jest.mock("react-native", () => {
  const actual = jest.requireActual("react-native");
  return new Proxy(actual, {
    get: (target, key) => key === "useWindowDimensions" ? () => mockDimensions : Reflect.get(target, key),
  });
});
jest.mock("../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote, useRemote: () => mockRemote }));
jest.mock("../SessionList", () => ({ SessionList: () => null }));
jest.mock("../../i18n/LanguageSettings", () => ({ LanguageSettings: () => null }));
jest.mock("../../update/prompt", () => ({ promptUpgrade: jest.fn() }));
jest.mock("../../update/update", () => ({ checkForUpdate: jest.fn() }));
jest.mock("lucide-react-native", () => Object.fromEntries(
  ["ArrowLeft", "ChevronLeft", "ChevronRight", "ChevronDown", "Check", "Folder", "LogOut", "MessageCircle", "Monitor", "Plus", "Pin", "Pencil", "Trash2", "Settings", "Unplug", "X"].map(name => [name, name]),
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

/**
 * The device header keeps its two guarantees from before the picker became a
 * dropdown: a real touch target on a small phone, and real gutters so the
 * selector does not collide with the connection badge beside it.
 */
test("device header has real gutters and a non-shrinking touch target on a small phone", () => {
  const selector = button("desktops.pickerLabel");
  const style = StyleSheet.flatten(selector.props.style({ pressed: false }));
  expect(style.minHeight).toBeGreaterThanOrEqual(44);
  expect(style.minWidth).toBe(0);
  // The picker renders its own modal alongside the trigger, so the header is
  // found by walking up to the nearest ancestor that actually defines gutters —
  // asserting on a fixed number of parents would break the next time the
  // picker grows a wrapper.
  let header = selector.parent;
  while (header && StyleSheet.flatten(header.props?.style)?.paddingHorizontal === undefined) {
    header = header.parent;
  }
  expect(StyleSheet.flatten(header?.props.style).paddingHorizontal).toBe(16);
  const badge = tree.root.findByType(ConnectionBadge);
  expect(badge.findAllByType(Text)).toHaveLength(0);
  expect(badge.props.active).toBe(true);
  // Opening the picker is the selector's own action now; managing pairings moved
  // into the picker's list, which is where someone looking for another desktop
  // will be. Rendering it must not throw (the mocked icon set is the reason a
  // missing export would show up here rather than in the app).
  act(() => selector.props.onPress());
  expect(onManageDesktops).not.toHaveBeenCalled();
  expect(tree.root.findAllByType(DesktopPicker).length).toBe(1);
});

test("the picker offers every paired desktop plus the merged option", () => {
  const picker = tree.root.findByType(DesktopPicker);
  expect(picker.props.desktops.map((desktop: { desktopId: string }) => desktop.desktopId))
    .toEqual(["desktop_1", "desktop_2"]);
  // Defaults to the merged list so a user with several pairings sees them all
  // without having to discover the picker first.
  expect(picker.props.filter).toEqual({ kind: "all" });
  act(() => picker.props.onChange({ kind: "desktop", desktopId: "desktop_2" }));
  expect(tree.root.findByType(DesktopPicker).props.filter)
    .toEqual({ kind: "desktop", desktopId: "desktop_2" });
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

const createWorkspaceRow = () =>
  tree.root.findAll(node =>
    node.props.accessibilityLabel === "sessions.createWorkspace"
    && typeof node.props.onPress === "function",
  )[0]!;
const formInput = (label: string) =>
  visibleModal()!.findAllByType(TextInput).find(node => node.props.accessibilityLabel === label);
const footerButton = (label: string) =>
  visibleModal()!.findAllByType(Button).find(node => node.props.label === label)!;

test("the create-workspace row opens a form that registers a typed desktop path", async () => {
  act(() => tree.root.findByType(SessionList).props.onTabChange("workspace"));
  act(() => button("sessions.new").props.onPress());
  act(() => createWorkspaceRow().props.onPress());
  // The form replaces the picker, and its action stays disabled until a path
  // is typed — an empty path names no directory on the desktop.
  expect(formInput("sessions.workspacePath")).toBeDefined();
  expect(footerButton("sessions.createWorkspaceAction").props.disabled).toBe(true);
  act(() => formInput("sessions.workspaceName")!.props.onChangeText("New"));
  act(() => formInput("sessions.workspacePath")!.props.onChangeText("  /host/new  "));
  expect(footerButton("sessions.createWorkspaceAction").props.disabled).toBe(false);
  await act(async () => { footerButton("sessions.createWorkspaceAction").props.onPress(); });
  // The path is trimmed before it reaches the desktop; the name is optional.
  expect(mockRemote.createWorkspace).toHaveBeenCalledWith("/host/new", "New");
  // Back on the picker with the created workspace selected, so the ordinary
  // "new conversation" action starts a conversation against it.
  expect(formInput("sessions.workspacePath")).toBeUndefined();
  const dialog = visibleModal()!;
  act(() => footerButton("sessions.new").props.onPress());
  expect(mockRemote.newConversation).not.toHaveBeenCalled();
  act(() => dialog.props.onDismiss());
  expect(mockRemote.newConversation).toHaveBeenCalledWith("workspace", "w9");
});

test("an empty workspace list still offers the create row", () => {
  act(() => tree.root.findByType(SessionList).props.onTabChange("workspace"));
  act(() => button("sessions.new").props.onPress());
  expect(createWorkspaceRow()).toBeDefined();
});

test("a desktop that does not advertise the capability hides the create row", () => {
  const advertised = mockRemote.capabilities;
  mockRemote.capabilities = new Set(["desktop_settings_v1"]);
  try {
    act(() => tree.root.findByType(SessionList).props.onTabChange("workspace"));
    act(() => button("sessions.new").props.onPress());
    // An older desktop answers `create_workspace` with "Unsupported command",
    // so the action must not be offered at all.
    expect(createWorkspaceRow()).toBeUndefined();
  } finally {
    mockRemote.capabilities = advertised;
  }
});

test("a refused workspace creation is reported and keeps the path on screen", async () => {
  jest.mocked(mockRemote.createWorkspace).mockRejectedValueOnce(new Error("no such directory"));
  act(() => tree.root.findByType(SessionList).props.onTabChange("workspace"));
  act(() => button("sessions.new").props.onPress());
  act(() => createWorkspaceRow().props.onPress());
  act(() => formInput("sessions.workspacePath")!.props.onChangeText("/host/missing"));
  await act(async () => { footerButton("sessions.createWorkspaceAction").props.onPress(); });
  expect(modalText()).toContain("sessions.createWorkspaceFailed");
  // The form stays open with the path, so the user corrects it instead of
  // retyping from the picker.
  expect(formInput("sessions.workspacePath")!.props.value).toBe("/host/missing");
});

const visibleModal = () =>
  tree.root.findAllByType(Modal).find(node => node.props.visible);
const modalText = () =>
  visibleModal()!.findAllByType(Text)
    .map(node => node.props.children)
    .filter((child): child is string => typeof child === "string");
const session = { sessionId: "s", threadId: "t", title: "Reply", streaming: false };

describe("screen-owned surfaces", () => {
  const platform = Platform.OS;

  beforeEach(() => {
    mockRemote.connectionPresentation = { level: "connected", titleKey: "connection.connected", hintKey: "connection.offlineHint" };
    mockRemote.phase = "connected";
    mockRemote.hasConnectedContent = true;
    mockRemote.error = null;
    mockRemote.desktopOnline = true;
  });
  afterEach(() => {
    act(() => jest.runOnlyPendingTimers());
    Platform.OS = platform;
  });

  test("deactivating the screen drops every surface it owns", () => {
    act(() => button("sessions.settings").props.onPress());
    expect(visibleModal()).toBeDefined();
    act(() => tree.update(createElement(SessionsScreen, { onManageDesktops, active: false })));
    expect(visibleModal()).toBeUndefined();
    act(() => tree.update(createElement(SessionsScreen, { onManageDesktops, active: true })));
    expect(visibleModal()).toBeUndefined();
  });

  test("unpairing asks first and only then disconnects", () => {
    act(() => tree.root.findByType(ConnectionBadge).props.onUnpair());
    expect(modalText()).toContain("sessions.unpairConfirm");
    const dialog = visibleModal()!;
    const confirm = dialog.findAllByType(Button)
      .find(node => node.props.label === "sessions.unpair")!;
    act(() => confirm.props.onPress());
    expect(mockRemote.unpair).not.toHaveBeenCalled();
    act(() => dialog.props.onDismiss());
    expect(mockRemote.unpair).toHaveBeenCalledTimes(1);
  });

  test("a session rename trims, submits once and reports failure", async () => {
    act(() => tree.root.findByType(SessionList).props.onMenu(session));
    const menu = tree.root.findByType(ActionMenu);
    act(() => button("chat.rename").props.onPress());
    await act(async () => { menu.findByType(Modal).props.onDismiss(); });
    expect(tree.root.findByType(RenameModal).props.renameOpen).toBe(true);
    expect(tree.root.findByType(RenameModal).props.renameValue).toBe("Reply");
    // The title generator is wired to the desktop command in the UI language.
    await act(async () => { await tree.root.findByType(RenameModal).props.onGenerate(); });
    expect(mockRemote.generateTitle).toHaveBeenCalledWith("s", "en");
    // An empty (or whitespace-only) name must not reach the desktop.
    act(() => tree.root.findByType(RenameModal).props.setRenameValue("   "));
    expect(tree.root.findByType(RenameModal).props.renameValue).toBe("   ");
    await act(async () => { await tree.root.findByType(RenameModal).props.submitRename(); });
    expect(mockRemote.rename).not.toHaveBeenCalled();
    act(() => tree.root.findByType(RenameModal).props.setRenameValue("  New name  "));
    await act(async () => { await tree.root.findByType(RenameModal).props.submitRename(); });
    expect(mockRemote.rename).toHaveBeenCalledWith("s", "New name");
    expect(tree.root.findByType(RenameModal).props.renameOpen).toBe(false);
    act(() => tree.root.findByType(RenameModal).props.onClose());
    // A refused rename is reported rather than silently dropped.
    jest.mocked(mockRemote.rename).mockRejectedValueOnce(new Error("desktop busy"));
    act(() => tree.root.findByType(SessionList).props.onMenu(session));
    act(() => button("chat.rename").props.onPress());
    await act(async () => { tree.root.findByType(ActionMenu).findByType(Modal).props.onDismiss(); });
    act(() => tree.root.findByType(RenameModal).props.setRenameValue("Again"));
    await act(async () => { await tree.root.findByType(RenameModal).props.submitRename(); });
    expect(modalText()).toContain("common.error");
  });

  test("a failed pin toggle surfaces an error instead of a stuck row", async () => {
    jest.mocked(mockRemote.setSessionPinned).mockRejectedValueOnce(new Error("offline"));
    act(() => tree.root.findByType(SessionList).props.onMenu(session));
    const menu = tree.root.findByType(ActionMenu);
    act(() => button("sessions.pin").props.onPress());
    await act(async () => { menu.findByType(Modal).props.onDismiss(); });
    expect(modalText()).toContain("common.error");
  });

  test("deleting a session confirms with the title and reports a failed delete", async () => {
    act(() => tree.root.findByType(SessionList).props.onMenu(session));
    const menu = tree.root.findByType(ActionMenu);
    act(() => button("sessions.delete").props.onPress());
    await act(async () => { menu.findByType(Modal).props.onDismiss(); });
    expect(modalText()).toContain("sessions.deleteConfirm");
    const dialog = visibleModal()!;
    const confirm = dialog.findAllByType(Button)
      .find(node => node.props.label === "sessions.delete")!;
    act(() => confirm.props.onPress());
    await act(async () => { dialog.props.onDismiss(); });
    expect(mockRemote.deleteSession).toHaveBeenCalledWith("s", "t");
    // A refused delete is reported instead of leaving the row in place silently.
    jest.mocked(mockRemote.deleteSession).mockRejectedValueOnce(new Error("file is open"));
    act(() => tree.root.findByType(SessionList).props.onMenu(session));
    const again = tree.root.findByType(ActionMenu);
    act(() => button("sessions.delete").props.onPress());
    await act(async () => { again.findByType(Modal).props.onDismiss(); });
    const second = visibleModal()!;
    act(() => second.findAllByType(Button).find(node => node.props.label === "sessions.delete")!
      .props.onPress());
    await act(async () => { second.props.onDismiss(); });
    expect(modalText()).toContain("common.error");
  });

  test("the Android new-conversation dialog defers the start until the sheet is gone", async () => {
    Platform.OS = "android";
    act(() => tree.root.findByType(SessionList).props.onTabChange("workspace"));
    act(() => button("sessions.new").props.onPress());
    // Switching back to chat keeps whatever workspace was selected.
    const modes = visibleModal()!.findAll(node =>
      node.props.accessibilityRole === "radio" && typeof node.props.onPress === "function");
    act(() => modes[1]!.props.onPress());
    expect(mockRemote.newConversation).not.toHaveBeenCalled();
    act(() => visibleModal()!.findAllByType(Button).find(node => node.props.label === "sessions.new")!
      .props.onPress());
    expect(mockRemote.newConversation).not.toHaveBeenCalled();
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
    expect(mockRemote.newConversation).toHaveBeenCalledWith("chat", "");
  });

  test("Android's dismissal event is not a second flush point", async () => {
    Platform.OS = "android";
    act(() => tree.root.findByType(SessionList).props.onTabChange("workspace"));
    act(() => button("sessions.new").props.onPress());
    // Switching back to chat keeps whatever workspace was selected.
    const modes = visibleModal()!.findAll(node =>
      node.props.accessibilityRole === "radio" && typeof node.props.onPress === "function");
    act(() => modes[1]!.props.onPress());
    expect(mockRemote.newConversation).not.toHaveBeenCalled();
    const dialog = visibleModal()!;
    act(() => dialog.findAllByType(Button).find(node => node.props.label === "sessions.new")!
      .props.onPress());
    expect(mockRemote.newConversation).not.toHaveBeenCalled();
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
    expect(mockRemote.newConversation).toHaveBeenCalledTimes(1);
    // Off iOS the deferred start is the only flush point; the dismissal event
    // that follows must not start the conversation a second time.
    act(() => dialog.props.onDismiss());
    expect(mockRemote.newConversation).toHaveBeenCalledTimes(1);
  });

  test("an empty workspace list keeps the dialog open and creates nothing", () => {
    Platform.OS = "android";
    act(() => tree.root.findByType(SessionList).props.onTabChange("workspace"));
    act(() => button("sessions.new").props.onPress());
    const dialog = visibleModal()!;
    // Re-picking the workspace mode with no workspaces must not fabricate an id.
    act(() => dialog.findAll(node => node.props.accessibilityRole === "radio")[0]!.props.onPress());
    act(() => dialog.findAllByType(Button).find(node => node.props.label === "sessions.new")!.props.onPress());
    expect(mockRemote.newConversation).not.toHaveBeenCalled();
    act(() => dialog.findAll(node => node.props.accessibilityLabel === "common.close"
      && node.props.onPress)[0]!.props.onPress());
    expect(visibleModal()).toBeUndefined();
  });

  test("iOS starts the conversation only after the sheet reports its dismissal", async () => {
    Platform.OS = "ios";
    mockRemote.workspaces = [{ id: "w1", name: "One" }];
    act(() => tree.root.findByType(SessionList).props.onTabChange("workspace"));
    act(() => button("sessions.new").props.onPress());
    const dialog = visibleModal()!;
    act(() => dialog.findAllByType(Button).find(node => node.props.label === "sessions.new")!
      .props.onPress());
    expect(mockRemote.newConversation).not.toHaveBeenCalled();
    act(() => dialog.props.onDismiss());
    expect(mockRemote.newConversation).toHaveBeenCalledWith("workspace", "w1");
    expect(visibleModal()).toBeUndefined();
  });

  test("an update check reports both an available and an up-to-date release", async () => {
    Platform.OS = "android";
    const checkFromSettings = async () => {
      await act(async () => { button("sessions.settings").props.onPress(); });
      const link = visibleModal()!.findAllByType(SettingsLink)
        .find(node => node.props.label === "update.check")!;
      act(() => link.props.onPress());
      await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
    };
    jest.mocked(checkForUpdate).mockResolvedValueOnce({ hasUpdate: true } as never);
    await checkFromSettings();
    expect(promptUpgrade).toHaveBeenCalledTimes(1);
    await act(async () => { button("sessions.settings").props.onPress(); });
    const again = visibleModal()!.findAllByType(SettingsLink)
      .find(node => node.props.label === "update.check")!;
    jest.mocked(checkForUpdate).mockResolvedValueOnce({ hasUpdate: false } as never);
    act(() => again.props.onPress());
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
    expect(modalText()).toContain("update.upToDate");
    expect(promptUpgrade).toHaveBeenCalledTimes(1);
  });

  test("reconnecting from a disconnected screen tracks the retry lifecycle", async () => {
    mockRemote.connectionPresentation = { level: "disconnected", titleKey: "connection.disconnected", hintKey: "connection.offlineHint" };
    await act(async () => { tree.update(createElement(SessionsScreen, { onManageDesktops })); });
    expect(moduleDisconnected()).toBeDefined();
    await act(async () => { moduleDisconnected()!.props.onReconnect(); });
    expect(mockRemote.reconnect).toHaveBeenCalledTimes(1);
    // The transport starts working: the screen holds the standalone status.
    mockRemote.phase = "connecting";
    await act(async () => { tree.update(createElement(SessionsScreen, { onManageDesktops })); });
    expect(moduleDisconnected()).toBeDefined();
    // ...and only a settled connection ends the standalone view.
    mockRemote.phase = "connected";
    mockRemote.connectionPresentation = { level: "connected", titleKey: "connection.connected", hintKey: "connection.offlineHint" };
    await act(async () => { tree.update(createElement(SessionsScreen, { onManageDesktops })); });
    expect(moduleDisconnected()).toBeUndefined();
  });
});

const moduleDisconnected = () => tree.root.findAllByType(DisconnectedScreen)[0];
