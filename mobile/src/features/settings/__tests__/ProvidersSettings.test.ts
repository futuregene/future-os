import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Text, TextInput } from "react-native";
import { Button } from "../../../components/Button";
import { SettingsLink, SettingsSection } from "../SettingsPrimitives";
import { SettingsScreen } from "../SettingsScreen";
import { ActionMenu } from "../../../components/ActionMenu";
import type { ProvidersView } from "../../../remote/types";

let providers: ProvidersView;

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>(yes => { resolve = yes; });
  return { promise, resolve };
}

const mockRemote = {
  credentials: { pairId: "pair", expectedDesktopId: "desktop" },
  desktops: [{ pairId: "pair", desktopId: "desktop", name: "Work computer" }],
  desktopOnline: true,
  capabilities: new Set(["desktop_settings_v1", "skill_management_v1", "provider_management_v1"]),
  desktopSettingsRevision: 0,
  skillsRevision: 0,
  approvalTier: "manual",
  sandboxAvailable: true,
  getDesktopSettings: jest.fn(async () => ({
    autoTitleFirstTurn: true,
    autoUpgradeSkills: true,
    autoConnectRemote: false,
    hiddenModels: [],
  })),
  updateDesktopSettings: jest.fn(async () => providers),
  setApprovalTier: jest.fn(async () => {}),
  listSettingsModels: jest.fn(async () => []),
  listInstalledSkills: jest.fn(async () => []),
  listAvailableSkills: jest.fn(async () => []),
  installSkill: jest.fn(async () => {}),
  uninstallSkill: jest.fn(async () => true),
  listProviders: jest.fn(async () => providers),
  updateBuiltinProvider: jest.fn(async () => providers),
  upsertCustomProvider: jest.fn(async () => providers),
  deleteCustomProvider: jest.fn(async () => providers),
};
jest.mock("../../../remote/RemoteContext", () => ({ useRemoteControls: () => mockRemote }));
jest.mock("../../../i18n/LanguageSettings", () => ({ LanguageSettings: () => null }));
jest.mock("lucide-react-native", () => ({
  ArrowLeft: "ArrowLeft",
  Monitor: "Monitor",
  ChevronRight: "ChevronRight",
  ChevronDown: "ChevronDown",
  Trash2: "Trash2",
  // ActionMenu (the API-type picker and the delete confirmation) needs these.
  Search: "Search",
  X: "X",
}));
jest.mock("react-native-safe-area-context", () => ({ SafeAreaView: "SafeAreaView" }));
jest.mock("react-i18next", () => ({
  useTranslation: () => ({
    t: (key: string, options?: Record<string, unknown>) => (options?.name ? `${key}: ${options.name}` : key),
    i18n: { language: "en" },
  }),
}));

let tree: ReactTestRenderer;
const props = { onClose: jest.fn(), onCheckUpdate: jest.fn(), checkingUpdate: false };
const link = (label: string) => tree.root.findAllByType(SettingsLink).find(node => node.props.label === label)!;
const button = (label: string) => tree.root.findAllByType(Button).find(node => node.props.label === label)!;
const input = (label: string) => tree.root.findAllByType(TextInput).find(node => node.props.accessibilityLabel === label)!;
const pressable = (label: string) => tree.root.findAll(node => node.props.accessibilityLabel === label && typeof node.props.onPress === "function")[0]!;
const back = () => tree.root.findAll(node => node.props.accessibilityLabel === "common.back" && typeof node.props.onPress === "function")[0]!;
const section = (title: string) => tree.root.findAllByType(SettingsSection).find(node => node.props.title === title)!;
const sectionTitles = () => tree.root.findAllByType(SettingsSection).map(node => node.props.title);
/** Row names of the built-in section, in render order (descriptions are i18n keys). */
const builtinNames = () => section("desktopSettings.builtinProviders").findAllByType(Text)
  .map(node => node.props.children)
  .filter((text): text is string => typeof text === "string" && !text.startsWith("desktopSettings."));
const rendered = () => tree.root.findAllByType(Text).map(node => node.props.children);
const type = async (label: string, text: string) => act(async () => input(label).props.onChangeText(text));

async function openProviders() {
  await act(async () => link("desktopSettings.providers").props.onPress());
  await act(async () => { await Promise.resolve(); });
}

