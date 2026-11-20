import type { SettingsTab } from "./SettingsDialog";

/**
 * What a settings search can find, and where it lives.
 *
 * Hand-kept rather than derived from the pages: a page renders its rows from
 * props, and a row's label is an i18n key only at its call site, so there is
 * nothing to read back at search time without mounting every page. The list is
 * therefore the one place that knows the whole map — `settingsIndex.test.ts`
 * holds it to the locale files, and to the rule that a tab with no entry is a
 * page a reader cannot reach by name.
 *
 * Rows that come from a machine's own data (a provider's name, a model's label)
 * are deliberately absent: they are not settings, and their text arrives at
 * runtime rather than from the locale files.
 */
export interface SettingsEntry {
  /** Key in the `settings` namespace. */
  labelKey: string;
  tab: SettingsTab;
}

export const SETTINGS_ENTRIES: readonly SettingsEntry[] = [
  // 应用
  { labelKey: "language.title", tab: "appearance" },
  { labelKey: "appearance.title", tab: "appearance" },
  { labelKey: "theme.title", tab: "appearance" },
  { labelKey: "fontSize.title", tab: "appearance" },
  { labelKey: "replyDetail.title", tab: "appearance" },
  // This machine's own notification, not something a reply does on every
  // client: the bell plays here and a paired phone never sees the row.
  { labelKey: "bellOnComplete.title", tab: "appearance" },
  { labelKey: "dialog.tabs.shortcuts", tab: "shortcuts" },
  // 会话
  { labelKey: "approvalTier.title", tab: "permission" },
  { labelKey: "choiceCards.title", tab: "reply" },
  { labelKey: "autoTitleFirstTurn.title", tab: "reply" },
  { labelKey: "archive.cleanupEnabled", tab: "storage" },
  { labelKey: "archive.autoEnabled", tab: "storage" },
  { labelKey: "archive.idleDays", tab: "storage" },
  { labelKey: "reset.organizeDatabase", tab: "storage" },
  // 模型与技能
  { labelKey: "providers.builtinTitle", tab: "providers" },
  { labelKey: "providers.customTitle", tab: "providers" },
  { labelKey: "providers.addCustom", tab: "providers" },
  { labelKey: "dialog.tabs.models", tab: "models" },
  { labelKey: "autoUpgradeSkills.title", tab: "skills" },
  { labelKey: "skillRecommend.title", tab: "skills" },
  // 账号
  { labelKey: "account.futureGene", tab: "account" },
  { labelKey: "account.logout", tab: "account" },
  { labelKey: "update.title", tab: "update" },
  { labelKey: "dialog.tabs.about", tab: "about" },
  // 调试
  { labelKey: "dialog.tabs.reset", tab: "reset" },
  { labelKey: "fusion.enabled", tab: "fusion" },
  { labelKey: "dialog.tabs.environment", tab: "environment" },
];

/** How many matches the nav column shows at once: more than a reader scans. */
export const SETTINGS_SEARCH_LIMIT = 8;

/** The page's own name, as the nav and the page header both print it. */
export function settingsTabTitleKey(tab: SettingsTab): string {
  return `dialog.tabs.${tab}`;
}

/**
 * Entries whose row label or page name contains the query, in index order.
 *
 * A match on the page name keeps every row of that page, because a reader who
 * types 「存储」 is asking for the page, not for the row that happens to share
 * its name.
 */
export function searchSettings(
  translate: (key: string) => string,
  query: string,
  entries: readonly SettingsEntry[] = SETTINGS_ENTRIES,
): SettingsEntry[] {
  const needle = query.trim().toLowerCase();
  if (!needle)
    return [];
  return entries.filter((entry) => {
    const text = `${translate(entry.labelKey)} ${translate(settingsTabTitleKey(entry.tab))}`.toLowerCase();
    return text.includes(needle);
  }).slice(0, SETTINGS_SEARCH_LIMIT);
}
