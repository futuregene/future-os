// @vitest-environment node
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { searchSettings, SETTINGS_ENTRIES, SETTINGS_SEARCH_LIMIT, settingsTabTitleKey } from "./settingsIndex";

const LOCALES: Record<string, Record<string, unknown>> = {
  en: JSON.parse(readFileSync(new URL("../../i18n/locales/en/settings.json", import.meta.url), "utf-8")),
  zh: JSON.parse(readFileSync(new URL("../../i18n/locales/zh/settings.json", import.meta.url), "utf-8")),
};

function lookup(locale: Record<string, unknown>, key: string): unknown {
  return key.split(".").reduce<unknown>((node, part) => (node && typeof node === "object" ? (node as Record<string, unknown>)[part] : undefined), locale);
}

/** The pages the dialog can show, in nav order. */
const TABS = [
  "appearance",
  "shortcuts",
  "permission",
  "reply",
  "storage",
  "providers",
  "models",
  "skills",
  "account",
  "update",
  "about",
  "reset",
  "fusion",
  "environment",
];

describe("settingsIndex", () => {
  it("names only keys both locales have", () => {
    const missing: string[] = [];
    for (const entry of SETTINGS_ENTRIES) {
      for (const key of [entry.labelKey, settingsTabTitleKey(entry.tab)]) {
        for (const [lang, locale] of Object.entries(LOCALES)) {
          if (typeof lookup(locale, key) !== "string")
            missing.push(`${lang}:${key}`);
        }
      }
    }
    // A row label the locale files do not have renders as the key itself, which
    // is what a search would then be matching against.
    expect(missing).toEqual([]);
  });

  it("gives every page at least one entry", () => {
    const covered = new Set(SETTINGS_ENTRIES.map(entry => entry.tab));
    expect([...covered].sort()).toEqual([...TABS].sort());
  });

  it("matches a page name and keeps that page's rows", () => {
    const translate = (key: string) => String(lookup(LOCALES.zh!, key));
    const hits = searchSettings(translate, "存储");

    expect(hits.length).toBeGreaterThan(1);
    expect(new Set(hits.map(hit => hit.tab))).toEqual(new Set(["storage"]));
  });

  it("matches a row in either language, ignoring case and surrounding space", () => {
    const zh = searchSettings((key: string) => String(lookup(LOCALES.zh!, key)), "  提示音 ");
    expect(zh.map(hit => hit.labelKey)).toEqual(["bellOnComplete.title"]);

    const en = searchSettings((key: string) => String(lookup(LOCALES.en!, key)), "ARCHIVE");
    expect(en.every(hit => hit.tab === "storage")).toBe(true);
  });

  it("returns nothing for an empty or unmatched query", () => {
    const translate = (key: string) => String(lookup(LOCALES.zh!, key));
    expect(searchSettings(translate, "   ")).toEqual([]);
    expect(searchSettings(translate, "没有这一项")).toEqual([]);
  });

  it("caps the list at what the rail can show", () => {
    const translate = (key: string) => String(lookup(LOCALES.zh!, key));
    expect(searchSettings(translate, "会").length).toBeLessThanOrEqual(SETTINGS_SEARCH_LIMIT);
  });
});
