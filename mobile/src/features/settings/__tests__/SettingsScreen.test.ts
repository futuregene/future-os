import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ScrollView, SectionList, StyleSheet, Switch, Text, TextInput } from "react-native";
import { Button } from "../../../components/Button";
import { SettingsScreen, TITLED_SETTINGS_PAGES, type SettingsScreenHandle } from "../SettingsScreen";
import { resources } from "../../../i18n/locales";
import { SettingsLink, SettingsSwitch, useSettingsStyles } from "../SettingsPrimitives";
import { lightTheme } from "../../../theme/theme";
import type { DesktopSettings, InstalledSkill, PairedDesktop, ProvidersView } from "../../../remote/types";
import { PEER_ICONS } from "../../../remote/peerIcons";

let stored: DesktopSettings;
let installed: InstalledSkill[];
/** UI language the mocked `useTranslation` reports; flipped by language tests. */
let mockLanguage = "en";
/** The paired desktops the mocked context reports, in their stored order. */
const pairedDesktops = (): PairedDesktop[] => [
  { pairId: "other", desktopId: "other-desktop", name: "Other computer" },
  { pairId: "pair", desktopId: "desktop", name: "Work computer" },
];
/**
 * Apply a label the way the real store does: the name and the icon are
 * independent, and an empty value clears its field rather than storing "".
 */
