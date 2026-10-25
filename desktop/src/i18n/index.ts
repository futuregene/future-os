import i18n from "i18next";
import { initReactI18next } from "react-i18next";

/**
 * Locale bundles live in `./locales/<lang>/<namespace>.json`. They are loaded
 * eagerly via Vite's glob import so new namespace files are picked up
 * automatically without touching this file.
 */
const modules = import.meta.glob("./locales/*/*.json", { eager: true }) as Record<
  string,
  { default: Record<string, unknown> }
>;

const resources: Record<string, Record<string, Record<string, unknown>>> = {};
for (const [path, mod] of Object.entries(modules)) {
  // The glob pattern guarantees "./locales/<lang>/<namespace>.json", so the
  // match and both capture groups always exist (invariant, not a guess).
  const match = /\.\/locales\/([^/]+)\/([^/]+)\.json$/.exec(path)!;
  const lang = match[1]!;
  const namespace = match[2]!;
  (resources[lang] ??= {})[namespace] = mod.default;
}

export const SUPPORTED_LANGUAGES = ["zh", "en"] as const;
export type Language = (typeof SUPPORTED_LANGUAGES)[number];

/**
 * What the user picked in settings. "system" defers to the OS locale, which is
 * also the default until the user chooses either language explicitly.
 */
export const LANGUAGE_PREFERENCES = ["system", "zh", "en"] as const;
export type LanguagePreference = (typeof LANGUAGE_PREFERENCES)[number];

/** Labels that read the same in either UI language; the "system" one is translated by the caller. */
export const LANGUAGE_LABELS: Record<Language, string> = {
  zh: "中文",
  en: "English",
};

const STORAGE_KEY = "future.language";
export const DEFAULT_LANGUAGE: Language = "zh";
const DEFAULT_PREFERENCE: LanguagePreference = "system";

/**
 * The system default follows the OS: the webview mirrors the system locale, and
 * we only ship zh/en bundles, so a Chinese system picks "zh" and any other
 * language falls back to English. `navigator` is guarded for non-DOM contexts.
 */
function systemLanguage(): Language {
  try {
    return navigator.language.toLowerCase().startsWith("zh") ? "zh" : "en";
  }
  catch {
    return DEFAULT_LANGUAGE;
  }
}

function readStoredPreference(): LanguagePreference {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "system" || stored === "zh" || stored === "en")
      return stored;
  }
  catch {
    // localStorage may be unavailable; fall through to the system default.
  }
  return DEFAULT_PREFERENCE;
}

/** The bundle a preference resolves to right now. */
export function resolveLanguage(preference: LanguagePreference): Language {
  return preference === "system" ? systemLanguage() : preference;
}

const namespaces = Object.keys(resources[DEFAULT_LANGUAGE] ?? {});
let currentPreference = readStoredPreference();
const initialLanguage = resolveLanguage(currentPreference);

function syncDocumentLanguage(language: string): void {
  if (typeof document !== "undefined")
    document.documentElement.lang = language === "en" ? "en" : "zh-CN";
}

syncDocumentLanguage(initialLanguage);
i18n.on("languageChanged", syncDocumentLanguage);

void i18n.use(initReactI18next).init({
  resources,
  lng: initialLanguage,
  fallbackLng: "en",
  ns: namespaces.length > 0 ? namespaces : ["common"],
  defaultNS: "common",
  interpolation: { escapeValue: false },
  returnNull: false,
});

/** The language the UI is actually showing (a "system" preference already resolved). */
export function getLanguage(): Language {
  const current = i18n.language;
  return current === "en" ? "en" : "zh";
}

/** The preference the picker should show, which may be "system". */
export function getLanguagePreference(): LanguagePreference {
  return currentPreference;
}

export function setLanguage(preference: LanguagePreference): void {
  currentPreference = preference;
  try {
    localStorage.setItem(STORAGE_KEY, preference);
  }
  catch {
    // Persistence is best-effort; the change still applies for this session.
  }
  void i18n.changeLanguage(resolveLanguage(preference));
}

export default i18n;
