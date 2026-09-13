import { createElement } from "react";
import { act, create, type ReactTestRenderer } from "react-test-renderer";
import { AppState, Text, View, type AppStateStatus } from "react-native";
import { useTranslation } from "react-i18next";
import { LanguageSettings } from "../LanguageSettings";
import { useSystemLanguage } from "../useSystemLanguage";
import i18n from "../index";
import { getLanguagePreference, setLanguagePreference } from "../preferences";

let mockLocales = [{ languageCode: "en" }];
const mockSetItem = jest.fn(async (_key: string, _value: string) => {});
jest.mock("@react-native-async-storage/async-storage", () => ({
  getItem: jest.fn(async () => null),
  setItem: (key: string, value: string) => mockSetItem(key, value),
}));
jest.mock("expo-localization", () => ({
  getLocales: () => mockLocales ?? [{ languageCode: "en" }],
  useLocales: () => mockLocales,
}));

let tree: ReactTestRenderer;
let listener: (state: AppStateStatus) => void;
const remove = jest.fn();
function Harness() {
  const ready = useSystemLanguage();
  const { t } = useTranslation();
  return ready ? createElement(View, null,
    createElement(Text, { testID: "translated-screen" }, t("sessions.settings")),
    createElement(LanguageSettings),
  ) : null;
}
const radios = () => tree.root.findAll(node => node.props.accessibilityRole === "radio" && typeof node.props.onPress === "function");
const radio = (label: string) => radios().find(node => node.props.accessibilityLabel === label)!;
const screenText = () => tree.root.findByProps({ testID: "translated-screen" }).props.children;
async function select(label: string) {
  await act(async () => { radio(label).props.onPress(); });
}
async function localeEvent(languageCode: string) {
  mockLocales = [{ languageCode }];
  await act(async () => { tree.update(createElement(Harness)); });
}

beforeEach(async () => {
  mockLocales = [{ languageCode: "en" }];
  mockSetItem.mockReset().mockResolvedValue(undefined);
  await setLanguagePreference("system");
  jest.clearAllMocks();
  jest.spyOn(AppState, "addEventListener").mockImplementation((_event, fn) => {
    listener = fn;
    return { remove };
  });
  await act(async () => { tree = create(createElement(Harness)); });
});
afterEach(() => {
  act(() => tree.unmount());
  jest.restoreAllMocks();
});

it("offers all three choices and changes the visible screen without a restart", async () => {
  expect(radio("Follow system").props.accessibilityState.checked).toBe(true);
  expect(screenText()).toBe("Settings");
  await select("中文");
  expect(screenText()).toBe("设置");
  expect(radio("中文").props.accessibilityState.checked).toBe(true);
  expect(mockSetItem).toHaveBeenLastCalledWith("future.mobile.language", "zh");
  await select("English");
  expect(screenText()).toBe("Settings");
  expect(getLanguagePreference()).toBe("en");
});

it("follows locale events in system mode and treats non-Chinese as English", async () => {
  await localeEvent("zh");
  expect(screenText()).toBe("设置");
  await localeEvent("ja");
  expect(screenText()).toBe("Settings");
});

it("reads fresh locales on resume even without a locale event", async () => {
  mockLocales = [{ languageCode: "zh" }];
  await act(async () => { listener("background"); });
  expect(screenText()).toBe("Settings");
  await act(async () => { listener("active"); });
  expect(screenText()).toBe("设置");
  const change = jest.spyOn(i18n, "changeLanguage");
  await act(async () => { listener("active"); });
  expect(change).not.toHaveBeenCalled();
});

it("keeps manual overrides during OS changes and resumes following on selection", async () => {
  await select("English");
  await localeEvent("zh");
  await act(async () => { listener("active"); });
  expect(screenText()).toBe("Settings");
  await select("Follow system");
  expect(screenText()).toBe("设置");
  await select("中文");
  await localeEvent("fr");
  await act(async () => { listener("active"); });
  expect(screenText()).toBe("设置");
});

it("reports save errors and retains the selected radio until a successful retry", async () => {
  mockSetItem.mockRejectedValueOnce(new Error("full"));
  await select("中文");
  expect(radio("Follow system").props.accessibilityState.checked).toBe(true);
  expect(tree.root.findAllByType(Text).some(node => node.props.children === "Could not save the language setting. Please try again.")).toBe(true);
  await select("中文");
  expect(screenText()).toBe("设置");
  expect(tree.root.findAllByType(Text).some(node => node.props.accessibilityRole === "alert")).toBe(false);
});

it("disables all choices while saving", async () => {
  let finish!: () => void;
  mockSetItem.mockReturnValueOnce(new Promise<void>(resolve => { finish = resolve; }));
  await select("中文");
  expect(radios().length).toBeGreaterThanOrEqual(3);
  expect(radios().every(node => node.props.disabled)).toBe(true);
  await act(async () => { finish(); });
  expect(screenText()).toBe("设置");
  expect(radio("中文").props.disabled).toBe(false);
});

it("removes its foreground listener on unmount", () => {
  act(() => tree.unmount());
  expect(remove).toHaveBeenCalledTimes(1);
});
