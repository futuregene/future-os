const mockGetItem = jest.fn();
const mockSetItem = jest.fn();
jest.mock("@react-native-async-storage/async-storage", () => ({
  getItem: (...args: unknown[]) => mockGetItem(...args),
  setItem: (...args: unknown[]) => mockSetItem(...args),
}));
jest.mock("expo-localization", () => ({ getLocales: () => [{ languageCode: "zh" }] }));

beforeEach(() => {
  mockGetItem.mockReset().mockResolvedValue(null);
  mockSetItem.mockReset().mockResolvedValue(undefined);
});

async function freshModules() {
  let preferences!: typeof import("../preferences");
  let i18n!: (typeof import("../index"))["default"];
  jest.isolateModules(() => {
    preferences = jest.requireActual("../preferences");
    i18n = jest.requireActual("../index").default;
  });
  await preferences.initializeLanguagePreference();
  return { preferences, i18n };
}

it.each([
  [null, "system", "zh"],
  ["system", "system", "zh"],
  ["zh", "zh", "zh"],
  ["en", "en", "en"],
  ["invalid", "system", "zh"],
])("restores %s on a Chinese system", async (stored, preference, language) => {
  mockGetItem.mockResolvedValue(stored);
  const state = await freshModules();
  expect(state.preferences.getLanguagePreference()).toBe(preference);
  expect(state.i18n.language).toBe(language);
  await state.preferences.initializeLanguagePreference();
  expect(mockGetItem).toHaveBeenCalledTimes(1);
});

it("launches with the system language when storage cannot be read", async () => {
  mockGetItem.mockRejectedValue(new Error("unavailable"));
  const { preferences, i18n } = await freshModules();
  expect(preferences.getLanguagePreference()).toBe("system");
  expect(i18n.language).toBe("zh");
});

it("persists the selection, restores it on restart, and can return to system", async () => {
  const { preferences, i18n } = await freshModules();
  const listener = jest.fn();
  const unsubscribe = preferences.subscribeLanguagePreference(listener);
  await preferences.setLanguagePreference("en");
  expect(mockSetItem).toHaveBeenLastCalledWith("future.mobile.language", "en");
  expect(i18n.language).toBe("en");
  expect(listener).toHaveBeenCalledTimes(1);
  mockGetItem.mockResolvedValue("en");
  const restarted = await freshModules();
  expect(restarted.i18n.language).toBe("en");
  unsubscribe();
  await preferences.setLanguagePreference("system");
  expect(mockSetItem).toHaveBeenLastCalledWith("future.mobile.language", "system");
  expect(i18n.language).toBe("zh");
  expect(listener).toHaveBeenCalledTimes(1);
});

it("keeps the previous language and preference when saving fails", async () => {
  const { preferences, i18n } = await freshModules();
  mockSetItem.mockRejectedValue(new Error("full"));
  await expect(preferences.setLanguagePreference("en")).rejects.toThrow("full");
  expect(preferences.getLanguagePreference()).toBe("system");
  expect(i18n.language).toBe("zh");
});
