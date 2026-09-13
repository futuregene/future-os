import type { Locale } from "expo-localization";
import { resources } from "../locales";

let mockLocales: Partial<Locale>[] = [];
jest.mock("expo-localization", () => ({ getLocales: () => mockLocales }));

it.each([
  ["Simplified Chinese", [{ languageCode: "zh", languageTag: "zh-Hans-CN" }], "zh"],
  ["Traditional Chinese", [{ languageCode: "zh", languageTag: "zh-Hant-TW" }], "zh"],
  ["Hong Kong Chinese", [{ languageCode: "zh", languageTag: "zh-Hant-HK" }], "zh"],
  ["English", [{ languageCode: "en", languageTag: "en-US" }], "en"],
  ["Japanese", [{ languageCode: "ja", languageTag: "ja-JP" }], "en"],
  ["French before Chinese", [{ languageCode: "fr" }, { languageCode: "zh" }], "en"],
  ["Chinese before English", [{ languageCode: "zh" }, { languageCode: "en" }], "zh"],
  ["missing locales", [], "en"],
  ["unknown language", [{ languageCode: null }], "en"],
] as const)("initializes synchronously for %s", (_label, locales, language) => {
  mockLocales = [...locales];
  jest.isolateModules(() => {
    const { default: i18n } = jest.requireActual<typeof import("../index")>("../index");
    expect(i18n.isInitialized).toBe(true);
    expect(i18n.language).toBe(language);
    expect(i18n.t("sessions.settings")).toBe(language === "zh" ? "设置" : "Settings");
  });
});

function translationKeys(value: object, prefix = ""): string[] {
  return Object.entries(value).flatMap(([key, text]) => {
    const path = prefix ? `${prefix}.${key}` : key;
    return typeof text === "string" ? [path] : translationKeys(text, path);
  }).sort();
}

it("has matching Chinese and English keys for every interface string", () => {
  expect(translationKeys(resources.zh.translation)).toEqual(translationKeys(resources.en.translation));
});