beforeEach(async () => {
  jest.clearAllMocks();
  mockRemote.desktopOnline = true;
  mockRemote.capabilities = new Set(["desktop_settings_v1", "skill_management_v1", "provider_management_v1"]);
  providers = {
    builtin: [
      { id: "future", name: "Future", baseUrl: "https://future-os.cn/api/v1", hasApiKey: true, modelCount: 9, requiresBaseUrl: false },
      { id: "deepseek", name: "DeepSeek", baseUrl: "https://api.deepseek.com/v1", hasApiKey: true, modelCount: 3, requiresBaseUrl: false },
      { id: "azure", name: "Azure OpenAI", baseUrl: "https://YOUR_RESOURCE.openai.azure.com/openai", hasApiKey: false, modelCount: 1, requiresBaseUrl: true },
    ],
    custom: [{
      id: "acme",
      name: "Acme Gateway",
      api: "openai-completions",
      baseUrl: "https://gateway.acme.example.com/v1",
      hasApiKey: true,
      models: [{ id: "acme-large", name: "Acme Large", supportsImages: true, reasoning: true, contextWindow: 128000, maxTokens: 16384, inputCost: 1.5, outputCost: 6, cacheReadCost: 0, cacheWriteCost: 0 }],
    }],
  };
  await act(async () => { tree = create(createElement(SettingsScreen, props)); });
});
afterEach(() => act(() => tree.unmount()));

test("lists the desktop's built-in and custom providers, with the account provider read-only", async () => {
  await openProviders();
  expect(mockRemote.listProviders).toHaveBeenCalledTimes(1);
  expect(link("DeepSeek").props.description).toContain("desktopSettings.providerKeySet");
  expect(link("Acme Gateway").props.description).toContain("desktopSettings.providerKeySet");
  // The sign-in provider has no editor: it renders as a plain, unpressable row.
  expect(link("Future")).toBeUndefined();
  expect(rendered()).toContain("desktopSettings.providerManaged");
});

test("leads with custom providers, puts keyed built-ins first, and folds the tail", async () => {
  const provider = (id: string, name: string, hasApiKey: boolean) =>
    ({ id, name, baseUrl: `https://${id}.example.com/v1`, hasApiKey, modelCount: 2, requiresBaseUrl: false });
  providers = {
    // Catalog order deliberately unlike the expected render order.
    builtin: [
      provider("anthropic", "Anthropic", false),
      provider("deepseek", "DeepSeek", true),
      provider("google", "Google", false),
      provider("future", "Future", true),
      provider("kimi-coding", "Kimi", true),
      provider("moonshotai", "Moonshot", false),
      provider("openai", "OpenAI", false),
      provider("zhipuai", "Zhipu", false),
    ],
    custom: [{ id: "acme", name: "Acme Gateway", api: "openai-completions", baseUrl: "https://gateway.acme.example.com/v1", hasApiKey: true, models: [] }],
  };
  await openProviders();

  expect(sectionTitles()).toEqual(["desktopSettings.customProviders", "desktopSettings.builtinProviders"]);
  // Future first, then the keyed providers, then the rest in catalog order.
  expect(builtinNames()).toEqual(["Future", "DeepSeek", "Kimi", "Anthropic", "Google"]);
  expect(button("desktopSettings.showMoreBuiltin")).toBeDefined();

  await act(async () => button("desktopSettings.showMoreBuiltin").props.onPress());
  expect(builtinNames()).toEqual(["Future", "DeepSeek", "Kimi", "Anthropic", "Google", "Moonshot", "OpenAI", "Zhipu"]);
  expect(button("desktopSettings.showMoreBuiltin")).toBeUndefined();

  await act(async () => button("desktopSettings.hideMoreBuiltin").props.onPress());
  expect(builtinNames()).toEqual(["Future", "DeepSeek", "Kimi", "Anthropic", "Google"]);
});

test("a short built-in list has nothing to fold", async () => {
  await openProviders();
  expect(builtinNames()).toEqual(["Future", "DeepSeek", "Azure OpenAI"]);
  expect(button("desktopSettings.showMoreBuiltin")).toBeUndefined();
  expect(button("desktopSettings.hideMoreBuiltin")).toBeUndefined();
});

test("writes a built-in key atomically and returns to the list", async () => {
  await openProviders();
  await act(async () => link("DeepSeek").props.onPress());
  expect(input("desktopSettings.apiKey")).toBeDefined();
  await type("desktopSettings.apiKey", "  sk-live  ");
  await act(async () => { button("desktopSettings.save").props.onPress(); await Promise.resolve(); });
  expect(mockRemote.updateBuiltinProvider).toHaveBeenCalledWith({ id: "deepseek", apiKey: "sk-live", updateApiKey: true });
  // Back on the list, which re-read the desktop's view.
  expect(mockRemote.listProviders).toHaveBeenCalledTimes(2);
});

