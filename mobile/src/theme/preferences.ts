import AsyncStorage from "@react-native-async-storage/async-storage";
import type { FontSizePreference } from "./tokens";

/** Follow the phone's own light/dark setting, or pin one scheme. */
export type ThemeMode = "system" | "light" | "dark";

/** This phone's appearance. Every field is local to it; no desktop is written. */
export type Appearance = {
  themeMode: ThemeMode;
  fontSize: FontSizePreference;
  /**
   * Keep a settled reply to the answer it ends on: its reasoning, tool calls and
   * notes fold into one "show process" row above the answer.
   *
   * A reading preference of *this phone*, not of the machine it is reading: the
   * same choice applies to every paired desktop's conversation, because it is
   * this client that decides how a transcript is drawn — the desktop sends the
   * same ordered slices either way.
   */
  finalAnswerOnly: boolean;
};

const STORAGE_KEY = "future.mobile.appearance";

/** Light at the size the app was designed at, which is also what a reader who
 * has never opened the setting should get — and the whole reply, work included. */
export const DEFAULT_APPEARANCE: Appearance = { themeMode: "system", fontSize: "medium", finalAnswerOnly: false };

let appearance = DEFAULT_APPEARANCE;
let initialization: Promise<void> | undefined;
const listeners = new Set<() => void>();

export const getAppearance = (): Appearance => appearance;

export function subscribeAppearance(listener: () => void): () => void {
  listeners.add(listener);
  return () => { listeners.delete(listener); };
}

function apply(next: Appearance): void {
  appearance = next;
  listeners.forEach(listener => listener());
}

/** Read one stored record, field by field: an unreadable theme must not also
 * cost the reader their text size (or the other way round). */
export function parseAppearance(value: string | null): Appearance {
  let stored: unknown;
  try {
    stored = value === null ? undefined : JSON.parse(value);
  } catch {
    // Storage held something that is not our JSON; treat it as absent.
    stored = undefined;
  }
  const record = typeof stored === "object" && stored !== null ? stored as Record<string, unknown> : {};
  const themeMode = record.themeMode;
  const fontSize = record.fontSize;
  // Anything but a stored `true` is off: a record written before this field
  // existed must read as the default rather than as undefined.
  const finalAnswerOnly = record.finalAnswerOnly === true;
  return {
    themeMode: themeMode === "light" || themeMode === "dark" || themeMode === "system" ? themeMode : DEFAULT_APPEARANCE.themeMode,
    fontSize: fontSize === "small" || fontSize === "large" || fontSize === "xlarge" || fontSize === "medium"
      ? fontSize
      : DEFAULT_APPEARANCE.fontSize,
    finalAnswerOnly,
  };
}

/** Restore before showing screens so a saved override doesn't flash the default. */
export function initializeAppearance(): Promise<void> {
  initialization ??= AsyncStorage.getItem(STORAGE_KEY).then(value => {
    apply(parseAppearance(value));
  }).catch(() => {
    // Unavailable storage must not prevent launch; use the defaults for this run.
    apply(DEFAULT_APPEARANCE);
  });
  return initialization;
}

export async function setAppearance(patch: Partial<Appearance>): Promise<void> {
  await initializeAppearance();
  const next = { ...appearance, ...patch };
  // Commit only after persistence succeeds, keeping the selection honest on failure.
  await AsyncStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  apply(next);
}
