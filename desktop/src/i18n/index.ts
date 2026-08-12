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

export const LANGUAGE_LABELS: Record<Language, string> = {
  zh: "中文",
  en: "English",
};

const STORAGE_KEY = "future.language";
export const DEFAULT_LANGUAGE: Language = "zh";

/**
 * First-run default follows the OS: the webview mirrors the system locale, and
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

function readStoredLanguage(): Language {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "zh" || stored === "en")
      return stored;
  }
  catch {
    // localStorage may be unavailable; fall through to the system language.
  }
  return systemLanguage();
}

const namespaces = Object.keys(resources[DEFAULT_LANGUAGE] ?? {});

void i18n.use(initReactI18next).init({
  resources,
  lng: readStoredLanguage(),
  fallbackLng: "en",
  ns: namespaces.length > 0 ? namespaces : ["common"],
  defaultNS: "common",
  interpolation: { escapeValue: false },
  returnNull: false,
});

export function getLanguage(): Language {
  const current = i18n.language;
  return current === "en" ? "en" : "zh";
}

export function setLanguage(language: Language): void {
  try {
    localStorage.setItem(STORAGE_KEY, language);
  }
  catch {
    // Persistence is best-effort; the change still applies for this session.
  }
  void i18n.changeLanguage(language);
}

export default i18n;