test("a second built-in key save cannot overlap the write already in flight", async () => {
  await openProviders();
  await act(async () => link("DeepSeek").props.onPress());
  await type("desktopSettings.apiKey", "sk-live");
  const write = deferred<ProvidersView>();
  mockRemote.updateBuiltinProvider.mockReturnValueOnce(write.promise);
  await act(async () => { button("desktopSettings.save").props.onPress(); await Promise.resolve(); });
  expect(mockRemote.updateBuiltinProvider).toHaveBeenCalledTimes(1);
  // The button is disabled while busy, but a tap already in the pipe must not
  // start a second write of the same key while the first is unresolved.
  await act(async () => button("desktopSettings.save").props.onPress());
  expect(mockRemote.updateBuiltinProvider).toHaveBeenCalledTimes(1);
  await act(async () => { write.resolve(providers); await Promise.resolve(); });
  // The page only pops once the write lands.
  expect(mockRemote.updateBuiltinProvider).toHaveBeenCalledTimes(1);
});

test("a refused key write shows the desktop's own reason and does not pop the page", async () => {
  await openProviders();
  await act(async () => link("DeepSeek").props.onPress());
  // A non-Error rejection still has to reach the user as text: the desktop can
  // reject with a bare string.
  mockRemote.updateBuiltinProvider.mockRejectedValueOnce("catalog rejected the key");
  await type("desktopSettings.apiKey", "sk-bad");
  await act(async () => { button("desktopSettings.save").props.onPress(); await Promise.resolve(); });
  expect(rendered()).toContain("catalog rejected the key");
  // A failed write must not report success, so the page stays put: the key
  // field is still the visible surface rather than the provider list.
  expect(input("desktopSettings.apiKey")).toBeDefined();
  expect(link("DeepSeek")).toBeUndefined();
  // A second attempt is allowed once the failure has been reported.
  await act(async () => { button("desktopSettings.save").props.onPress(); await Promise.resolve(); });
  expect(mockRemote.updateBuiltinProvider).toHaveBeenCalledTimes(2);
});

test("a failed provider read offers a retry that re-reads the desktop", async () => {
  mockRemote.listProviders.mockRejectedValueOnce(new Error("offline"));
  await openProviders();
  expect(rendered()).toContain("desktopSettings.loadFailed");
  mockRemote.listProviders.mockClear();
  await act(async () => { button("common.retry").props.onPress(); await Promise.resolve(); });
  expect(mockRemote.listProviders).toHaveBeenCalledTimes(1);
  expect(rendered()).not.toContain("desktopSettings.loadFailed");
  expect(link("DeepSeek")).toBeDefined();
});

test("clears a stored key and refuses an empty key where one is required", async () => {
  await openProviders();
  await act(async () => link("DeepSeek").props.onPress());
  await act(async () => { button("desktopSettings.clearApiKey").props.onPress(); await Promise.resolve(); });
  expect(mockRemote.updateBuiltinProvider).toHaveBeenCalledWith({ id: "deepseek", apiKey: null, updateApiKey: true });

  await act(async () => link("DeepSeek").props.onPress());
  await act(async () => button("desktopSettings.save").props.onPress());
  expect(rendered()).toContain("desktopSettings.apiKeyRequired");
  expect(mockRemote.updateBuiltinProvider).toHaveBeenCalledTimes(1);
});

test("a placeholder provider requires its Base URL and keeps an untouched key", async () => {
  await openProviders();
  await act(async () => link("Azure OpenAI").props.onPress());
  expect(input("desktopSettings.baseUrl").props.value).toBe("");
  await act(async () => button("desktopSettings.save").props.onPress());
  expect(rendered()).toContain("desktopSettings.baseUrlRequired");

  await type("desktopSettings.baseUrl", "https://acme.openai.azure.com/openai");
  await act(async () => { button("desktopSettings.save").props.onPress(); await Promise.resolve(); });
  expect(mockRemote.updateBuiltinProvider).toHaveBeenCalledWith({
    id: "azure",
    baseUrl: "https://acme.openai.azure.com/openai",
    updateApiKey: false,
  });
});

