/**
 * Walking the panel's menu: which row is selected, and what `↑`/`↓` do to it.
 *
 * Pure and separate from the component for two reasons: `VibeMenu.tsx` then
 * exports only a component (which is what fast refresh wants), and the walk is the
 * part a remote control depends on — it wraps, it skips a row that cannot run, and
 * it never lands nowhere — so it is worth pinning down without a DOM.
 */

/** One row of the panel's menu. */
export interface VibeMenuRow {
  id: string;
  label: string;
  /** A choice row's current value, shown at the right (`自动`, `2 列`, `本机`). */
  value?: string;
  disabled?: boolean;
  onRun: () => void;
  /**
   * A choice row's `←`/`→`. `delta` is -1 or +1; the caller wraps or clamps.
   *
   * Present only on rows that hold a value: an action row leaves the horizontal
   * keys alone, and the menu shows `← →` only where it does something.
   */
  onAdjust?: (delta: -1 | 1) => void;
  /** Extra line under the label: what the row does, for the rows worth explaining. */
  description?: string;
  /** Draws a separator above this row. */
  group?: boolean;
}

/** The first row a reader can actually run, from `from` onwards (wrapping). */
export function firstEnabled(rows: readonly VibeMenuRow[], from: number): number {
  if (rows.length === 0)
    return 0;
  for (let offset = 0; offset < rows.length; offset++) {
    const candidate = (from + offset) % rows.length;
    if (!rows[candidate]!.disabled)
      return candidate;
  }
  return from;
}

/**
 * The next row in `delta`'s direction, **wrapping**, skipping disabled ones.
 *
 * Wrapping here is the opposite of the grid's rule, and deliberately: a menu is a
 * list of a few rows where "down at the bottom" has an obvious next, while the grid
 * is a place where teleporting would lose the reader. When every row is disabled the
 * selection stays put.
 */
export function nextEnabled(
  rows: readonly VibeMenuRow[],
  current: number,
  delta: number,
): number {
  if (rows.length === 0)
    return 0;
  const start = Math.min(Math.max(current, 0), rows.length - 1);
  for (let offset = 1; offset <= rows.length; offset++) {
    const candidate = ((start + delta * offset) % rows.length + rows.length) % rows.length;
    if (!rows[candidate]!.disabled)
      return candidate;
  }
  return start;
}
