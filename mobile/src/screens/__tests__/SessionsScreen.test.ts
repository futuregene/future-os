import { createElement, type ReactElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import {
  ActivityIndicator,
  Modal,
  Platform,
  ScrollView,
  StyleSheet,
  Text,
  TextInput,
  View,
} from "react-native";
import { Check } from "lucide-react-native";
import { ConnectionBadge } from "../../components/ConnectionBadge";
import { Button } from "../../components/Button";
import { DialogSurface } from "../../components/DialogSurface";
import { SessionsScreen } from "../SessionsScreen";
import { SessionList } from "../SessionList";
import { DesktopPicker } from "../DesktopPicker";
import { ActionMenu } from "../../components/ActionMenu";
import type { MachineSummary } from "../../remote/desktopStatus";
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
  newConversationOn: jest.fn(async (_desktopId: string, _mode?: string, _workspaceId?: string) => {}),
  reconnect: jest.fn(),
  setSessionPinned: jest.fn(async () => {}),
  unpair: jest.fn(async () => {}),
  rename: jest.fn(async () => {}),
  generateTitle: jest.fn(async () => {}),
  deleteSession: jest.fn(async () => {}),
  setWorkspacePinned: jest.fn(async () => {}),
  renameWorkspace: jest.fn(async () => {}),
  deleteWorkspace: jest.fn(async () => {}),
  createWorkspace: jest.fn(async (_path: string, _name: string) => ({ id: "w9", name: "New", path: "/host/new" })),
  switchDesktop: jest.fn(async (_desktopId: string) => {}),
  refreshWorkspaces: jest.fn(async () => {}),
  refreshModels: jest.fn(async () => {}),
  clearError: jest.fn(),
  phase: "connected" as string,
  hasConnectedContent: true,
  error: null as string | null,
  presence: null as { reason?: string } | null,
  catalogs: null as unknown,
};
jest.mock("react-native", () => {
  const actual = jest.requireActual("react-native");
  return new Proxy(actual, {
    get: (target, key) => key === "useWindowDimensions" ? () => mockDimensions : Reflect.get(target, key),
  });
});
jest.mock("../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote, useRemote: () => mockRemote }));
const mockCatalogs = {
  catalogs: [] as { desktopId: string; label: string; sessions: never[] }[],
  machines: [] as MachineSummary[],
  refresh: jest.fn(async () => {}),
  reconnect: jest.fn(async () => {}),
  commandsFor: jest.fn(() => null) as jest.Mock,
  readWorkspaces: jest.fn(async (_desktopId: string) => [] as { id: string; name: string; path: string }[]),
};
// The reader of every other machine's catalogue now comes from the provider,
// where each machine's runtime lives, rather than from a hook of this screen.
mockRemote.catalogs = mockCatalogs;
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
// The machine a string names is one of the options it is given — an action
// sheet's heading, the new-conversation step's title — so the mock keeps it:
// naming the wrong machine must fail an expectation rather than read as a bare
// key.
jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: { name?: string }) =>
      options?.name ? `${key}:${options.name}` : key,
    i18n: { language: "en" },
  }),
}));

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

test("the picker can be dismissed without choosing a desktop", () => {
  const pickerModal = () => tree.root.findByType(DesktopPicker).findAllByType(Modal)[0]!;
  // Nothing to dismiss while the list is shut, so the close button the user
  // reaches for only exists once the picker is open.
  expect(pickerModal().props.visible).toBe(false);
  act(() => button("desktops.pickerLabel").props.onPress());
  const open = pickerModal();
  expect(open.props.visible).toBe(true);
  const close = open.findAll(node =>
    node.props.accessibilityLabel === "common.close" && typeof node.props.onPress === "function",
  )[0]!;
  act(() => close.props.onPress());
  expect(pickerModal().props.visible).toBe(false);
  // Backing out is not a choice: the list still shows every desktop.
  expect(tree.root.findByType(DesktopPicker).props.filter).toEqual({ kind: "all" });
});