test("adds a custom provider with models and prices", async () => {
  await openProviders();
  await act(async () => button("desktopSettings.addProvider").props.onPress());
  await type("desktopSettings.providerId", "Acme!");
  await act(async () => button("desktopSettings.save").props.onPress());
  expect(rendered()).toContain("desktopSettings.providerIdPattern");
  expect(mockRemote.upsertCustomProvider).not.toHaveBeenCalled();

  await type("desktopSettings.providerId", "newco");
  await type("desktopSettings.providerName", "New Co");
  await type("desktopSettings.baseUrl", "https://api.newco.example.com/v1");
  await type("desktopSettings.apiKey", "sk-new");
  await act(async () => pressable("desktopSettings.addModel").props.onPress());
  await type("desktopSettings.modelId", "newco-large");
  await type("desktopSettings.modelPriceInput", "2");
  await type("desktopSettings.modelPriceOutput", "8");
  await act(async () => { button("desktopSettings.save").props.onPress(); await Promise.resolve(); });

  expect(mockRemote.upsertCustomProvider).toHaveBeenCalledWith(expect.objectContaining({
    id: "newco",
    name: "New Co",
    api: "openai-completions",
    baseUrl: "https://api.newco.example.com/v1",
    apiKey: "sk-new",
    create: true,
    models: [expect.objectContaining({
      id: "newco-large",
      contextWindow: 128000,
      maxTokens: 16384,
      inputCost: 2,
      outputCost: 8,
      cacheReadCost: 0,
    })],
  }));
});

test("editing keeps the provider id locked and deleting needs confirmation", async () => {
  await openProviders();
  await act(async () => link("Acme Gateway").props.onPress());
  expect(input("desktopSettings.providerId").props.editable).toBe(false);
  expect(input("desktopSettings.providerId").props.value).toBe("acme");
  expect(input("desktopSettings.apiKey").props.value).toBe("");

  await act(async () => button("desktopSettings.deleteProvider").props.onPress());
  expect(mockRemote.deleteCustomProvider).not.toHaveBeenCalled();
  await act(async () => { button("desktopSettings.deleteProvider").props.onPress(); await Promise.resolve(); });
  expect(mockRemote.deleteCustomProvider).toHaveBeenCalledWith("acme");
  expect(mockRemote.listProviders).toHaveBeenCalledTimes(2);
});

test("editing a model rejects limits and prices the desktop would refuse", async () => {
  await openProviders();
  await act(async () => link("Acme Gateway").props.onPress());
  await act(async () => pressable("Acme Large").props.onPress());
  await type("desktopSettings.modelMaxTokens", "200000");
  await act(async () => button("desktopSettings.save").props.onPress());
  expect(rendered()).toContain("desktopSettings.modelMaxTokensExceedsContext");

  await type("desktopSettings.modelMaxTokens", "4096");
  await type("desktopSettings.modelPriceInput", "-1");
  await act(async () => button("desktopSettings.save").props.onPress());
  expect(rendered()).toContain("desktopSettings.modelPriceInvalid");
  expect(mockRemote.upsertCustomProvider).not.toHaveBeenCalled();
});

test("the new-provider form refuses each shape the desktop would reject", async () => {
  await openProviders();
  await act(async () => button("desktopSettings.addProvider").props.onPress());
  // No id at all, then an id outside the desktop's 2..40 character window.
  await act(async () => button("desktopSettings.save").props.onPress());
  expect(rendered()).toContain("desktopSettings.providerIdRequired");
  await type("desktopSettings.providerId", "a");
  await act(async () => button("desktopSettings.save").props.onPress());
  expect(rendered()).toContain("desktopSettings.providerIdLength");
  // A syntactically valid id with an address the desktop cannot parse.
  await type("desktopSettings.providerId", "newco");
  await type("desktopSettings.baseUrl", "not-a-url");
  await act(async () => button("desktopSettings.save").props.onPress());
  expect(rendered()).toContain("desktopSettings.baseUrlInvalid");
  expect(mockRemote.upsertCustomProvider).not.toHaveBeenCalled();
  // A model with no id, then two models sharing one id.
  await type("desktopSettings.baseUrl", "https://api.newco.example.com/v1");
  await act(async () => pressable("desktopSettings.addModel").props.onPress());
  await act(async () => button("desktopSettings.save").props.onPress());
  expect(rendered()).toContain("desktopSettings.modelIdRequired");
  await type("desktopSettings.modelId", "shared");
  await act(async () => pressable("desktopSettings.addModel").props.onPress());
  await type("desktopSettings.modelId", "shared");
  await act(async () => button("desktopSettings.save").props.onPress());
  expect(rendered()).toContain("desktopSettings.modelIdDuplicate");
  // Give the second model a distinct id but limits the desktop would refuse.
  await type("desktopSettings.modelId", "other");
  await type("desktopSettings.modelContextWindow", "0");
  await act(async () => button("desktopSettings.save").props.onPress());
  expect(rendered()).toContain("desktopSettings.modelLimitsInvalid");
  expect(mockRemote.upsertCustomProvider).not.toHaveBeenCalled();
});

