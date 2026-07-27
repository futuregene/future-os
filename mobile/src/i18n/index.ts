import { createInstance } from "i18next";
import { initReactI18next } from "react-i18next";
import { getLocales } from "expo-localization";
import { resources } from "./locales";

const language = getLocales()[0]?.languageCode === "zh" ? "zh" : "en";

const i18n = createInstance();

void i18n.use(initReactI18next).init({
  compatibilityJSON: "v4",
  initAsync: false,
  fallbackLng: "en",
  lng: language,
  resources,
  interpolation: {
    escapeValue: false,
  },
});

export default i18n;
