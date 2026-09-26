import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { ScrollView, SectionList, StyleSheet, Switch, Text, TextInput } from "react-native";
import { Button } from "../../../components/Button";
import { SettingsScreen } from "../SettingsScreen";
import { SettingsLink, SettingsSwitch, settingsStyles } from "../SettingsPrimitives";
import type { DesktopSettings, InstalledSkill, ProvidersView } from "../../../remote/types";

let stored: DesktopSettings;
let installed: InstalledSkill[];
/** UI language the mocked `useTranslation` reports; flipped by language tests. */
let mockLanguage = "en";
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
  listProviders: jest.fn(async () => providers),
  listInstalledSkills: jest.fn(async () => [...installed]),
  listAvailableSkills: jest.fn(async () => [{ id: "skill", name: "Skill", description: "", latestVersion: "1.2.0" }, { id: "new", name: "New", description: "", latestVersion: "1.0" }]),
  installSkill: jest.fn(async () => {}),
  uninstallSkill: jest.fn(async () => true),
};
jest.mock("../../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote }));
jest.mock("../../../i18n/LanguageSettings", () => ({ LanguageSettings: () => null }));
jest.mock("lucide-react-native", () => ({ ArrowLeft: "ArrowLeft", Monitor: "Monitor", ChevronDown: "ChevronDown", ChevronRight: "ChevronRight" }));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));
jest.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string, options?: { name?: string }) => options?.name ? `${key}: ${options.name}` : key, i18n: { get language() { return mockLanguage; } } }) }));

let tree: ReactTestRenderer;
let providers: ProvidersView;
const props = { onClose: jest.fn(), onCheckUpdate: jest.fn(), checkingUpdate: false };
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
  mockRemote.desktopOnline = true;
  mockRemote.capabilities = new Set(["desktop_settings_v1", "skill_management_v1"]);
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

test("visibility groups models under a foldable provider heading", async () => {
  mockRemote.capabilities = new Set(["desktop_settings_v1", "skill_management_v1", "provider_management_v1"]);
  await act(async () => tree.update(createElement(SettingsScreen, props)));
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
  await act(async () => tree.update(createElement(SettingsScreen, props)));
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
  await act(async () => tree.update(createElement(SettingsScreen, props)));
  expect(text()).toEqual(expect.arrayContaining(["future-image", "Generate and edit images.", "Local skill."]));
  expect(text()).not.toContain("生成、编辑与分析图像");
  expect(text()).not.toContain("本地技能");
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
  // auto-upgrade, auto-title, skill recommendation, auto-connect.
  expect(tree.root.findAllByType(SettingsSwitch)).toHaveLength(4);
  expect(tree.root.findAllByType(Button)).toHaveLength(0);
});

test("the official-account entry stays reachable while the desktop is offline", async () => {
  // Every other row on "This phone" is gated on the desktop connection; following
  // an account needs no connection, so a regression that adds a `disabled` here
  // would strand the entry exactly when the user has no desktop at hand.
  // Inspect the Pressable the row renders — that is what decides whether a tap
  // goes through — rather than the SettingsLink element's own props.
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
const radio = (tier: string) => tree.root.findAll(node =>
  node.props.accessibilityRole === "radio" && node.props.accessibilityLabel === tier)[0]!;
/** Fresh mount: `useDesktopResource` only re-reads when its inputs change, so a
 * test that needs a *failed* read has to remount rather than `update`. */
async function remount() {
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
  await act(async () => toggle("desktopSettings.autoConnectRemote").props.onChange(true));
  expect(mockRemote.updateDesktopSettings).toHaveBeenLastCalledWith({ autoConnectRemote: true });
  // Each write re-read the desktop, so the switches show the stored values.
  expect(mockRemote.getDesktopSettings).toHaveBeenCalledTimes(4);
  expect(stored).toMatchObject({ autoUpgradeSkills: false, skillRecommend: false, autoConnectRemote: true });
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
  await act(async () => { tree.update(createElement(SettingsScreen, props)); });
  expect(toggle("desktopSettings.autoTitleFirstTurn").props.disabled).toBe(true);
  // A tap that races the disconnect must not queue a write the desktop will
  // apply once it reconnects.
  await act(async () => toggle("desktopSettings.autoTitleFirstTurn").props.onChange(false));
  expect(mockRemote.updateDesktopSettings).not.toHaveBeenCalled();
});

test("an approval tier cannot be written while the desktop is unreachable", async () => {
  await openPreferences();
  mockRemote.desktopOnline = false;
  await act(async () => { tree.update(createElement(SettingsScreen, props)); });
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