test("the model editor writes every field and can drop a model again", async () => {
  await openProviders();
  await act(async () => link("Acme Gateway").props.onPress());
  // The stored provider's model starts collapsed; opening it is what makes the
  // fields reachable at all.
  expect(input("desktopSettings.modelName")).toBeUndefined();
  await act(async () => pressable("Acme Large").props.onPress());
  await type("desktopSettings.modelName", "Acme Large v2");
  await type("desktopSettings.modelContextWindow", "200000");
  await type("desktopSettings.modelMaxTokens", "8192");
  const capability = (label: string) =>
    tree.root.findAll(node => node.props.label === label && typeof node.props.onChange === "function")[0]!;
  await act(async () => capability("desktopSettings.modelSupportsImages").props.onChange(true));
  await act(async () => capability("desktopSettings.modelReasoning").props.onChange(true));
  await act(async () => { button("desktopSettings.save").props.onPress(); await Promise.resolve(); });
  expect(mockRemote.upsertCustomProvider).toHaveBeenCalledWith(expect.objectContaining({
    id: "acme",
    create: false,
    models: [expect.objectContaining({
      id: "acme-large",
      name: "Acme Large v2",
      contextWindow: 200000,
      maxTokens: 8192,
      supportsImages: true,
      reasoning: true,
    })],
  }));
});

test("adding a model then removing it leaves the provider as it was", async () => {
  await openProviders();
  await act(async () => link("Acme Gateway").props.onPress());
  await act(async () => pressable("desktopSettings.addModel").props.onPress());
  await type("desktopSettings.modelId", "scratch");
  expect(pressable("scratch")).toBeDefined();
  await type("desktopSettings.modelContextWindow", "8192");
  await type("desktopSettings.modelMaxTokens", "8192");
  await act(async () => button("desktopSettings.removeModel").props.onPress());
  expect(pressable("scratch")).toBeUndefined();
  // With the scratch model gone the stored one is all that is submitted, so an
  // abandoned draft cannot leak into the saved provider.
  await act(async () => { button("desktopSettings.save").props.onPress(); await Promise.resolve(); });
  expect(mockRemote.upsertCustomProvider).toHaveBeenCalledWith(expect.objectContaining({
    models: [expect.objectContaining({ id: "acme-large" })],
  }));
});

test("the API-type menu writes the chosen protocol and closes", async () => {
  await openProviders();
  await act(async () => button("desktopSettings.addProvider").props.onPress());
  await act(async () => pressable("desktopSettings.apiType").props.onPress());
  const menu = tree.root.findByType(ActionMenu);
  expect(menu.props.visible).toBe(true);
  const chosen = menu.props.actions[1].label as string;
  await act(async () => menu.props.actions[1].onPress());
  await act(async () => menu.props.onClose());
  expect(tree.root.findByType(ActionMenu).props.visible).toBe(false);
  // The row now shows the chosen protocol, so the menu wrote through to state.
  const row = tree.root.findAll(node =>
    node.props.accessibilityLabel === "desktopSettings.apiType"
    && node.props.accessibilityRole === "button")[0]!;
  expect(row.findAllByType(Text).map(node => node.props.children)).toContain(chosen);
});

test("deleting a provider is a two-step action that can be backed out of", async () => {
  await openProviders();
  await act(async () => link("Acme Gateway").props.onPress());
  await act(async () => button("desktopSettings.deleteProvider").props.onPress());
  expect(rendered()).toContain("desktopSettings.deleteProviderConfirm: Acme Gateway");
  await act(async () => button("chat.cancel").props.onPress());
  expect(mockRemote.deleteCustomProvider).not.toHaveBeenCalled();
  // Backing out arms nothing: the next tap only re-opens the confirmation.
  await act(async () => button("desktopSettings.deleteProvider").props.onPress());
  expect(rendered()).toContain("desktopSettings.deleteProviderConfirm: Acme Gateway");
  expect(mockRemote.deleteCustomProvider).not.toHaveBeenCalled();
  await act(async () => { button("desktopSettings.deleteProvider").props.onPress(); await Promise.resolve(); });
  expect(mockRemote.deleteCustomProvider).toHaveBeenCalledWith("acme");
});

