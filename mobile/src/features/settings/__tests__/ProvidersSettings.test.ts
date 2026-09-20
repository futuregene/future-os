import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { Text, TextInput } from "react-native";
import { Button } from "../../../components/Button";
import { SettingsLink } from "../SettingsPrimitives";
import { SettingsScreen } from "../SettingsScreen";
import type { ProvidersView } from "../../../remote/types";

let providers: ProvidersView;

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
