/**
 * The panel's own view preferences: the order the user left, and an explicit
 * column count.
 *
 * `localStorage`, like the theme, the font size and the panel widths, and for the
 * same reasons: these are per machine (a layout tuned for a 16-inch laptop is
 * wrong on a 4K monitor), they are read on the render path rather than on a data
 * path, and none of them belongs to the account. `VIBE.md` §12 records the plan
 * for moving them into `app.db`; until then this is the only file that touches the
 * keys, so the move stays a change to `read()` / `write()` here.
 *
 * Stored input is untrusted — an older build, a hand-edited profile, a partial
 * write — so every field is validated on read and a bad value is dropped rather
 * than guessed at.
 */

import type { VibeDeviceFilter } from "./vibeCells";

export interface VibePreferences {
  /** The panel order as the user left it, oldest first. See `order.ts`. */
  order: string[];
  /**
   * Each conversation's answer about the reading layer's second column, by panel key.
   *
   * Per conversation and not per machine: "I am reading this one's files" is a fact about
   * one conversation, and remembering it globally would make every other cell pay for it.
   */
  panel: Record<string, string>;
  /**
   * Which machines the panel shows, as last chosen.
   *
   * A view preference like the columns: per machine, read on the render path, and
   * worth remembering — a reader who pinned their attention to one host should not
   * have to say so again on the next launch.
   */
  device: VibeDeviceFilter;
  /**
   * An explicit column count, or `null` for the automatic layout.
   *
   * `null` is the useful default: the automatic rule follows the window, so a
   * reader who never touched the keys gets the right grid on every screen.
   */
  columns: number | null;
}

const ORDER_KEY = "future.vibe.order";
const PREFS_KEY = "future.vibe.prefs";

const EMPTY: VibePreferences = { columns: null, device: { kind: "all" }, order: [], panel: {} };

function readOrder(): string[] {
  try {
    const raw = localStorage.getItem(ORDER_KEY);
    if (!raw)
      return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed))
      return [];
    // Only strings that look like panel keys: a stored entry from a future
    // version with a different shape is dropped, never reinterpreted.
    return parsed.filter((entry): entry is string => typeof entry === "string" && entry.includes(":"));
  }
  catch {
    return [];
  }
}

function readDevice(): VibeDeviceFilter {
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (!raw)
      return { kind: "all" };
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null)
      return { kind: "all" };
    const device = (parsed as { device?: unknown }).device;
    if (typeof device !== "object" || device === null)
      return { kind: "all" };
    const { kind, desktopId } = device as { kind?: unknown; desktopId?: unknown };
    if (kind === "all")
      return { kind: "all" };
    // A machine id is only meaningful with `device`; anything else is dropped rather
    // than guessed at, and "all" is the answer that shows something either way.
    if (kind === "device" && typeof desktopId === "string" && desktopId)
      return { desktopId, kind: "device" };
    return { kind: "all" };
  }
  catch {
    return { kind: "all" };
  }
}

function readColumns(): number | null {
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (!raw)
      return null;
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null)
      return null;
    const value = (parsed as { columns?: unknown }).columns;
    return typeof value === "number" && Number.isInteger(value) && value > 0 ? value : null;
  }
  catch {
    return null;
  }
}

/** The stored preferences, for a caller that is not a component. */
function readPanelMap(): Record<string, string> {
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (!raw)
      return {};
    const parsed: unknown = JSON.parse(raw);
    if (typeof parsed !== "object" || parsed === null)
      return {};
    const panel = (parsed as { panel?: unknown }).panel;
    if (typeof panel !== "object" || panel === null)
      return {};
    // Values are validated by the reader (`readingPanel.readPanelPreference`), so a value
    // this build does not know falls back to `auto` instead of being dropped here.
    return { ...panel as Record<string, string> };
  }
  catch {
    return {};
  }
}

export function readVibePreferences(): VibePreferences {
  return { columns: readColumns(), device: readDevice(), order: readOrder(), panel: readPanelMap() };
}

/** Remember one conversation's second-column choice. */
export function writeVibePanel(key: string, preference: string): void {
  writePrefs(current => ({ ...current, panel: { ...current.panel, [key]: preference } }));
}

export function writeVibeOrder(order: readonly string[]): void {
  try {
    localStorage.setItem(ORDER_KEY, JSON.stringify(order));
  }
  catch {
    // Best effort: the session keeps working from state, and the next reorder
    // that does persist wins. Nothing here is data the user cannot retype by
    // dragging a cell.
  }
}

export function writeVibeColumns(columns: number | null): void {
  writePrefs(current => ({ ...current, columns }));
}

/** Remember which machines the panel shows. */
export function writeVibeDevice(device: VibeDeviceFilter): void {
  writePrefs(current => ({ ...current, device }));
}

/**
 * Merge one field into the preferences document.
 *
 * Read-modify-write rather than a whole-object write, because the panel writes two
 * different fields from two different keys and a write that carried only its own
 * field would erase the other one on the next launch.
 */
function writePrefs(update: (current: {
  columns: number | null;
  device: VibeDeviceFilter;
  panel: Record<string, string>;
}) => { columns: number | null; device: VibeDeviceFilter; panel: Record<string, string> }): void {
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : {};
    const current = typeof parsed === "object" && parsed !== null ? parsed as Record<string, unknown> : {};
    const next = update({
      columns: typeof current.columns === "number" ? current.columns : null,
      device: readDevice(),
      panel: readPanelMap(),
    });
    localStorage.setItem(PREFS_KEY, JSON.stringify(next));
  }
  catch {
    // Best effort: the session keeps working from state.
  }
}

/** Test seam: forget everything this module stored. */
export function clearVibePreferences(): void {
  try {
    localStorage.removeItem(ORDER_KEY);
    localStorage.removeItem(PREFS_KEY);
  }
  catch {
    // Ignore.
  }
}

export { EMPTY as EMPTY_VIBE_PREFERENCES };
