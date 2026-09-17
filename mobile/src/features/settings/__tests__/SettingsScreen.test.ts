import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ScrollView, SectionList, Text } from "react-native";
import { Button } from "../../../components/Button";
import { SettingsScreen } from "../SettingsScreen";
import { SettingsLink, SettingsSwitch } from "../SettingsPrimitives";
import type { DesktopSettings, InstalledSkill } from "../../../remote/types";

let stored: DesktopSettings;
let installed: InstalledSkill[];
const mockRemote = {
  credentials: { pairId: "pair", expectedDesktopId: "desktop" },
  desktops: [{ pairId: "pair", desktopId: "desktop", name: "Work computer" }],
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
jest.mock("lucide-react-native", () => ({ ChevronLeft: "ChevronLeft", ChevronRight: "ChevronRight", X: "X" }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key, i18n: { language: "en" } }) }));

let tree: ReactTestRenderer;
const props = { onClose: jest.fn(), onManageDesktops: jest.fn(), onCheckUpdate: jest.fn(), onUnpair: jest.fn(), checkingUpdate: false };
const link = (label: string) => tree.root.findAllByType(SettingsLink).find(node => node.props.label === label)!;
const button = (label: string) => tree.root.findAllByType(Button).find(node => node.props.label === label)!;
const toggle = (label: string) => tree.root.findAllByType(SettingsSwitch).find(node => node.props.label === label)!;
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
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.value).toBe(true);
  await act(async () => toggle("desktopSettings.autoTitleFirstTurn").props.onChange(false));
  expect(mockRemote.updateDesktopSettings).toHaveBeenCalledWith({ autoTitleFirstTurn: false });
  expect(stored.autoTitleFirstTurn).toBe(false);
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.value).toBe(false);
});

test("reloads a desktop-originated change", async () => {
  stored.autoConnectRemote = true;
  mockRemote.desktopSettingsRevision++;
  await act(async () => tree.update(createElement(SettingsScreen, props)));
  expect(toggle("desktopSettings.autoConnectRemote").props.value).toBe(true);
});

test("offline and old desktops cannot write new preferences", async () => {
  mockRemote.desktopOnline = false;
  await act(async () => tree.update(createElement(SettingsScreen, props)));
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.disabled).toBe(true);
  expect(link("desktopSettings.models").props.disabled).toBe(true);
  mockRemote.desktopOnline = true;
  mockRemote.capabilities.clear();
  await act(async () => tree.update(createElement(SettingsScreen, props)));
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.disabled).toBe(true);
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
  mockRemote.getDesktopSettings.mockRejectedValueOnce(new Error("offline"));
  mockRemote.desktopSettingsRevision++;
  await act(async () => tree.update(createElement(SettingsScreen, props)));
  await flush();
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.disabled).toBe(true);
  expect(tree.root.findAllByType(Text).map(node => node.props.children)).toContain("desktopSettings.loadFailed");
});
