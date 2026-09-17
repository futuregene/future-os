import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ScrollView, SectionList, StyleSheet, Text } from "react-native";
import { Button } from "../../../components/Button";
import { SettingsScreen } from "../SettingsScreen";
import { SettingsLink, SettingsSwitch, settingsStyles } from "../SettingsPrimitives";
import type { DesktopSettings, InstalledSkill } from "../../../remote/types";

let stored: DesktopSettings;
let installed: InstalledSkill[];
const mockRemote = {
  credentials: { pairId: "pair", expectedDesktopId: "desktop" },
  desktops: [
    { pairId: "other", desktopId: "other-desktop", name: "Other computer" },
    { pairId: "pair", desktopId: "desktop", name: "Work computer" },
  ],
  desktopOnline: true,
  capabilities: new Set(["desktop_settings_v1", "skill_management_v1"]),
  desktopSettingsRevision: 0,
  skillsRevision: 0,
  approvalTier: "manual",
  sandboxAvailable: true,
  getDesktopSettings: jest.fn(async () => ({ ...stored })),
  updateDesktopSettings: jest.fn(async (patch: Partial<DesktopSettings>) => { stored = { ...stored, ...patch }; return stored; }),
  setApprovalTier: jest.fn(async () => {}),
  listSettingsModels: jest.fn(async () => [{ id: "same", provider: "one" }, { id: "same", provider: "two" }]),
  listInstalledSkills: jest.fn(async () => [...installed]),
  listAvailableSkills: jest.fn(async () => [{ id: "skill", name: "Skill", description: "", latestVersion: "1.2.0" }, { id: "new", name: "New", description: "", latestVersion: "1.0" }]),
  installSkill: jest.fn(async () => {}),
  uninstallSkill: jest.fn(async () => true),
};
jest.mock("../../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote }));
jest.mock("../../../i18n/LanguageSettings", () => ({ LanguageSettings: () => null }));
jest.mock("lucide-react-native", () => ({ ArrowLeft: "ArrowLeft", Monitor: "Monitor", ChevronRight: "ChevronRight" }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string, options?: { name?: string }) => options?.name ? `${key}: ${options.name}` : key, i18n: { language: "en" } }) }));

let tree: ReactTestRenderer;
const props = { onClose: jest.fn(), onCheckUpdate: jest.fn(), checkingUpdate: false };
const link = (label: string) => tree.root.findAllByType(SettingsLink).find(node => node.props.label === label)!;
const button = (label: string) => tree.root.findAllByType(Button).find(node => node.props.label === label)!;
const toggle = (label: string) => tree.root.findAllByType(SettingsSwitch).find(node => node.props.label === label)!;
async function openPreferences() { await act(async () => link("desktopSettings.preferences").props.onPress()); }
async function flush() { await act(async () => { await Promise.resolve(); }); }

beforeEach(async () => {
  jest.clearAllMocks();
  stored = { autoTitleFirstTurn: true, autoUpgradeSkills: true, autoConnectRemote: false, hiddenModels: ["one/same", "other/hidden"] };
  installed = [{ id: "skill", name: "Skill", description: "", version: "1.0.0" }];
  mockRemote.desktopOnline = true;
  mockRemote.capabilities = new Set(["desktop_settings_v1", "skill_management_v1"]);
  mockRemote.desktopSettingsRevision = 0;
  mockRemote.skillsRevision = 0;
  mockRemote.getDesktopSettings.mockImplementation(async () => ({ ...stored }));
  await act(async () => { tree = create(createElement(SettingsScreen, props)); });
});
afterEach(() => act(() => tree.unmount()));

test("groups scrollable desktop settings separately from phone preferences and saves to desktop", async () => {
  expect(tree.root.findAllByType(ScrollView).length).toBeGreaterThan(0);
  expect(tree.root.findAllByType(Text).map(node => node.props.children)).toContain("desktopSettings.currentDesktop");
  expect(tree.root.findAllByType(SettingsSwitch)).toHaveLength(0);
  expect(tree.root.findAllByType(Button)).toHaveLength(0);
  await openPreferences();
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.value).toBe(true);
  await act(async () => toggle("desktopSettings.autoTitleFirstTurn").props.onChange(false));
  expect(mockRemote.updateDesktopSettings).toHaveBeenCalledWith({ autoTitleFirstTurn: false });
  expect(stored.autoTitleFirstTurn).toBe(false);
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.value).toBe(false);
});

test("reloads a desktop-originated change", async () => {
  await openPreferences();
  stored.autoConnectRemote = true;
  mockRemote.desktopSettingsRevision++;
  await act(async () => tree.update(createElement(SettingsScreen, props)));
  expect(toggle("desktopSettings.autoConnectRemote").props.value).toBe(true);
});

