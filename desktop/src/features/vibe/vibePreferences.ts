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

export interface VibePreferences {
  /** The panel order as the user left it, oldest first. See `order.ts`. */
  order: string[];
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

const EMPTY: VibePreferences = { columns: null, order: [] };

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
export function readVibePreferences(): VibePreferences {
  return { columns: readColumns(), order: readOrder() };
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
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify({ columns }));
  }
  catch {
    // As above.
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
