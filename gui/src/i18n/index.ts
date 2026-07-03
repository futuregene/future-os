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
  const match = path.match(/\.\/locales\/([^/]+)\/([^/]+)\.json$/);
  if (!match)
    continue;
  const lang = match[1];
  const namespace = match[2];
  if (!lang || !namespace)
    continue;
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

function readStoredLanguage(): Language {
  try {
    const stored = localStorage.getItem(STORAGE_KEY);
    if (stored === "zh" || stored === "en")
      return stored;
  }
  catch {
    // localStorage may be unavailable; fall through to the default.
  }
  return DEFAULT_LANGUAGE;
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
