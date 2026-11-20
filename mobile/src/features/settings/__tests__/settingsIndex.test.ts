import { resources } from "../../../i18n/locales";
import { TITLED_SETTINGS_PAGES } from "../settingsPages";
import {
  DEVICE_SETTINGS_ENTRIES,
  PHONE_SETTINGS_ENTRIES,
  SETTINGS_SEARCH_LIMIT,
  searchMobileSettings,
  settingsPageTitleKey,
} from "../settingsIndex";

/**
 * The index as data: the search itself is nothing without it.
 *
 * Two rules are pinned here. Every key it sends the reader to must exist in
 * both languages — a key that does not resolve makes a row whose label is the
 * key itself, which is exactly how `appearance` first shipped. And every titled
 * page needs an entry, or it is a page no one can reach by name.
 */

/** The dotted path of every string in a language block. */
function keys(language: "en" | "zh"): Set<string> {
  const out = new Set<string>();
  const walk = (node: Record<string, unknown>, prefix = "") => {
    for (const [key, value] of Object.entries(node)) {
      const path = prefix ? `${prefix}.${key}` : key;
      if (value && typeof value === "object")
        walk(value as Record<string, unknown>, path);
      else
        out.add(path);
    }
  };
  walk(resources[language].translation as unknown as Record<string, unknown>);
  return out;
}

const translate = (key: string) => key;

describe("the mobile settings index", () => {
  it("sends the reader only to keys that exist in both languages", () => {
    for (const language of ["en", "zh"] as const) {
      const available = keys(language);
      const missing = [...PHONE_SETTINGS_ENTRIES, ...DEVICE_SETTINGS_ENTRIES]
        .flatMap(entry => [entry.labelKey, settingsPageTitleKey(entry.page)])
        .filter(key => !available.has(key));
      // Listed rather than asserted one by one: the failure has to name the key.
      expect(missing).toEqual([]);
    }
  });

  it("has an entry for every titled page, so no page is reachable only by scrolling", () => {
    const pages = new Set([...PHONE_SETTINGS_ENTRIES, ...DEVICE_SETTINGS_ENTRIES].map(entry => entry.page));
    expect(TITLED_SETTINGS_PAGES.filter(page => !pages.has(page))).toEqual([]);
  });

  it("keeps each entry on its own level", () => {
    expect(PHONE_SETTINGS_ENTRIES.every(entry => entry.scope === "phone")).toBe(true);
    expect(DEVICE_SETTINGS_ENTRIES.every(entry => entry.scope === "device")).toBe(true);
  });
});

describe("searching the mobile settings index", () => {
  it("returns nothing for an empty query rather than the whole index", () => {
    expect(searchMobileSettings(translate, "", DEVICE_SETTINGS_ENTRIES)).toEqual([]);
    expect(searchMobileSettings(translate, "   ", DEVICE_SETTINGS_ENTRIES)).toEqual([]);
  });

  it("answers a page name with the page, not with each of its rows", () => {
    expect(searchMobileSettings(translate, "desktopSettings.storage", DEVICE_SETTINGS_ENTRIES))
      .toEqual([{ page: "storage", labelKey: "desktopSettings.storage", pageMatch: true }]);
  });

  it("answers a row name with that row, naming the page it lives on", () => {
    const matches = searchMobileSettings(translate, "desktopSettings.archiveRunLogCleanup", DEVICE_SETTINGS_ENTRIES);
    expect(matches).toEqual([{ page: "storage", labelKey: "desktopSettings.archiveRunLogCleanup", pageMatch: false }]);
  });

  it("matches every row whose name contains the text, in index order", () => {
    const matches = searchMobileSettings(translate, "desktopSettings.autoArchive", DEVICE_SETTINGS_ENTRIES);
    expect(matches.map(match => match.labelKey)).toEqual([
      "desktopSettings.autoArchive",
      "desktopSettings.autoArchiveIdleDays",
    ]);
    expect(matches.every(match => match.page === "storage")).toBe(true);
  });

  it("ignores case and surrounding spaces", () => {
    expect(searchMobileSettings(translate, "  APPEARANCE.THEME ", PHONE_SETTINGS_ENTRIES))
      .toEqual([{ page: "appearance", labelKey: "appearance.theme", pageMatch: false }]);
  });

  it("searches one level only: a phone query does not reach a paired desktop", () => {
    expect(searchMobileSettings(translate, "approvalTier.title", PHONE_SETTINGS_ENTRIES)).toEqual([]);
    expect(searchMobileSettings(translate, "appearance.theme", DEVICE_SETTINGS_ENTRIES)).toEqual([]);
  });

  it("stops at the limit rather than filling the screen", () => {
    const matches = searchMobileSettings(translate, "desktopSettings.", DEVICE_SETTINGS_ENTRIES);
    expect(matches).toHaveLength(SETTINGS_SEARCH_LIMIT);
  });
});