test("a picker row is the desktop's name and nothing else", () => {
  act(() => button("desktops.pickerLabel").props.onPress());
  // Host nodes only: a composite and its host both carry the row's props.
  const rows = tree.root.findAll(node =>
    typeof node.type === "string" &&
    node.props.accessibilityRole === "button" &&
    node.props.accessibilityState?.selected !== undefined);
  const labels = rows.map(row =>
    row.findAllByType(Text).flatMap(text => text.props.children as string[]).join(""));
  // `desktop_1` is the one this phone is paired with, and the header's status dot
  // already says whether the phone is connected to it — a "Connected" label on
  // that one row repeats the dot and makes the spare row look second-class.
  expect(labels).toEqual(["desktops.filterAll", "🖥️ A very long desktop name", "🖥️ desktop_2"]);
});

test("the status dot answers for every machine, and reconnect reaches them all", () => {
  // The merged list holds several machines, so the dot's own answer is whether
  // all of them are up — not the state of whichever one is active.
  mockCatalogs.machines = [
    { desktopId: "desktop_2", label: "Office", active: false, status: "offline" },
  ];
  try {
    act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
    const badge = tree.root.findByType(ConnectionBadge);
    expect(badge.props.machines.map((machine: MachineSummary) => machine.desktopId))
      .toEqual(["desktop_1", "desktop_2"]);
    expect(badge.props.machines[0].active).toBe(true);
    expect(badge.props.machines[0].label).toBe("A very long desktop name");

    act(() => badge.props.onReconnect());
    expect(mockRemote.reconnect).toHaveBeenCalledTimes(1);
    expect(mockCatalogs.reconnect).toHaveBeenCalledTimes(1);
  } finally {
    mockCatalogs.machines = [];
  }
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

test.each(["desktopSettings.followAccount", "language.title"])(
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
  expect(mockRemote.newConversationOn).not.toHaveBeenCalled();
  act(() => modal.props.onDismiss());
  // The machine is the one the phone is on — named like any other, so the
  // provider is the only place that decides whether a move is needed.
  expect(mockRemote.newConversationOn).toHaveBeenCalledWith("desktop_1", "workspace", "w24");
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
  // One machine can take it, so there is nothing to name and nothing to move:
  // the open machine's own verb is what the shortcut has always used.
  expect(mockRemote.newConversation).toHaveBeenCalledWith("chat");
  expect(mockRemote.newConversationOn).not.toHaveBeenCalled();
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
  expect(mockRemote.newConversationOn).not.toHaveBeenCalled();
  act(() => dialog.props.onDismiss());
  expect(mockRemote.newConversationOn).toHaveBeenCalledWith("desktop_1", "workspace", "w9");
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
    expect(mockRemote.newConversationOn).not.toHaveBeenCalled();
    act(() => visibleModal()!.findAllByType(Button).find(node => node.props.label === "sessions.new")!
      .props.onPress());
    expect(mockRemote.newConversationOn).not.toHaveBeenCalled();
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
    expect(mockRemote.newConversationOn).toHaveBeenCalledWith("desktop_1", "chat", "");
  });

  test("Android's dismissal event is not a second flush point", async () => {
    Platform.OS = "android";
    act(() => tree.root.findByType(SessionList).props.onTabChange("workspace"));
    act(() => button("sessions.new").props.onPress());
    // Switching back to chat keeps whatever workspace was selected.
    const modes = visibleModal()!.findAll(node =>
      node.props.accessibilityRole === "radio" && typeof node.props.onPress === "function");
    act(() => modes[1]!.props.onPress());
    expect(mockRemote.newConversationOn).not.toHaveBeenCalled();
    const dialog = visibleModal()!;
    act(() => dialog.findAllByType(Button).find(node => node.props.label === "sessions.new")!
      .props.onPress());
    expect(mockRemote.newConversationOn).not.toHaveBeenCalled();
    await act(async () => { await new Promise(resolve => setTimeout(resolve, 5)); });
    expect(mockRemote.newConversationOn).toHaveBeenCalledTimes(1);
    // Off iOS the deferred start is the only flush point; the dismissal event
    // that follows must not start the conversation a second time.
    act(() => dialog.props.onDismiss());
    expect(mockRemote.newConversationOn).toHaveBeenCalledTimes(1);
  });

  test("an empty workspace list keeps the dialog open and creates nothing", () => {
    Platform.OS = "android";
    act(() => tree.root.findByType(SessionList).props.onTabChange("workspace"));
    act(() => button("sessions.new").props.onPress());
    const dialog = visibleModal()!;
    // Re-picking the workspace mode with no workspaces must not fabricate an id.
    act(() => dialog.findAll(node => node.props.accessibilityRole === "radio")[0]!.props.onPress());
    act(() => dialog.findAllByType(Button).find(node => node.props.label === "sessions.new")!.props.onPress());
    expect(mockRemote.newConversationOn).not.toHaveBeenCalled();
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
    expect(mockRemote.newConversationOn).not.toHaveBeenCalled();
    act(() => dialog.props.onDismiss());
    expect(mockRemote.newConversationOn).toHaveBeenCalledWith("desktop_1", "workspace", "w1");
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

/**
 * A conversation merged in from another machine.
 *
 * Its actions are commands to *that* desktop, so they go down that machine's
 * own connection — the phone's active connection has never heard of the
 * session — and they are offered only while that machine can take one.
 */
describe("conversations on other machines", () => {
  const session = { sessionId: "elsewhere", threadId: "t-elsewhere", title: "Elsewhere", streaming: false };
  const peerCommands = {
    renameSession: jest.fn(async (_sessionId: string, _name: string) => {}),
    generateTitle: jest.fn(async (_sessionId: string, _language: string) => "Generated"),
    setSessionPinned: jest.fn(async (_sessionId: string, _threadId: string, _pinned: boolean) => {}),
    deleteSession: jest.fn(async (_sessionId: string, _threadId: string) => {}),
    renameWorkspace: jest.fn(async (_workspaceId: string, _name: string) => {}),
    setWorkspacePinned: jest.fn(async (_workspaceId: string, _pinned: boolean) => {}),
    deleteWorkspace: jest.fn(async (_workspaceId: string) => {}),
  };

  beforeEach(() => {
    for (const command of Object.values(peerCommands)) command.mockClear();
    mockCatalogs.commandsFor = jest.fn(() => peerCommands);
  });

  afterEach(() => {
    mockCatalogs.machines = [];
    mockCatalogs.commandsFor = jest.fn(() => null);
  });

  const openMenu = () =>
    act(() => tree.root.findByType(SessionList).props.onMenu(session, "desktop_2"));
  const actions = () => tree.root.findByType(ActionMenu).props.actions;

  test("the menu names the machine its actions are addressed to", () => {
    mockCatalogs.machines = [
      { desktopId: "desktop_2", label: "Office", active: false, status: "connected" },
    ];
    act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
    openMenu();

    // The sheet says where the commands land before the first destructive item:
    // a delete that runs on another machine without saying so is the one action
    // a reader cannot take back.
    expect(actions()[0]).toMatchObject({ heading: true, label: "sessions.actionsOnDesktop:Office" });
    expect(actions().map((action: { label: string }) => action.label))
      .toEqual(["sessions.actionsOnDesktop:Office", "sessions.pin", "chat.rename", "sessions.delete"]);
    // Every one of them can run: the machine it addresses is connected.
    expect(actions().filter((action: { onPress?: unknown }) => action.onPress)
      .every((action: { disabled?: boolean }) => !action.disabled)).toBe(true);
  });

  test("a pin and a rename go to the machine that owns the conversation", async () => {
    mockCatalogs.machines = [
      { desktopId: "desktop_2", label: "Office", active: false, status: "connected" },
    ];
    act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
    openMenu();

    act(() => button("sessions.pin").props.onPress());
    await act(async () => { tree.root.findByType(ActionMenu).findByType(Modal).props.onDismiss(); });
    expect(peerCommands.setSessionPinned).toHaveBeenCalledWith("elsewhere", "t-elsewhere", true);
    expect(mockRemote.setSessionPinned).not.toHaveBeenCalled();

    openMenu();
    act(() => button("chat.rename").props.onPress());
    await act(async () => { tree.root.findByType(ActionMenu).findByType(Modal).props.onDismiss(); });
    // The title generator is the owning machine's too, not the active one's.
    expect(tree.root.findByType(RenameModal).props.generationKey).toBe("elsewhere");
    await act(async () => { await tree.root.findByType(RenameModal).props.onGenerate(); });
    expect(peerCommands.generateTitle).toHaveBeenCalledWith("elsewhere", "en");
    expect(mockRemote.generateTitle).not.toHaveBeenCalled();
    act(() => tree.root.findByType(RenameModal).props.setRenameValue("New name"));
    await act(async () => { await tree.root.findByType(RenameModal).props.submitRename(); });
    expect(peerCommands.renameSession).toHaveBeenCalledWith("elsewhere", "New name");
    expect(mockRemote.rename).not.toHaveBeenCalled();
  });

  test("a machine that cannot answer offers no menu at all", () => {
    // `connecting`, `preparing` and `offline` are all states in which the host
    // refuses or parks a command, so the row's action is not offered.
    mockCatalogs.machines = [
      { desktopId: "desktop_2", label: "Office", active: false, status: "offline" },
    ];
    act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
    openMenu();

    expect(tree.root.findByType(ActionMenu).props.visible).toBe(false);
    expect(mockCatalogs.commandsFor).not.toHaveBeenCalled();
  });

  test("a command whose machine went away is reported instead of lost", async () => {
    mockCatalogs.machines = [
      { desktopId: "desktop_2", label: "Office", active: false, status: "connected" },
    ];
    act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
    openMenu();
    // The machine drops while the sheet is already up, so the item is what has
    // to say the command could not run: a tap that silently does nothing reads
    // as a pin that worked.
    mockCatalogs.commandsFor = jest.fn(() => null);

    act(() => button("sessions.pin").props.onPress());
    await act(async () => {
      tree.root.findByType(ActionMenu).findByType(Modal).props.onDismiss();
    });

    expect(peerCommands.setSessionPinned).not.toHaveBeenCalled();
    expect(modalText()).toContain("common.error");
  });
});

/**
 * A conversation of the phone's *own* machine names it too.
 *
 * The merged list is one list of several machines, so a row's sheet says which
 * machine its commands are addressed to wherever the conversation lives. Naming
 * only a peer's row left the shape of a sheet depending on which machine the
 * phone happened to be reading — the same conversation gained and lost the line
 * as the reader moved between machines.
 */
test("a conversation on the machine the phone is reading names it too", () => {
  const local = { sessionId: "here", threadId: "t-here", title: "Here", streaming: false };
  act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
  act(() => tree.root.findByType(SessionList).props.onMenu(local, "desktop_1"));

  const actions = tree.root.findByType(ActionMenu).props.actions;
  // The machine as this phone labels it — the name, not the id, because the
  // reader has given it one.
  expect(actions[0]).toMatchObject({
    heading: true,
    label: "sessions.actionsOnDesktop:A very long desktop name",
  });
  expect(actions.map((action: { label: string }) => action.label)).toEqual([
    "sessions.actionsOnDesktop:A very long desktop name",
    "sessions.pin",
    "chat.rename",
    "sessions.delete",
  ]);
});

/**
 * Creating a conversation when more than one machine could take it.
 *
 * The phone holds several pairings and renders one merged list, so "New
 * conversation" used to create the conversation on whichever machine happened
 * to be *connected* — not the one the list may have been filtered to, and not
 * necessarily the one the user meant. The machine is now the first thing the
 * dialog asks, and the conversation is created on the machine it names.
 */
describe("creating on the machine the user picks", () => {
  const platform = Platform.OS;
  const peerWorkspaces = [{ id: "pw1", name: "Peer project", path: "/host/peer" }];
  const creatablePeer = {
    desktopId: "desktop_2",
    label: "Office",
    active: false,
    status: "connected" as const,
  };

  beforeEach(() => {
    Platform.OS = "ios";
    mockRemote.desktopOnline = true;
    mockRemote.connectionPresentation = {
      level: "connected",
      titleKey: "connection.connected",
      hintKey: "connection.offlineHint",
    };
    mockCatalogs.machines = [creatablePeer];
    mockCatalogs.commandsFor = jest.fn(() => ({ capabilities: new Set<string>() }));
    jest.mocked(mockCatalogs.readWorkspaces).mockResolvedValue(peerWorkspaces);
    // The machine list is read while rendering, so it has to be in place before
    // the tree is told to render it.
    act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
  });

  afterEach(() => {
    Platform.OS = platform;
    mockCatalogs.machines = [];
    mockCatalogs.commandsFor = jest.fn(() => null);
    mockCatalogs.readWorkspaces = jest.fn(async (_desktopId: string) => []);
  });

  const openDialog = (tab: "chat" | "workspace") => {
    act(() => tree.root.findByType(SessionList).props.onTabChange(tab));
    act(() => button("sessions.new").props.onPress());
  };
  /** A machine's row in the picker, addressed by the name the user sees. */
  const deviceRow = (label: string) =>
    tree.root.findAll(node =>
      node.props.accessibilityLabel === label
      && node.props.accessibilityRole === "button"
      && typeof node.props.onPress === "function",
    )[0]!;
  const footer = (label: string) =>
    visibleModal()!.findAllByType(Button).find(node => node.props.label === label)!;

  test("a chat asks which machine, and is created on the one it names", async () => {
    openDialog("chat");
    expect(modalText()).toContain("sessions.chooseDevice");
    // Nothing has been created yet: the machine is the user's to name.
    expect(mockRemote.newConversationOn).not.toHaveBeenCalled();
    // The machine the phone is on is the one marked — it is the row that needs
    // no move, so the reader can tell it from the machines to move to.
    const current = deviceRow("A very long desktop name");
    expect(current.props.accessibilityState).toMatchObject({ selected: true, disabled: false });
    expect(current.findAllByType(Check)).toHaveLength(1);

    const dialog = visibleModal()!;
    await act(async () => { deviceRow("Office").props.onPress(); });
    // The chat has no workspace to choose, so the machine was the whole
    // question: it is created *on that machine*, named outright rather than
    // reached by moving the phone first. `newConversation` cannot say that — it
    // is the open machine's verb, and this dialog's copy of it was built from
    // the render that drew the dialog, so calling it after a move would build
    // the draft on the machine the phone had just left.
    expect(mockRemote.newConversationOn).not.toHaveBeenCalled();
    await act(async () => { dialog.props.onDismiss(); });
    expect(mockRemote.newConversationOn).toHaveBeenCalledWith("desktop_2", "chat", "");
    expect(mockRemote.newConversation).not.toHaveBeenCalled();
  });

  test("a chat on the machine the phone is already on is created on it", async () => {
    openDialog("chat");
    const dialog = visibleModal()!;
    await act(async () => { deviceRow("A very long desktop name").props.onPress(); });
    await act(async () => { dialog.props.onDismiss(); });
    // Named like any other machine: the provider decides that this one needs no
    // move, and the screen does not keep a second opinion about which machine.
    expect(mockRemote.newConversationOn).toHaveBeenCalledWith("desktop_1", "chat", "");
    expect(mockRemote.newConversation).not.toHaveBeenCalled();
  });

  test("a workspace list on another machine is that machine's, read from it", async () => {
    mockRemote.workspaces = [{ id: "w1", name: "Connected project" }];
    openDialog("workspace");
    await act(async () => { deviceRow("Office").props.onPress(); });
    // The list is read down the other machine's own connection: the connected
    // machine's workspaces name workspaces the other machine has never heard of.
    expect(mockCatalogs.readWorkspaces).toHaveBeenCalledWith("desktop_2");
    // The header names the machine the step is about — the mock keeps the name
    // the string was given, so this is that machine and not just any.
    expect(modalText()).toContain("sessions.newOnDevice:Office");
    expect(modalText()).toContain("Peer project");
    expect(modalText()).not.toContain("Connected project");
    // Registering a folder writes through the *active* connection, so a machine
    // the phone is not on does not offer it here.
    expect(createWorkspaceRow()).toBeUndefined();
    // ...and creating names that machine, with the workspace read from it.
    const dialog = visibleModal()!;
    act(() => footer("sessions.new").props.onPress());
    await act(async () => { await Promise.resolve(); });
    expect(mockRemote.newConversationOn).not.toHaveBeenCalled();
    await act(async () => { dialog.props.onDismiss(); });
    // The selection moved with the machine: the workspace named is the other
    // machine's own, not the one the connected machine had selected.
    expect(mockRemote.newConversationOn).toHaveBeenCalledWith("desktop_2", "workspace", "pw1");
  });

  test("a machine that cannot take the conversation is listed, with why", () => {
    mockCatalogs.machines = [
      creatablePeer,
      { desktopId: "desktop_3", label: "Studio", active: false, status: "offline" },
    ];
    act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
    openDialog("workspace");
    // Paired but silent is still a machine the reader is looking for, so it is
    // shown and says which state it is in rather than being quietly absent.
    const offline = deviceRow("Studio");
    expect(offline.props.accessibilityState).toMatchObject({ disabled: true });
    expect(offline.findAllByType(Text).some(node => node.props.children === "connection.machine.offline"))
      .toBe(true);
    expect(offline.findAllByType(Check)).toHaveLength(0);
    expect(offline.findAllByType(ActivityIndicator)).toHaveLength(0);
  });

  test("the machine list is reachable again from the machine it named", async () => {
    openDialog("workspace");
    await act(async () => { deviceRow("Office").props.onPress(); });
    expect(modalText()).toContain("sessions.newOnDevice:Office");
    act(() => visibleModal()!.findAll(node =>
      node.props.accessibilityLabel === "common.back" && typeof node.props.onPress === "function",
    )[0]!.props.onPress());
    expect(modalText()).toContain("sessions.chooseDevice");
    expect(mockRemote.newConversation).not.toHaveBeenCalled();
  });

  test("a machine that cannot take the conversation says so, and creates nothing", async () => {
    jest.mocked(mockRemote.newConversationOn)
      .mockRejectedValueOnce(new Error("desktop_unavailable"));
    openDialog("chat");
    const dialog = visibleModal()!;
    await act(async () => { deviceRow("Office").props.onPress(); });
    await act(async () => { dialog.props.onDismiss(); });
    await act(async () => { await Promise.resolve(); });
    // The machine is named outright, so the machine — not the draft — is what
    // fails when it cannot be reached, and the alert holds the same wording the
    // open machine's own failures have always used.
    expect(modalText()).toContain("desktops.switchFailed");
    expect(mockRemote.newConversation).not.toHaveBeenCalled();
  });

  test("backing out while another machine's workspaces are being read advances nothing", async () => {
    // Reading a peer's workspaces is the one await left in the dialog, so an
    // abandoned choice is observable: it must neither step the dialog on nor
    // create a conversation.
    let finishRead: (workspaces: { id: string; name: string; path: string }[]) => void = () => {};
    jest.mocked(mockCatalogs.readWorkspaces).mockImplementationOnce(
      () => new Promise(resolve => { finishRead = resolve; }),
    );
    openDialog("workspace");
    act(() => { deviceRow("Office").props.onPress(); });
    act(() => visibleModal()!.findAll(node =>
      node.props.accessibilityLabel === "common.close" && typeof node.props.onPress === "function",
    )[0]!.props.onPress());
    await act(async () => { finishRead([{ id: "pw1", name: "Peer project", path: "/host/peer" }]); });
    await act(async () => { await Promise.resolve(); });
    expect(mockRemote.newConversationOn).not.toHaveBeenCalled();
    expect(visibleModal()).toBeUndefined();
  });

  test("one machine keeps the dialog it always had", () => {
    mockCatalogs.machines = [];
    act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
    openDialog("workspace");
    // Nothing to choose between, so the machine step never appears.
    expect(modalText()).not.toContain("sessions.chooseDevice");
    expect(modalText()).toContain("sessions.new");
  });
});

/**
 * The picker names a machine, and the list under it belongs to that machine.
 *
 * The list can only draw the machine the phone is reading and the machines that
 * answer, so a machine with nothing to draw is exactly where a screen is tempted
 * to fall back to something else — and one machine's conversations under another
 * machine's name is what made choosing an offline desktop look like the tap had
 * been ignored.
 */
describe("the machine the picker names", () => {
  const peer = {
    desktopId: "desktop_2",
    label: "Office",
    active: false,
    status: "offline" as "offline" | "connected",
  };

  beforeEach(() => {
    mockRemote.connectionPresentation = {
      level: "connected",
      titleKey: "connection.connected",
      hintKey: "connection.offlineHint",
    };
    mockRemote.desktopOnline = true;
  });

  afterEach(() => {
    mockCatalogs.machines = [];
    mockCatalogs.catalogs = [];
    mockCatalogs.commandsFor = jest.fn(() => null);
  });

  /** Name a machine in the picker, which is what tapping its row does. */
  const nameDesktop = (desktopId: string) =>
    act(() => tree.root.findByType(DesktopPicker).props.onChange({ kind: "desktop", desktopId }));
  /** The props the session list was drawn with, for the scope under test. */
  const list = () => tree.root.findByType(SessionList).props;
  /** Render the empty state the list was handed, so its own copy can be read. */
  const emptyState = () => {
    let renderer: ReactTestRenderer | null = null;
    act(() => { renderer = create(list().empty as ReactElement); });
    const rendered = renderer as unknown as ReactTestRenderer;
    return {
      /** The copy on screen, as the reader sees it: our mocked `t` is the key. */
      texts: rendered.root.findAllByType(Text).map(node => String(node.props.children)),
      root: rendered.root,
      unmount: () => act(() => rendered.unmount()),
    };
  };
  /** A machine's row in a dialog, addressed by the name the user sees. */
  const dialogRow = (label: string) =>
    tree.root.findAll(node =>
      node.props.accessibilityLabel === label
      && node.props.accessibilityRole === "button"
      && typeof node.props.onPress === "function",
    )[0]!;

  test.each(["offline", "connecting", "preparing"] as const)(
    "a machine that is %s is described in its own state, on its own connection",
    (status) => {
      mockCatalogs.machines = [{ ...peer, status }];
      act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
      nameDesktop("desktop_2");

      // An empty array of rows rather than none at all: the rows below belong to
      // the machine the picker names, of which there are none — not to the machine
      // the phone happens to be reading.
      expect(list().merged).toEqual([]);
      const empty = emptyState();
      expect(empty.texts.some(text => text.includes("Office"))).toBe(true);
      expect(empty.texts).toContain(`connection.machine.${status}`);
      expect(empty.texts).toContain("desktops.unreachableHint");
      // ...and the one action that can change that is on that machine, not on the
      // machine the phone is talking to.
      const reconnect = empty.root.findAllByType(Button)[0]!;
      expect(reconnect.props.label).toBe("connection.reconnect");
      act(() => reconnect.props.onPress());
      expect(mockRemote.reconnect).toHaveBeenCalledWith("desktop_2");
      empty.unmount();
    },
  );

  test("a machine that is answering keeps the ordinary empty list, addressed to it", () => {
    mockCatalogs.machines = [{ ...peer, status: "connected" }];
    mockCatalogs.commandsFor = jest.fn(() => ({ capabilities: new Set<string>() }));
    act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
    act(() => tree.root.findByType(SessionList).props.onTabChange("chat"));
    nameDesktop("desktop_2");

    expect(list().merged).toEqual([]);
    const empty = emptyState();
    expect(empty.texts).toContain("sessions.emptyConnectedTitle");
    // Starting one from this list names the machine the list is showing, rather
    // than the machine the phone reads, which is what the button's words say.
    act(() => empty.root.findAllByType(Button)[0]!.props.onPress());
    expect(mockRemote.newConversationOn).toHaveBeenCalledWith("desktop_2", "chat", "");
    expect(mockRemote.newConversation).not.toHaveBeenCalled();
    empty.unmount();
  });

  test("no new-conversation button is offered for a machine that cannot take one", () => {
    mockCatalogs.machines = [peer];
    act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
    nameDesktop("desktop_2");
    // The machine is named and unreachable: the list says so and offers to reach
    // it, and nothing on the screen offers to create on it.
    expect(button("sessions.new")).toBeUndefined();
    // The machine the phone is reading is not the one named, so its own button
    // is untouched.
    nameDesktop("desktop_1");
    expect(button("sessions.new")).toBeDefined();
  });

  test("a conversation started while another machine is named is created on it", () => {
    mockCatalogs.machines = [{ ...peer, status: "connected" }];
    mockCatalogs.commandsFor = jest.fn(() => ({ capabilities: new Set<string>() }));
    act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
    act(() => tree.root.findByType(SessionList).props.onTabChange("chat"));
    nameDesktop("desktop_2");

    act(() => button("sessions.new").props.onPress());
    expect(mockRemote.newConversationOn).toHaveBeenCalledWith("desktop_2", "chat", "");
    expect(mockRemote.newConversation).not.toHaveBeenCalled();
  });

  test("a workspace list for a named machine starts by confirming that machine", () => {
    mockCatalogs.machines = [{ ...peer, status: "connected" }];
    mockCatalogs.commandsFor = jest.fn(() => ({ capabilities: new Set<string>() }));
    act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
    act(() => tree.root.findByType(SessionList).props.onTabChange("workspace"));
    nameDesktop("desktop_2");

    act(() => button("sessions.new").props.onPress());
    // A workspace list is that machine's, and the step that reads it is the one
    // that names the machine — so the dialog opens there, with the machine the
    // list names already the one it is about.
    expect(modalText()).toContain("sessions.chooseDevice");
    expect(dialogRow("Office").props.accessibilityState).toMatchObject({ selected: true });
    expect(dialogRow("A very long desktop name").props.accessibilityState).toMatchObject({ selected: false });
  });

  test("a scope on a machine that is no longer paired goes back to every desktop", () => {
    const paired = [...mockRemote.desktops];
    try {
      nameDesktop("desktop_2");
      expect(tree.root.findByType(DesktopPicker).props.filter)
        .toEqual({ kind: "desktop", desktopId: "desktop_2" });
      mockRemote.desktops = [paired[0]!];
      act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
      // Nothing to show and nothing to reach: the list stands on every desktop
      // rather than on a machine the picker no longer offers.
      expect(tree.root.findByType(DesktopPicker).props.filter).toEqual({ kind: "all" });
    } finally {
      mockRemote.desktops = paired;
      act(() => tree.update(createElement(SessionsScreen, { onManageDesktops })));
    }
  });
});