test("offline and old desktops cannot write new preferences", async () => {
  mockRemote.desktopOnline = false;
  await act(async () => tree.update(createElement(SettingsScreen, props)));
  expect(link("desktopSettings.models").props.disabled).toBe(true);
  await openPreferences();
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.disabled).toBe(true);
  mockRemote.desktopOnline = true;
  mockRemote.capabilities.clear();
  await act(async () => tree.update(createElement(SettingsScreen, props)));
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.disabled).toBe(true);
  act(() => tree.root.findAll(node => node.props.accessibilityLabel === "common.back" && typeof node.props.onPress === "function")[0]!.props.onPress());
  expect(link("desktopSettings.skills").props.disabled).toBe(true);
  expect(mockRemote.updateDesktopSettings).not.toHaveBeenCalled();
});

test("visibility uses a separate virtualized all-models page and provider-qualified ids", async () => {
  await act(async () => link("desktopSettings.models").props.onPress());
  expect(tree.root.findAllByType(SectionList)).toHaveLength(1);
  expect(mockRemote.listSettingsModels).toHaveBeenCalled();
  const models = tree.root.findAllByType(SettingsSwitch).filter(node => node.props.description === "same");
  expect(models.map(node => node.props.value)).toEqual([false, true]);
  await act(async () => models[0]!.props.onChange(true));
  expect(mockRemote.updateDesktopSettings).toHaveBeenCalledWith({ hiddenModels: ["other/hidden"] });
});

test("skill operations target desktop and removal needs confirmation", async () => {
  await act(async () => link("desktopSettings.skills").props.onPress());
  expect(mockRemote.listInstalledSkills).toHaveBeenCalled();
  await act(async () => button("desktopSettings.upgrade").props.onPress());
  expect(mockRemote.installSkill).toHaveBeenCalledWith("skill", "1.2.0");
  act(() => button("desktopSettings.uninstall").props.onPress());
  expect(mockRemote.uninstallSkill).not.toHaveBeenCalled();
  await act(async () => button("desktopSettings.remove").props.onPress());
  expect(mockRemote.uninstallSkill).toHaveBeenCalledWith("skill");
  await act(async () => button("desktopSettings.available").props.onPress());
  await act(async () => button("desktopSettings.install").props.onPress());
  expect(mockRemote.installSkill).toHaveBeenCalledWith("new", "1.0");
});

test("failed reads leave mutations disabled instead of inventing mobile defaults", async () => {
  await openPreferences();
  mockRemote.getDesktopSettings.mockRejectedValueOnce(new Error("offline"));
  mockRemote.desktopSettingsRevision++;
  await act(async () => tree.update(createElement(SettingsScreen, props)));
  await flush();
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.disabled).toBe(true);
  expect(tree.root.findAllByType(Text).map(node => node.props.children)).toContain("desktopSettings.loadFailed");
  await act(async () => button("common.retry").props.onPress());
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.disabled).toBe(false);
  expect(tree.root.findAllByType(Button)).toHaveLength(0);
});

test("settings identifies its paired desktop without device selection or unpair actions", async () => {
  const labels = tree.root.findAllByType(SettingsLink).map(node => node.props.label);
  expect(labels).not.toContain("desktops.title");
  expect(labels).not.toContain("sessions.unpair");
  const scope = () => tree.root.findAllByType(Text).find(node => node.props.children === "desktopSettings.boundDesktop: Work computer");
  expect(scope()).toBeDefined();
  expect(scope()!.props.numberOfLines).toBeUndefined();
  await openPreferences();
  expect(scope()).toBeDefined();
});

test("phone preferences stay separate and update checking remains available", async () => {
  act(() => link("update.check").props.onPress());
  expect(props.onCheckUpdate).toHaveBeenCalledTimes(1);
  await act(async () => tree.update(createElement(SettingsScreen, { ...props, checkingUpdate: true })));
  expect(link("update.check").props.disabled).toBe(true);
  expect(link("update.check").props.loading).toBe(true);
  act(() => link("language.title").props.onPress());
  const text = tree.root.findAllByType(Text).map(node => node.props.children);
  expect(text).toContain("desktopSettings.language");
  expect(text).toContain("desktopSettings.thisPhone");
  expect(text).not.toContain("desktopSettings.boundDesktop: Work computer");
});

test("subpages use the shared back control and rounded card styling", async () => {
  const back = () => tree.root.findAll(node => node.props.accessibilityLabel === "common.back" && typeof node.props.onPress === "function")[0]!;
  const title = tree.root.findAllByType(Text).find(node => node.props.children === "sessions.settings")!;
  expect(StyleSheet.flatten(title.props.style)).toMatchObject({ fontSize: 22, fontWeight: "700" });
  expect(settingsStyles.row).toMatchObject({ borderRadius: 16, minHeight: 52 });
  await openPreferences();
  act(() => back().props.onPress());
  expect(link("desktopSettings.preferences")).toBeDefined();
  expect(props.onClose).not.toHaveBeenCalled();
  act(() => back().props.onPress());
  expect(props.onClose).toHaveBeenCalledTimes(1);
});

test("preferences auto-load without a persistent refresh button", async () => {
  await openPreferences();
  expect(mockRemote.getDesktopSettings).toHaveBeenCalledTimes(1);
  expect(tree.root.findAllByType(SettingsSwitch)).toHaveLength(3);
  expect(tree.root.findAllByType(Button)).toHaveLength(0);
});
