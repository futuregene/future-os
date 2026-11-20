/**
 * Every settings level whose header names itself through `desktopSettings.<name>`.
 *
 * A value and not only a type, because a level whose title key is missing does
 * not fail loudly — it renders the key itself in the header, which is how
 * `appearance` first shipped. A test holds the two translations to this list.
 *
 * In a module of its own rather than beside the screen that renders them: the
 * search index needs this list, and importing the screen would drag React
 * Native and every page into a test that only reads data.
 */
export const TITLED_SETTINGS_PAGES = [
  "permission", "reply", "storage", "models", "fusion", "skills", "tasks", "language", "appearance", "providers", "followAccount",
] as const;

export type TitledSettingsPage = (typeof TITLED_SETTINGS_PAGES)[number];