test("a second delete cannot overlap the removal already in flight", async () => {
  await openProviders();
  await act(async () => link("Acme Gateway").props.onPress());
  const removal = deferred<ProvidersView>();
  mockRemote.deleteCustomProvider.mockReturnValueOnce(removal.promise);
  await act(async () => button("desktopSettings.deleteProvider").props.onPress());
  await act(async () => { button("desktopSettings.deleteProvider").props.onPress(); await Promise.resolve(); });
  expect(mockRemote.deleteCustomProvider).toHaveBeenCalledTimes(1);
  // A tap that still reaches the confirm action while the removal is unresolved
  // must not ask the desktop to delete the same provider twice.
  await act(async () => { button("desktopSettings.deleteProvider").props.onPress(); await Promise.resolve(); });
  expect(mockRemote.deleteCustomProvider).toHaveBeenCalledTimes(1);
  await act(async () => { removal.resolve(providers); await Promise.resolve(); });
});

test("a refused save or delete shows the desktop's own reason and stays on the page", async () => {
  await openProviders();
  await act(async () => link("Acme Gateway").props.onPress());
  mockRemote.upsertCustomProvider.mockRejectedValueOnce("catalog says no");
  await act(async () => { button("desktopSettings.save").props.onPress(); await Promise.resolve(); });
  expect(rendered()).toContain("catalog says no");
  // A refused save must not pop the page: its own fields are still on screen,
  // and the provider list (the level underneath) is not.
  expect(input("desktopSettings.providerId")).toBeDefined();
  expect(link("Acme Gateway")).toBeUndefined();
  mockRemote.deleteCustomProvider.mockRejectedValueOnce(new Error("provider still referenced"));
  await act(async () => button("desktopSettings.deleteProvider").props.onPress());
  await act(async () => { button("desktopSettings.deleteProvider").props.onPress(); await Promise.resolve(); });
  expect(rendered()).toContain("provider still referenced");
});

test("a second save cannot overlap the write already in flight", async () => {
  await openProviders();
  await act(async () => link("Acme Gateway").props.onPress());
  const write = deferred<ProvidersView>();
  mockRemote.upsertCustomProvider.mockReturnValueOnce(write.promise);
  await act(async () => { button("desktopSettings.save").props.onPress(); await Promise.resolve(); });
  expect(mockRemote.upsertCustomProvider).toHaveBeenCalledTimes(1);
  // The save button is disabled while busy; a tap that still reaches the
  // handler must not enqueue a second write of the same provider.
  await act(async () => button("desktopSettings.save").props.onPress());
  expect(mockRemote.upsertCustomProvider).toHaveBeenCalledTimes(1);
  await act(async () => { write.resolve(providers); await Promise.resolve(); });
});

test("an older desktop hides the provider pages and an offline one disables them", async () => {
  mockRemote.capabilities = new Set(["desktop_settings_v1", "skill_management_v1"]);
  await act(async () => tree.update(createElement(SettingsScreen, props)));
  expect(link("desktopSettings.providers").props.disabled).toBe(true);

  mockRemote.desktopOnline = false;
  mockRemote.capabilities = new Set(["desktop_settings_v1", "skill_management_v1", "provider_management_v1"]);
  await act(async () => tree.update(createElement(SettingsScreen, props)));
  expect(link("desktopSettings.providers").props.disabled).toBe(true);
  expect(mockRemote.listProviders).not.toHaveBeenCalled();
});

test("provider pages keep the settings back stack and the desktop scope banner", async () => {
  await openProviders();
  expect(rendered()).toContain("desktopSettings.boundDesktop: Work computer");
  await act(async () => link("Acme Gateway").props.onPress());
  // The editor is a deeper level: no scope banner, and back pops one level.
  expect(rendered()).not.toContain("desktopSettings.boundDesktop: Work computer");
  await act(async () => { back().props.onPress(); await Promise.resolve(); });
  expect(link("Acme Gateway")).toBeDefined();
  expect(props.onClose).not.toHaveBeenCalled();
});