function relabel(desktopId: string, patch: { name?: string; icon?: string }): void {
  mockRemote.desktops = mockRemote.desktops.map((item) => {
    if (item.desktopId !== desktopId) return item;
    const name = patch.name === undefined ? item.name : patch.name;
    const icon = patch.icon === undefined ? item.icon : patch.icon;
    return {
      desktopId: item.desktopId,
      pairId: item.pairId,
      ...(name ? { name } : {}),
      ...(icon ? { icon } : {}),
    };
  });
}
const mockRemote = {
  credentials: { pairId: "pair", expectedDesktopId: "desktop" },
  // Typed as the real thing: a desktop row carries the label the user gave it
  // (name, icon), and an untyped literal here silently loses the right to set
  // them — which a test that writes an icon cannot do without.
  desktops: pairedDesktops(),
  desktopOnline: true,
  // The provider's own rule: reads need the machine to be online *and* to be
  // this phone's to drive.
  get desktopReadable() {
    return this.desktopOnline && (this.presence?.control ?? "active") !== "standby";
  },
  presence: null as { control?: "active" | "standby"; holder?: { name?: string } | null } | null,
  takeControl: jest.fn(async () => {}),
  capabilities: new Set(["desktop_settings_v1", "skill_management_v1", "tasks_v1"]),
  desktopSettingsRevision: 0,
  skillsRevision: 0,
  approvalTier: "manual",
  sandboxAvailable: true,
  getDesktopSettings: jest.fn(async () => ({ ...stored })),
  updateDesktopSettings: jest.fn(async (patch: Partial<DesktopSettings>) => { stored = { ...stored, ...patch }; return stored; }),
  setApprovalTier: jest.fn(async () => {}),
  switchDesktop: jest.fn(async () => {}),
  renameDesktop: jest.fn(async (desktopId: string, name: string) => relabel(desktopId, { name })),
  labelDesktop: jest.fn(async (desktopId: string, patch: { name?: string; icon?: string }) => relabel(desktopId, patch)),
  listSettingsModels: jest.fn(async () => [{ id: "same", provider: "one" }, { id: "same", provider: "two" }]),
  listProviders: jest.fn(async () => providers),
  listInstalledSkills: jest.fn(async () => [...installed]),
  listAvailableSkills: jest.fn(async () => [{ id: "skill", name: "Skill", description: "", latestVersion: "1.2.0" }, { id: "new", name: "New", description: "", latestVersion: "1.0" }]),
  installSkill: jest.fn(async () => {}),
  uninstallSkill: jest.fn(async () => true),
};
jest.mock("../../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote }));
jest.mock("../../../i18n/LanguageSettings", () => ({ LanguageSettings: () => null }));
jest.mock("../TasksSettingsPage", () => ({ TasksSettingsPage: (props: { desktopOnline: boolean; settings: unknown }) => { tasksPageProps = props as never; return null; } }));
jest.mock("lucide-react-native", () => ({ ArrowLeft: "ArrowLeft", Monitor: "Monitor", Check: "Check", ChevronDown: "ChevronDown", ChevronRight: "ChevronRight" }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView", useSafeAreaInsets: () => ({ top: 24, bottom: 34, left: 0, right: 0 }) }));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string, options?: { name?: string }) => options?.name ? `${key}: ${options.name}` : key, i18n: { get language() { return mockLanguage; } } }) }));

let tree: ReactTestRenderer;
let providers: ProvidersView;
let tasksPageProps: {
  desktopOnline: boolean;
  settings: unknown;
  onBackLevel?(handle: { goBack(): boolean } | null): void;
  onOpenConversation?(sessionId: string): void;
} | null = null;
const props = { onClose: jest.fn(), onCheckUpdate: jest.fn(), checkingUpdate: false };
/**
 * The same stack opened for one paired desktop.
 *
 * Every desktop-scoped level (preferences, models, providers, skills, tasks)
 * now lives under a device page, so this is the surface most of this file
 * drives; `props` without a `deviceId` is the phone's own screen.
 */
const deviceProps = { ...props, deviceId: "desktop" };
const link = (label: string) => tree.root.findAllByType(SettingsLink).find(node => node.props.label === label)!;
const button = (label: string) => tree.root.findAllByType(Button).find(node => node.props.label === label)!;
const toggle = (label: string) => tree.root.findAllByType(SettingsSwitch).find(node => node.props.label === label)!;
const input = (label: string) => tree.root.findAllByType(TextInput).find(node => node.props.accessibilityLabel === label)!;
const type = async (label: string, text: string) => act(async () => input(label).props.onChangeText(text));
/** Provider headings on the model-visibility page, in render order (the
 * Pressable and the view it renders both carry the props, so de-duplicate). */
const headings = () => [...new Set(tree.root
  .findAll(node => node.props.accessibilityRole === "button" && node.props.accessibilityState?.expanded !== undefined)
  .map(node => node.props.accessibilityLabel as string))];
/** A provider heading: the pressable that folds its models. */
const group = (title: string) => tree.root.findAll(node => node.props.accessibilityLabel === title && node.props.accessibilityRole === "button")[0]!;
const modelSwitches = () => tree.root.findAllByType(SettingsSwitch).filter(node => node.props.description === "same");
async function openPreferences() { await act(async () => link("desktopSettings.preferences").props.onPress()); }
async function flush() { await act(async () => { await Promise.resolve(); }); }

beforeEach(async () => {
  jest.clearAllMocks();
  stored = { autoTitleFirstTurn: true, autoUpgradeSkills: true, autoConnectRemote: false, hiddenModels: ["one/same", "other/hidden"] };
  installed = [{ id: "skill", name: "Skill", description: "", version: "1.0.0" }];
  mockLanguage = "en";
  mockRemote.desktops = pairedDesktops();
  mockRemote.credentials = { pairId: "pair", expectedDesktopId: "desktop" };
  mockRemote.desktopOnline = true;
  mockRemote.presence = null;
  mockRemote.takeControl = jest.fn(async () => {});
  mockRemote.capabilities = new Set(["desktop_settings_v1", "skill_management_v1", "tasks_v1"]);
  tasksPageProps = null;
  mockRemote.desktopSettingsRevision = 0;
  mockRemote.skillsRevision = 0;
  mockRemote.getDesktopSettings.mockImplementation(async () => ({ ...stored }));
  mockRemote.listSettingsModels.mockImplementation(async () => [{ id: "same", provider: "one" }, { id: "same", provider: "two" }]);
  mockRemote.listProviders.mockImplementation(async () => providers);
  providers = {
    // `one` is deliberately keyless: the page must list it anyway.
    builtin: [
      { id: "one", name: "One", baseUrl: "https://one.example.com/v1", hasApiKey: false, modelCount: 1, requiresBaseUrl: false },
      { id: "two", name: "Two", baseUrl: "https://two.example.com/v1", hasApiKey: true, modelCount: 1, requiresBaseUrl: false },
    ],
    custom: [],
  };
  await act(async () => { tree = create(createElement(SettingsScreen, deviceProps)); });
});
afterEach(() => act(() => tree.unmount()));

// A machine another device is driving refuses every read, so the page used to
// answer "无法读取桌面端数据，请重试" over a desktop that was working perfectly — just
// not for this phone. It says what is happening and offers the one action that
// changes it, and it does not ask for data it knows will be refused.
test("a taken-over desktop says so and offers the takeover instead of a read failure", async () => {
  mockRemote.presence = { control: "standby", holder: { name: "Studio laptop" } };
  mockRemote.desktopOnline = true;
  mockRemote.getDesktopSettings.mockClear();

  await act(async () => { tree = create(createElement(SettingsScreen, deviceProps)); });
  await flush();

  expect(strings()).toContain("connection.controlHeldBy: Studio laptop");
  expect(strings()).toContain("connection.standbyPageHint");
  // No read was attempted against a machine that would refuse it, so there is
  // no "could not read the desktop" banner to explain away.
  expect(mockRemote.getDesktopSettings).not.toHaveBeenCalled();
  expect(strings()).not.toContain("desktopSettings.loadFailed");

  await act(async () => button("connection.takeControl").props.onPress());
  expect(mockRemote.takeControl).toHaveBeenCalledTimes(1);
});

test("a refused takeover is reported beside the button, not swallowed", async () => {
  mockRemote.presence = { control: "standby", holder: { name: "Studio laptop" } };
  mockRemote.takeControl = jest.fn(async () => { throw new Error("device_offline"); });
  await act(async () => { tree = create(createElement(SettingsScreen, deviceProps)); });
  await flush();

  await act(async () => button("connection.takeControl").props.onPress());
  await flush();
  expect(strings()).toContain("connection.takeControlFailed");
});

test("the device page configures the desktop, and keeps the phone's own rows out", async () => {
  expect(tree.root.findAllByType(ScrollView).length).toBeGreaterThan(0);
  const keys = () => tree.root.findAllByType(Text).map(node => node.props.children);
  // What this device is called here, and what it does, are two sections of one
  // page: the first is the phone's own label, the second is read from the
  // machine.
  expect(keys()).toContain("desktops.identity");
  expect(keys()).toContain("desktops.deviceSettings");
  // The phone's own settings are a different screen now: device settings must
  // not smuggle in rows that write locally.
  expect(link("language.title")).toBeUndefined();
  expect(link("update.check")).toBeUndefined();
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
  stored.autoUpgradeSkills = false;
  mockRemote.desktopSettingsRevision++;
  await act(async () => tree.update(createElement(SettingsScreen, deviceProps)));
  expect(toggle("desktopSettings.autoUpgradeSkills").props.value).toBe(false);
});

/**
 * The remote desktop switch is not the phone's to offer.
 *
 * On that machine the Remote page's switch *is* the whole preference, and this
 * page would write the value the host reads at launch — so its own switch keeps
 * showing the old answer until a restart, and one state would read two ways.
 * The preference stays entirely on the desktop; the phone only shows the
 * preferences that take effect at once.
 */
test("does not offer the remote desktop switch", async () => {
  await openPreferences();
  expect(toggle("desktopSettings.autoConnectRemote")).toBeUndefined();
  expect(strings()).not.toContain("desktopSettings.autoConnectRemote");
  expect(tree.root.findAllByType(SettingsSwitch)).toHaveLength(3);
});

test("offline and old desktops cannot write new preferences", async () => {
  mockRemote.desktopOnline = false;
  await act(async () => tree.update(createElement(SettingsScreen, deviceProps)));
  expect(link("desktopSettings.models").props.disabled).toBe(true);
  await openPreferences();
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.disabled).toBe(true);
  mockRemote.desktopOnline = true;
  mockRemote.capabilities.clear();
  await act(async () => tree.update(createElement(SettingsScreen, deviceProps)));
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.disabled).toBe(true);
  act(() => tree.root.findAll(node => node.props.accessibilityLabel === "common.back" && typeof node.props.onPress === "function")[0]!.props.onPress());
  expect(link("desktopSettings.skills").props.disabled).toBe(true);
  expect(mockRemote.updateDesktopSettings).not.toHaveBeenCalled();
});

test("visibility groups models under a foldable provider heading", async () => {
  mockRemote.capabilities = new Set(["desktop_settings_v1", "skill_management_v1", "provider_management_v1"]);
  await act(async () => tree.update(createElement(SettingsScreen, deviceProps)));
  await act(async () => link("desktopSettings.models").props.onPress());
  expect(tree.root.findAllByType(SectionList)).toHaveLength(1);
  expect(mockRemote.listSettingsModels).toHaveBeenCalled();
  expect(mockRemote.listProviders).toHaveBeenCalled();
  // Every provider the desktop offers is grouped, key or no key, under the
  // desktop's display name — the models are already the desktop's scoped list.
  expect(headings()).toEqual(["One", "Two"]);
  expect(group("Two").props.accessibilityState.expanded).toBe(true);
  expect(modelSwitches().map(node => node.props.value)).toEqual([false, true]);

  await act(async () => group("Two").props.onPress());
  expect(group("Two").props.accessibilityState.expanded).toBe(false);
  expect(modelSwitches().map(node => node.props.value)).toEqual([false]);
  expect(group("One").props.accessibilityState.expanded).toBe(true);
  // The bulk switch stays on the heading and hides every model it covers.
  const bulk = tree.root.findAllByType(Switch).find(node => node.props.accessibilityLabel === "desktopSettings.toggleProvider: Two")!;
  await act(async () => bulk.props.onValueChange(false));
  expect(mockRemote.updateDesktopSettings).toHaveBeenCalledWith({ hiddenModels: ["one/same", "other/hidden", "two/same"] });
});

test("a keyless provider stays listed: the desktop already scoped the model list", async () => {
  mockRemote.capabilities = new Set(["desktop_settings_v1", "skill_management_v1", "provider_management_v1"]);
  // A local endpoint the agent can call without a key is reported as-is, and a
  // keyless built-in stays whatever the desktop decided to report.
  providers = {
    builtin: [{ id: "one", name: "One", baseUrl: "https://one.example.com/v1", hasApiKey: false, modelCount: 2, requiresBaseUrl: false }],
    custom: [{ id: "local", name: "Local Ollama", api: "openai-completions", baseUrl: "http://127.0.0.1:11434/v1", hasApiKey: false, models: [] }],
  };
  mockRemote.listSettingsModels.mockImplementation(async () => [
    { id: "same", provider: "one" },
    { id: "llama", provider: "local" },
  ]);
  await act(async () => tree.update(createElement(SettingsScreen, deviceProps)));
  await act(async () => link("desktopSettings.models").props.onPress());
  await flush();
  expect(headings()).toEqual(["Local Ollama", "One"]);
  // Both groups are open and both models are switchable, with the stored
  // visibility applied per provider-qualified reference.
  expect(tree.root.findAllByType(SettingsSwitch).map(node => [node.props.description, node.props.value]))
    .toEqual([["llama", true], ["same", false]]);
});

test("a search narrows to matching providers, opens them, and reports no match", async () => {
  await act(async () => link("desktopSettings.models").props.onPress());
  // Without the capability the names fall back to provider ids.
  expect(headings()).toEqual(["one", "two"]);
  await type("desktopSettings.searchModels", "one");
  // Only matching providers are left, and a match is always shown: folding is
  // suspended while a query is active.
  expect(headings()).toEqual(["one"]);
  expect(group("one").props.accessibilityState).toMatchObject({ disabled: true, expanded: true });
  expect(modelSwitches()).toHaveLength(1);
  await type("desktopSettings.searchModels", "nothing-matches");
  expect(tree.root.findAllByType(Text).map(node => node.props.children)).toContain("desktopSettings.noModels");
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

test("skill text follows the system language, borrowing the catalogue's translation", async () => {
  // Installed skills come from SKILL.md and usually ship English text only, so
  // the zh row falls back to the catalogue's pair for the same id; a skill that
  // carries its own zh text (or has no catalogue entry) keeps what it has.
  installed = [
    { id: "future-image", name: "future-image", description: "Generate and edit images.", version: "1.2.0" },
    { id: "side-loaded", name: "side-loaded", description: "Local skill.", descriptionZh: "本地技能", version: null },
  ];
  mockRemote.listAvailableSkills.mockImplementation(async () => [
    { id: "future-image", name: "future-image", description: "Generate and edit images.", nameZh: "图像生成与编辑", descriptionZh: "生成、编辑与分析图像", latestVersion: "1.2.0" },
  ]);
  const text = () => tree.root.findAllByType(Text).map(node => node.props.children);
  mockLanguage = "zh";
  await act(async () => link("desktopSettings.skills").props.onPress());
  expect(text()).toEqual(expect.arrayContaining(["图像生成与编辑", "生成、编辑与分析图像", "side-loaded", "本地技能"]));
  expect(text()).not.toContain("Generate and edit images.");
  // The page stays open, so a re-render is enough to read the new language.
  mockLanguage = "en";
  await act(async () => tree.update(createElement(SettingsScreen, deviceProps)));
  expect(text()).toEqual(expect.arrayContaining(["future-image", "Generate and edit images.", "Local skill."]));
  expect(text()).not.toContain("生成、编辑与分析图像");
  expect(text()).not.toContain("本地技能");
});

test("failed reads leave mutations disabled instead of inventing mobile defaults", async () => {
  await openPreferences();
  mockRemote.getDesktopSettings.mockRejectedValueOnce(new Error("offline"));
  mockRemote.desktopSettingsRevision++;
  await act(async () => tree.update(createElement(SettingsScreen, deviceProps)));
  await flush();
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.disabled).toBe(true);
  expect(tree.root.findAllByType(Text).map(node => node.props.children)).toContain("desktopSettings.loadFailed");
  await act(async () => button("common.retry").props.onPress());
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.disabled).toBe(false);
  expect(tree.root.findAllByType(Button)).toHaveLength(0);
});

test("the device page names the machine it configures, and offers no unpair action", async () => {
  const labels = tree.root.findAllByType(SettingsLink).map(node => node.props.label);
  expect(labels).not.toContain("desktops.title");
  expect(labels).not.toContain("sessions.unpair");
  // The page *is* this machine, so its header says which one — truncated rather
  // than wrapped, because a long label must not push the levels down.
  const heading = () => tree.root.findAllByType(Text).find(node => node.props.children === "Work computer");
  expect(heading()).toBeDefined();
  expect(heading()!.props.numberOfLines).toBe(1);
  // One level down the header names the level instead, so the machine comes
  // back as the banner above it.
  await openPreferences();
  expect(heading()).toBeUndefined();
  expect(strings()).toContain("desktopSettings.boundDesktop: Work computer");
});

/**
 * The banner names the machine these pages write to, so it wears that machine's
 * own icon: the fixed monitor glyph here was the same gap the device list had,
 * where picking an icon had no visible effect anywhere.
 */
test("the scoped banner wears the bound desktop's own icon", async () => {
  await openPreferences();
  expect(strings()).toContain(PEER_ICONS[0]!.glyph);
  mockRemote.desktops = [
    { pairId: "other", desktopId: "other-desktop", name: "Other computer" },
    { pairId: "pair", desktopId: "desktop", name: "Work computer", icon: "rocket" },
  ];
  await act(async () => tree.update(createElement(SettingsScreen, deviceProps)));
  expect(strings()).toContain("🚀");
});

test("the phone's own screen holds no device settings", async () => {
  await remountPhone();
  // Every machine-scoped row moved to that machine's page: this screen must
  // stay usable with nothing paired.
  expect(link("desktopSettings.preferences")).toBeUndefined();
  expect(link("desktopSettings.models")).toBeUndefined();
  expect(link("desktopSettings.providers")).toBeUndefined();
  expect(link("desktopSettings.skills")).toBeUndefined();
  expect(link("desktopSettings.tasks")).toBeUndefined();
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

test("every titled settings level has a header in both languages", () => {
  // A level whose title key is missing does not fail loudly: the header renders
  // the key itself (`desktopSettings.appearance`), which is how the appearance
  // page first shipped. The header is built from `desktopSettings.<name>`, so
  // hold that key to the shipped translations rather than to a mock's echo.
  for (const page of TITLED_SETTINGS_PAGES) {
    for (const language of ["en", "zh"] as const) {
      const title = translations(language)[`desktopSettings.${page}`];
      expect({ page, language, title }).toEqual({ page, language, title: expect.any(String) });
      expect(title).not.toBe(`desktopSettings.${page}`);
      expect(title).not.toBe("");
    }
  }
});

function translations(language: "en" | "zh"): Record<string, string> {
  const flat: Record<string, string> = {};
  const walk = (value: object, prefix = "") => {
    for (const [key, entry] of Object.entries(value)) {
      const path = prefix ? `${prefix}.${key}` : key;
      if (typeof entry === "string") flat[path] = entry;
      else if (entry && typeof entry === "object") walk(entry, path);
    }
  };
  walk(resources[language].translation);
  return flat;
}

test("the appearance level is reachable from the phone's own screen", async () => {
  await remountPhone();
  await act(async () => link("appearance.title").props.onPress());
  const radioLabels = tree.root
    .findAll(node => node.props.accessibilityRole === "radio" && typeof node.props.onPress === "function")
    .map(node => node.props.accessibilityLabel);
  // Every level here is phone-local, so they live on the phone's own screen —
  // and all of them are one tap from it, not buried under the device's page.
  expect(radioLabels).toEqual([
    "appearance.themeMode.system", "appearance.themeMode.light", "appearance.themeMode.dark",
    "appearance.fontSizeName.small", "appearance.fontSizeName.medium", "appearance.fontSizeName.large", "appearance.fontSizeName.xlarge",
    "appearance.replyDetail.default", "appearance.replyDetail.detailed",
  ]);
  expect(radioLabels).not.toContain("desktopSettings.boundDesktop: Work computer");
});

test("subpages use the shared back control and rounded card styling", async () => {
  await remountPhone();
  const back = () => tree.root.findAll(node => node.props.accessibilityLabel === "common.back" && typeof node.props.onPress === "function")[0]!;
  const title = tree.root.findAllByType(Text).find(node => node.props.children === "sessions.settings")!;
  expect(StyleSheet.flatten(title.props.style)).toMatchObject({ fontSize: 22, fontWeight: "700" });
  expect(useSettingsStyles.forTheme(lightTheme).row).toMatchObject({ borderRadius: 16, minHeight: 52 });
  await act(async () => link("language.title").props.onPress());
  act(() => back().props.onPress());
  expect(link("desktopSettings.followAccount")).toBeDefined();
  expect(props.onClose).not.toHaveBeenCalled();
  act(() => back().props.onPress());
  expect(props.onClose).toHaveBeenCalledTimes(1);
});

test("preferences auto-load without a persistent refresh button", async () => {
  await openPreferences();
  expect(mockRemote.getDesktopSettings).toHaveBeenCalledTimes(1);
  // auto-upgrade, auto-title, skill recommendation. The remote desktop switch
  // is the desktop's own (its Remote page), so the phone does not offer it.
  expect(tree.root.findAllByType(SettingsSwitch)).toHaveLength(3);
  expect(tree.root.findAllByType(Button)).toHaveLength(0);
});

test("the official-account entry stays reachable while the desktop is offline", async () => {
  // Following an account needs no connection, so a regression that adds a
  // `disabled` here would strand the entry exactly when the user has no desktop
  // at hand. Inspect the Pressable the row renders — that is what decides
  // whether a tap goes through — rather than the SettingsLink element's props.
  await remountPhone();
  const row = () => tree.root.findAll(node =>
    node.props.accessibilityLabel === "desktopSettings.followAccount"
    && node.props.accessibilityRole === "button")[0]!;
  expect(row().props.disabled).toBe(false);
  expect(row().props.accessibilityState.disabled).toBe(false);

  act(() => link("desktopSettings.followAccount").props.onPress());
  expect(tree.root.findAllByType(Text).map(node => node.props.children))
    .toContain("desktopSettings.followAccount");

  // Now take the desktop away and confirm the row is still usable.
  mockRemote.desktopOnline = false;
  await act(async () => { tree = create(createElement(SettingsScreen, props)); });
  expect(row().props.disabled).toBe(false);
  expect(row().props.accessibilityState.disabled).toBe(false);
});

const texts = () => tree.root.findAllByType(Text).map(node => node.props.children);
/** The string children only: a text node may hold an interpolated element. */
const strings = () => texts().filter((child): child is string => typeof child === "string");
const radio = (tier: string) => tree.root.findAll(node =>
  node.props.accessibilityRole === "radio" && node.props.accessibilityLabel === tier)[0]!;
/** Fresh mount: `useDesktopResource` only re-reads when its inputs change, so a
 * test that needs a *failed* read has to remount rather than `update`. */
async function remount() {
  act(() => tree.unmount());
  await act(async () => { tree = create(createElement(SettingsScreen, deviceProps)); });
}
/** Fresh mount of the phone's own screen, which is what `props` describes. */
async function remountPhone() {
  act(() => tree.unmount());
  await act(async () => { tree = create(createElement(SettingsScreen, props)); });
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}

test("each preference switch writes its own field and nothing else", async () => {
  await openPreferences();
  await act(async () => toggle("desktopSettings.autoUpgradeSkills").props.onChange(false));
  expect(mockRemote.updateDesktopSettings).toHaveBeenLastCalledWith({ autoUpgradeSkills: false });
  await act(async () => toggle("desktopSettings.skillRecommend").props.onChange(false));
  expect(mockRemote.updateDesktopSettings).toHaveBeenLastCalledWith({ skillRecommend: false });
  // Each write re-read the desktop, so the switches show the stored values.
  expect(mockRemote.getDesktopSettings).toHaveBeenCalledTimes(3);
  expect(stored).toMatchObject({ autoUpgradeSkills: false, skillRecommend: false });
});

test("a refused preference write is reported, and the page still re-reads the desktop", async () => {
  await openPreferences();
  mockRemote.updateDesktopSettings.mockRejectedValueOnce(new Error("desktop busy"));
  await act(async () => toggle("desktopSettings.autoTitleFirstTurn").props.onChange(false));
  expect(texts()).toContain("desktopSettings.saveFailed");
  // The read-back is deliberate: a lost reply can still follow a successful
  // save, so the page must not guess which side failed.
  expect(mockRemote.getDesktopSettings).toHaveBeenCalledTimes(2);
  // A later successful write clears the notice.
  await act(async () => toggle("desktopSettings.autoTitleFirstTurn").props.onChange(true));
  expect(texts()).not.toContain("desktopSettings.saveFailed");
});

test("a preference switch cannot write while the desktop is unreachable", async () => {
  await openPreferences();
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.disabled).toBe(false);
  mockRemote.desktopOnline = false;
  // The page is already mounted; a disconnect re-renders it rather than
  // remounting, which is the state a tap racing the disconnect sees.
  await act(async () => { tree.update(createElement(SettingsScreen, deviceProps)); });
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.disabled).toBe(true);
  // A tap that races the disconnect must not queue a write the desktop will
  // apply once it reconnects.
  await act(async () => toggle("desktopSettings.autoTitleFirstTurn").props.onChange(false));
  expect(mockRemote.updateDesktopSettings).not.toHaveBeenCalled();
});

test("an approval tier cannot be written while the desktop is unreachable", async () => {
  await openPreferences();
  mockRemote.desktopOnline = false;
  await act(async () => { tree.update(createElement(SettingsScreen, deviceProps)); });
  // The tier radios stay tappable (the page explains the offline state), so
  // the guard — not a disabled prop — is what stops the write.
  await act(async () => radio("approvalTier.sandbox").props.onPress());
  expect(mockRemote.setApprovalTier).not.toHaveBeenCalled();
});

test("a second tier tap while the first write is in flight writes once", async () => {
  await openPreferences();
  const pending = deferred<void>();
  mockRemote.setApprovalTier.mockReturnValueOnce(pending.promise);
  await act(async () => { void radio("approvalTier.sandbox").props.onPress(); });
  expect(mockRemote.setApprovalTier).toHaveBeenCalledTimes(1);
  // The row is still pressable until the reply lands; a second tap must not
  // queue another write, or the desktop ends up with the tier of the last tap
  // while the UI shows the first.
  await act(async () => radio("approvalTier.off").props.onPress());
  expect(mockRemote.setApprovalTier).toHaveBeenCalledTimes(1);
  expect(mockRemote.setApprovalTier).toHaveBeenCalledWith("sandbox");
  await act(async () => { pending.resolve(); });
});

test("choosing an approval tier writes it and reports a refusal", async () => {
  await openPreferences();
  expect(radio("approvalTier.manual").props.accessibilityState.checked).toBe(true);
  await act(async () => radio("approvalTier.sandbox").props.onPress());
  expect(mockRemote.setApprovalTier).toHaveBeenCalledWith("sandbox");
  expect(mockRemote.setApprovalTier).toHaveBeenCalledTimes(1);
  mockRemote.setApprovalTier.mockRejectedValueOnce(new Error("denied"));
  await act(async () => radio("approvalTier.off").props.onPress());
  expect(texts()).toContain("desktopSettings.saveFailed");
});

test("the models page retries its own read from its notice", async () => {
  mockRemote.capabilities = new Set(["desktop_settings_v1", "skill_management_v1", "provider_management_v1"]);
  // Both reads the page depends on fail: the settings snapshot it edits and the
  // model catalogue it lists.
  mockRemote.getDesktopSettings.mockRejectedValueOnce(new Error("offline"));
  mockRemote.listSettingsModels.mockRejectedValueOnce(new Error("offline"));
  await remount();
  await act(async () => link("desktopSettings.models").props.onPress());
  expect(texts()).toContain("desktopSettings.loadFailed");
  mockRemote.getDesktopSettings.mockClear();
  mockRemote.listSettingsModels.mockClear();
  // The page notice and the list header each own one of those reads.
  const retries = tree.root.findAllByType(Button).filter(node => node.props.label === "common.retry");
  expect(retries).toHaveLength(2);
  await act(async () => { for (const retry of retries) retry.props.onPress(); await Promise.resolve(); });
  expect(mockRemote.getDesktopSettings).toHaveBeenCalledTimes(1);
  expect(mockRemote.listSettingsModels).toHaveBeenCalledTimes(1);
  expect(texts()).not.toContain("desktopSettings.loadFailed");
});

test("a per-model switch and a folded group both write the desktop's hidden set", async () => {
  mockRemote.capabilities = new Set(["desktop_settings_v1", "skill_management_v1", "provider_management_v1"]);
  await remount();
  await act(async () => link("desktopSettings.models").props.onPress());
  // "one/same" is stored hidden; switching it back on removes just that one.
  await act(async () => modelSwitches()[0]!.props.onChange(true));
  expect(mockRemote.updateDesktopSettings).toHaveBeenLastCalledWith({ hiddenModels: ["other/hidden"] });
  await act(async () => group("One").props.onPress());
  expect(group("One").props.accessibilityState.expanded).toBe(false);
  // Folding a group twice is idempotent in the end state: the second tap opens
  // it again rather than leaving a stale "folded" entry behind.
  await act(async () => group("One").props.onPress());
  expect(group("One").props.accessibilityState.expanded).toBe(true);
});

test("a model switch cannot write before the desktop's settings have loaded", async () => {
  mockRemote.capabilities = new Set(["desktop_settings_v1", "skill_management_v1", "provider_management_v1"]);
  // The model list arrives, but the settings snapshot it edits does not: the
  // rows are visible and the switch must refuse rather than write a partial set.
  mockRemote.getDesktopSettings.mockRejectedValue(new Error("offline"));
  await remount();
  await act(async () => link("desktopSettings.models").props.onPress());
  expect(modelSwitches()).toHaveLength(2);
  await act(async () => modelSwitches()[0]!.props.onChange(false));
  expect(mockRemote.updateDesktopSettings).not.toHaveBeenCalled();
});

test("upgrading every skill installs each upgrade exactly once, in catalogue order", async () => {
  installed = [
    { id: "skill", name: "Skill", description: "", version: "1.0.0" },
    { id: "other", name: "Other", description: "", version: "1.0.0" },
  ];
  mockRemote.listAvailableSkills.mockImplementation(async () => [
    { id: "skill", name: "Skill", description: "", latestVersion: "1.2.0" },
    { id: "other", name: "Other", description: "", latestVersion: "2.0.0" },
  ]);
  await act(async () => link("desktopSettings.skills").props.onPress());
  await flush();
  // Two installed skills are behind the catalogue; a skill with no installed
  // version is a fresh install, not an upgrade, so it is not in this batch.
  expect(button("desktopSettings.upgradeAll").props.disabled).toBe(false);
  await act(async () => { button("desktopSettings.upgradeAll").props.onPress(); await Promise.resolve(); });
  expect(jest.mocked(mockRemote.installSkill).mock.calls).toEqual([
    ["skill", "1.2.0"],
    ["other", "2.0.0"],
  ]);
  // The installed list is re-read once for the batch, not once per skill.
  expect(mockRemote.listInstalledSkills).toHaveBeenCalledTimes(2);
});

test("a second skill mutation during the upgrade batch is ignored", async () => {
  installed = [
    { id: "skill", name: "Skill", description: "", version: "1.0.0" },
    { id: "other", name: "Other", description: "", version: "1.0.0" },
  ];
  mockRemote.listAvailableSkills.mockImplementation(async () => [
    { id: "skill", name: "Skill", description: "", latestVersion: "1.2.0" },
    { id: "other", name: "Other", description: "", latestVersion: "2.0.0" },
  ]);
  await act(async () => link("desktopSettings.skills").props.onPress());
  await flush();
  const first = deferred<void>();
  mockRemote.installSkill.mockReturnValueOnce(first.promise);
  // The batch is running and its first install is unresolved; a single-skill
  // action tapped now must not start a second mutation against the same page.
  const upgrade = button("desktopSettings.upgrade").props.onPress;
  await act(async () => {
    button("desktopSettings.upgradeAll").props.onPress();
    // The page's single-skill action is tapped while the batch's first install
    // is still unresolved: it must not start a second mutation on the page.
    upgrade();
    await Promise.resolve();
  });
  expect(mockRemote.installSkill.mock.calls).toEqual([["skill", "1.2.0"]]);
  await act(async () => { first.resolve(); await Promise.resolve(); });
  // The batch then continues on its own to the second skill; the ignored tap is
  // still not a call of its own, so "skill" is installed exactly once.
  expect(mockRemote.installSkill.mock.calls).toEqual([
    ["skill", "1.2.0"],
    ["other", "2.0.0"],
  ]);
});

test("closing the skills page stops the remaining upgrade batch", async () => {
  installed = [
    { id: "skill", name: "Skill", description: "", version: "1.0.0" },
    { id: "other", name: "Other", description: "", version: "1.0.0" },
  ];
  mockRemote.listAvailableSkills.mockImplementation(async () => [
    { id: "skill", name: "Skill", description: "", latestVersion: "1.2.0" },
    { id: "other", name: "Other", description: "", latestVersion: "2.0.0" },
  ]);
  await act(async () => link("desktopSettings.skills").props.onPress());
  await flush();
  const first = deferred<void>();
  mockRemote.installSkill.mockReturnValueOnce(first.promise);
  await act(async () => { button("desktopSettings.upgradeAll").props.onPress(); await Promise.resolve(); });
  expect(mockRemote.installSkill).toHaveBeenCalledTimes(1);
  // The page closes while the first install is still in flight; the operations
  // authorized for it must not carry over to whatever the user opens next.
  act(() => tree.unmount());
  await act(async () => { first.resolve(); await Promise.resolve(); });
  expect(mockRemote.installSkill).toHaveBeenCalledTimes(1);
});

test("the skills page retries both reads and reports a refused removal", async () => {
  mockRemote.listInstalledSkills.mockRejectedValueOnce(new Error("offline"));
  await act(async () => link("desktopSettings.skills").props.onPress());
  await flush();
  expect(texts()).toContain("desktopSettings.loadFailed");
  mockRemote.listInstalledSkills.mockClear();
  mockRemote.listAvailableSkills.mockClear();
  await act(async () => { button("common.retry").props.onPress(); await Promise.resolve(); });
  expect(mockRemote.listInstalledSkills).toHaveBeenCalledTimes(1);
  expect(mockRemote.listAvailableSkills).toHaveBeenCalledTimes(1);
  expect(texts()).not.toContain("desktopSettings.loadFailed");
  // A desktop that declines the removal must not be reported as removed.
  mockRemote.uninstallSkill.mockResolvedValueOnce(false);
  act(() => button("desktopSettings.uninstall").props.onPress());
  await act(async () => { button("desktopSettings.remove").props.onPress(); await Promise.resolve(); });
  expect(texts()).toContain("desktopSettings.skillFailed");
  // Backing out of the confirmation arms nothing.
  mockRemote.uninstallSkill.mockClear();
  act(() => button("desktopSettings.uninstall").props.onPress());
  await act(async () => button("chat.cancel").props.onPress());
  expect(texts()).not.toContain("desktopSettings.confirmRemove: Skill");
  expect(mockRemote.uninstallSkill).not.toHaveBeenCalled();
});

test("switching to the installed tab keeps the catalogue rows reachable", async () => {
  await act(async () => link("desktopSettings.skills").props.onPress());
  await flush();
  await act(async () => button("desktopSettings.available").props.onPress());
  await flush();
  expect(button("desktopSettings.install")).toBeDefined();
  await act(async () => button("desktopSettings.installed").props.onPress());
  await flush();
  expect(button("desktopSettings.upgrade")).toBeDefined();
  expect(button("desktopSettings.install")).toBeUndefined();
});

test("task management is gated by the desktop's capability and opens the page", async () => {
  expect(link("desktopSettings.tasks").props.disabled).toBe(false);

  // An older desktop that never declared tasks_v1 cannot open the page.
  mockRemote.capabilities = new Set(["desktop_settings_v1", "skill_management_v1"]);
  await act(async () => tree.update(createElement(SettingsScreen, deviceProps)));
  expect(link("desktopSettings.tasks").props.disabled).toBe(true);
  expect(tree.root.findAllByType(Text).map(node => node.props.children)).toContain("desktopSettings.updateDesktop");

  // Once the desktop declares it, the entry opens the page and hands it the
  // connection state and the desktop's settings (the task form's model list is
  // the user's enabled models) it needs.
  mockRemote.capabilities = new Set(["desktop_settings_v1", "skill_management_v1", "tasks_v1"]);
  await act(async () => tree.update(createElement(SettingsScreen, deviceProps)));
  await act(async () => link("desktopSettings.tasks").props.onPress());
  expect(tasksPageProps).toMatchObject({ desktopOnline: true });
  expect(tasksPageProps!.settings).toBeDefined();
});

// The system back gesture and the header arrow both call the stack's `goBack`.
// A page with levels of its own registers one, and it must consume the step
// first: swiping out of an open task returns to the task list, not to the
// settings home.
test("asks the open page to leave a level before popping the route", async () => {
  mockRemote.desktopOnline = true;
  mockRemote.capabilities = new Set(["desktop_settings_v1", "skill_management_v1", "tasks_v1"]);
  // The stack's `goBack` is the handle the modal's back gesture calls, so the
  // test drives the same entry point the phone does.
  const handle: { current: SettingsScreenHandle | null } = { current: null };
  await act(async () => tree.update(createElement(SettingsScreen, { ...deviceProps, ref: handle })));
  await act(async () => link("desktopSettings.tasks").props.onPress());

  // The page registers how to leave its own level; here it reports that it
  // consumed the step (as it does while a task is open).
  let asked = 0;
  await act(async () => tasksPageProps!.onBackLevel!({ goBack: () => { asked += 1; return true; } }));
  await act(async () => handle.current!.goBack());
  expect(asked).toBe(1);
  // Still on Tasks: the route was not popped.
  expect(tasksPageProps).not.toBeNull();

  // With nothing open the page declines, and the stack pops to the settings home.
  await act(async () => tasksPageProps!.onBackLevel!({ goBack: () => false }));
  await act(async () => handle.current!.goBack());
  expect(asked).toBe(1);
  expect(link("desktopSettings.tasks")).toBeDefined();
});

test("task management is unavailable while the desktop is offline", async () => {
  mockRemote.desktopOnline = false;
  await act(async () => tree.update(createElement(SettingsScreen, deviceProps)));
  expect(link("desktopSettings.tasks").props.disabled).toBe(true);
});

/** The device page's identity field. */
const nameField = () => tree.root.findAllByType(TextInput).find(node => node.props.accessibilityLabel === "desktops.name")!;
/** The identity grid's option for one icon. */
const iconOption = (label: string) => tree.root.findAll(node =>
  node.props.accessibilityLabel === label && node.props.accessibilityRole === "radio")[0]!;
/** Re-render the way the provider re-renders its consumers after a label write:
 * in the app, `labelDesktop` refreshes the desktop list in context state, and
 * the page reads the device back. The mocked context holds no state of its own,
 * so the refresh is spelled out here. */
async function refresh() {
  await act(async () => { tree.update(createElement(SettingsScreen, deviceProps)); });
}
/** Fresh mount of one paired device's page. */
async function mountDevice(deviceId: string) {
  act(() => tree.unmount());
  await act(async () => { tree = create(createElement(SettingsScreen, { ...props, deviceId })); });
}

test("the device page writes the name the user types, on the done key and on leaving the field", async () => {
  await act(async () => nameField().props.onChangeText("Studio"));
  await act(async () => nameField().props.onSubmitEditing());
  expect(mockRemote.renameDesktop).toHaveBeenCalledWith("desktop", "Studio");
  // One write for one edit: the done key dismisses the keyboard, which blurs the
  // field as well.
  expect(mockRemote.renameDesktop).toHaveBeenCalledTimes(1);
  await act(async () => nameField().props.onBlur());
  expect(mockRemote.renameDesktop).toHaveBeenCalledTimes(1);
});

test("an unchanged name writes nothing and a blank one clears the label", async () => {
  await act(async () => nameField().props.onChangeText("Work computer"));
  await act(async () => nameField().props.onBlur());
  expect(mockRemote.renameDesktop).not.toHaveBeenCalled();
  // Blank means "no name": the device goes back to being listed by its id.
  await act(async () => nameField().props.onChangeText("   "));
  await act(async () => nameField().props.onBlur());
  expect(mockRemote.renameDesktop).toHaveBeenCalledWith("desktop", "");
  expect(mockRemote.desktops[1]!.name).toBeUndefined();
});

test("a refused name write is reported, and the field goes back to the stored name", async () => {
  mockRemote.renameDesktop.mockRejectedValueOnce(new Error("storage full"));
  await act(async () => nameField().props.onChangeText("Studio"));
  await act(async () => nameField().props.onSubmitEditing());
  expect(texts()).toContain("desktops.actionFailed");
  // The field must never show a name the store does not have.
  expect(nameField().props.value).toBe("Work computer");
});

test("the icon is picked on the device page, and picking it again clears it", async () => {
  await act(async () => iconOption("desktops.icons.rocket").props.onPress());
  expect(mockRemote.labelDesktop).toHaveBeenCalledWith("desktop", { icon: "rocket" });
  await refresh();
  expect(iconOption("desktops.icons.rocket").props.accessibilityState.checked).toBe(true);
  // Toggle: there is no "none" entry in the grid, so the way back to the
  // default is picking the icon the device is already wearing.
  await act(async () => iconOption("desktops.icons.rocket").props.onPress());
  expect(mockRemote.labelDesktop).toHaveBeenLastCalledWith("desktop", { icon: "" });
  await refresh();
  expect(iconOption("desktops.icons.rocket").props.accessibilityState.checked).toBe(false);
});

test("a refused icon write is reported", async () => {
  mockRemote.labelDesktop.mockRejectedValueOnce(new Error("storage full"));
  await act(async () => iconOption("desktops.icons.rocket").props.onPress());
  expect(texts()).toContain("desktops.actionFailed");
});

/**
 * A machine's settings are read and written over that machine's own connection,
 * which it keeps while the phone reads another one (see `DesktopScope`). So a
 * device page has no connection step to perform: the rows are there when it
 * opens, whichever machine is the one the phone happens to be reading, and
 * nothing about opening it moves the phone.
 */
test("a device page reads the machine it names, with no connection step", async () => {
  await mountDevice("other-desktop");
  expect(link("desktopSettings.preferences")).toBeDefined();
  expect(link("desktopSettings.models")).toBeDefined();
  expect(link("desktops.connect")).toBeUndefined();
  expect(texts()).not.toContain("desktops.connectHint");
  // Reading a machine is not choosing it: which machine the phone is reading
  // changes where a conversation is opened, not on a settings page.
  expect(mockRemote.switchDesktop).not.toHaveBeenCalled();
  expect(mockRemote.updateDesktopSettings).not.toHaveBeenCalled();
});

/**
 * A machine that is not answering cannot be configured — its settings are read
 * from it, not from a local copy — and the page says which machine and why,
 * rather than drawing rows that would write somewhere else.
 */
test("a machine that is not answering disables its rows and says so", async () => {
  mockRemote.desktopOnline = false;
  mockRemote.getDesktopSettings.mockClear();
  await mountDevice("other-desktop");
  expect(texts()).toContain("desktopSettings.offline");
  expect(link("desktopSettings.models").props.disabled).toBe(true);
  expect(link("desktopSettings.providers").props.disabled).toBe(true);
  expect(mockRemote.getDesktopSettings).not.toHaveBeenCalled();
});

test("automatic review is offered only by capable desktops with a working sandbox", async () => {
  await openPreferences();
  expect(tree.root.findAllByProps({ accessibilityLabel: "approvalTier.auto" })).toHaveLength(0);
  mockRemote.capabilities.add("auto_approval_v1");
  await act(async () => { tree.update(createElement(SettingsScreen, deviceProps)); });
  await act(async () => radio("approvalTier.auto").props.onPress());
  expect(mockRemote.setApprovalTier).toHaveBeenCalledWith("auto");
  mockRemote.sandboxAvailable = false;
  await act(async () => { tree.update(createElement(SettingsScreen, deviceProps)); });
  expect(tree.root.findAllByProps({ accessibilityLabel: "approvalTier.auto" })).toHaveLength(0);
});
