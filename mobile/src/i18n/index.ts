import { createInstance } from "i18next";
import { initReactI18next } from "react-i18next";
import { getLocales } from "expo-localization";
import { resources } from "./locales";

function systemLanguage(): "zh" | "en" {
  // Only the primary system language matters, not a secondary preference.
  return getLocales()[0]?.languageCode === "zh" ? "zh" : "en";
}

const i18n = createInstance();

void i18n.use(initReactI18next).init({
  compatibilityJSON: "v4",
  initAsync: false,
  fallbackLng: "en",
  lng: systemLanguage(),
  resources,
  interpolation: {
    escapeValue: false,
  },
});

export type LanguagePreference = "system" | "zh" | "en";

export function syncLanguage(preference: LanguagePreference): void {
  const language = preference === "system" ? systemLanguage() : preference;
  if (i18n.language !== language) void i18n.changeLanguage(language);
}

export default i18n;
