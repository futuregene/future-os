import type { TitledSettingsPage } from "./settingsPages";

/**
 * What a settings search can find on a phone, and the page it opens.
 *
 * Hand-kept for the same reason the desktop's `settingsIndex.ts` is: a page
 * renders its rows from props and a row's label exists as an i18n key only at
 * its call site, so there is nothing to read back at search time without
 * mounting every page. `settingsIndex.test.ts` holds this list to the locale
 * files and to the rule that no page is reachable only by scrolling.
 *
 * Two scopes, because the phone keeps two stacks: its own settings, and one
 * paired desktop's. A search on the phone's own screen must not offer a row of
 * a machine it happens to be near — the same rule the two homes already keep.
 *
 * Rows that come from a machine's data (a provider's name, a model's label) are
 * deliberately absent: they are not settings, and their text arrives at runtime
 * rather than from the locale table.
 */
export type SettingsSearchScope = "phone" | "device";

export interface MobileSettingsEntry {
  scope: SettingsSearchScope;
  page: TitledSettingsPage;
  /** Key, in the default namespace, of the row inside that page. */
  labelKey: string;
}

/** This phone's own settings: everything `renderPhoneHome` opens. */
export const PHONE_SETTINGS_ENTRIES: readonly MobileSettingsEntry[] = [
  { scope: "phone", page: "appearance", labelKey: "appearance.theme" },
  { scope: "phone", page: "appearance", labelKey: "appearance.fontSize" },
  { scope: "phone", page: "appearance", labelKey: "appearance.reply" },
  { scope: "phone", page: "language", labelKey: "desktopSettings.language" },
  { scope: "phone", page: "followAccount", labelKey: "desktopSettings.followAccount" },
];

/** One paired desktop's settings: everything `renderDeviceHome` opens. */
export const DEVICE_SETTINGS_ENTRIES: readonly MobileSettingsEntry[] = [
  { scope: "device", page: "permission", labelKey: "approvalTier.title" },
  { scope: "device", page: "reply", labelKey: "desktopSettings.choiceCards" },
  { scope: "device", page: "reply", labelKey: "desktopSettings.autoTitleFirstTurn" },
  { scope: "device", page: "storage", labelKey: "desktopSettings.archiveRunLogCleanup" },
  { scope: "device", page: "storage", labelKey: "desktopSettings.autoArchive" },
  { scope: "device", page: "storage", labelKey: "desktopSettings.autoArchiveIdleDays" },
  // The pages below are reached as a whole: their rows are a machine's own
  // data (models, skills, providers, tasks), so the page name is the entry.
  { scope: "device", page: "models", labelKey: "desktopSettings.models" },
  { scope: "device", page: "fusion", labelKey: "desktopSettings.fusion" },
  { scope: "device", page: "providers", labelKey: "desktopSettings.providers" },
  { scope: "device", page: "skills", labelKey: "desktopSettings.autoUpgradeSkills" },
  { scope: "device", page: "skills", labelKey: "desktopSettings.skillRecommend" },
  { scope: "device", page: "tasks", labelKey: "desktopSettings.tasks" },
];

/** How many matches the list shows at once: more than fits above a keyboard. */
export const SETTINGS_SEARCH_LIMIT = 6;

/** The page's own name, as its header and its home row both print it. */
export function settingsPageTitleKey(page: TitledSettingsPage): string {
  return `desktopSettings.${page}`;
}

export interface MobileSettingsMatch {
  page: TitledSettingsPage;
  /** The row's key, or the page's own title key when the page name matched. */
  labelKey: string;
  /** True when the query named the page rather than one of its rows. */
  pageMatch: boolean;
}

/**
 * Matches for a query, in the order the home lists the pages.
 *
 * A query that names a page yields the page itself rather than each of its
 * rows, because a reader who types 「存储」 is asking for that page. Rows are
 * listed only for pages whose own name did not match — otherwise one page
 * would fill the whole list.
 */
export function searchMobileSettings(
  translate: (key: string) => string,
  query: string,
  entries: readonly MobileSettingsEntry[],
): MobileSettingsMatch[] {
  const needle = query.trim().toLowerCase();
  if (!needle)
    return [];
  const matches: MobileSettingsMatch[] = [];
  for (const entry of entries) {
    if (matches.length === SETTINGS_SEARCH_LIMIT)
      break;
    if (matches.some(match => match.page === entry.page && match.pageMatch))
      continue;
    const titleKey = settingsPageTitleKey(entry.page);
    if (translate(titleKey).toLowerCase().includes(needle)) {
      matches.push({ page: entry.page, labelKey: titleKey, pageMatch: true });
      continue;
    }
    // A page's placeholder entry carries its title as its label; with the title
    // ruled out above, there is nothing left for it to match.
    if (entry.labelKey === titleKey || !translate(entry.labelKey).toLowerCase().includes(needle))
      continue;
    matches.push({ page: entry.page, labelKey: entry.labelKey, pageMatch: false });
  }
  return matches;
}
