/**
 * The panel's grid geometry: how many columns and rows a given number of cells
 * gets in a given window.
 *
 * Pure on purpose — no DOM, no React — because the layout is the one part of the
 * panel that is easy to get subtly wrong and cheap to pin down with a table of
 * cases (see `layout.test.ts`, which asserts the documented table directly).
 *
 * Two rules decide it:
 *
 * - **`ceil(sqrt(n))` columns, capped at four.** That single rule reproduces the
 *   whole design table — 1 → 1×1, 2 → 2×1, 3–4 → 2×2, 5–6 → 3×2, 7–9 → 3×3,
 *   10–12 → 4×3 — without a lookup table to keep in sync with the cap.
 * - **A narrow window gives up a column rather than squeezing cells**, but only
 *   when giving it up actually buys height: that is what turns two cells into 1×2
 *   on a narrow window (two 400px slivers are unreadable; a stack is not) while
 *   leaving five cells at 3×2 (stacking five would give 180px strips).
 *
 * The floors below are **what a cell likes, not a hard constraint**. No
 * arrangement of five cells in a 1280px window can give every cell 520px, and
 * refusing to draw the grid would be worse than drawing small cells — so the
 * automatic shape wins, and the floors only decide when stacking is the lesser
 * evil.
 */

/**
 * The size a cell likes, in CSS pixels — used to decide *when stacking beats
 * squeezing*, never to refuse a grid (see the module comment).
 */
export const MIN_CELL_WIDTH = 520;
export const MIN_CELL_HEIGHT = 420;

/** More than this and the panel is not an overview any more. */
export const MAX_CELLS = 12;

/** The widest grid; four columns is where a 4K window stops getting more useful. */
export const MAX_COLUMNS = 4;

export interface GridLayout {
  columns: number;
  rows: number;
}

/**
 * The layout for `count` cells in a `width` × `height` area.
 *
 * `columns` overrides the automatic choice (the panel's `mod+=` / `mod+-`), and
 * is clamped to something that can actually be drawn: at least one column, never
 * more columns than cells.
 */
export function gridOf(
  count: number,
  width: number,
  height: number,
  columns?: number | null,
): GridLayout {
  const cells = Math.max(1, Math.floor(count));
  if (columns && columns >= 1)
    return { columns: Math.min(columns, cells), rows: Math.ceil(cells / Math.min(columns, cells)) };

  let chosen = Math.min(MAX_COLUMNS, Math.max(1, Math.ceil(Math.sqrt(cells))));
  // Give up a column only while it actually buys width and the new rows still fit:
  // the usual case is two cells in a narrow window, where 2×1 is a pair of
  // unreadable slivers and 1×2 is a readable stack.
  while (chosen > 1) {
    if (width / chosen >= MIN_CELL_WIDTH)
      break;
    const shorter = chosen - 1;
    const shorterRows = Math.ceil(cells / shorter);
    if (height / shorterRows < MIN_CELL_HEIGHT)
      break;
    chosen = shorter;
  }
  return { columns: chosen, rows: Math.ceil(cells / chosen) };
}

/**
 * Where a cell sits in the grid: `row * columns + column`.
 *
 * A cell's index is its place in the order, so the grid is filled left to right,
 * top to bottom, and a short last row stays short — cell 2 is always the second
 * place, never stretched across the row.
 */
export function cellPosition(index: number, layout: GridLayout): { column: number; row: number } {
  return {
    column: index % layout.columns,
    row: Math.floor(index / layout.columns),
  };
}

/**
 * Where a child sits, as CSS grid lines — **both axes, and that is the point**.
 *
 * A child given only `gridColumn` still has an automatic row, so CSS places it with the
 * auto-placement cursor rather than in the square this function names: the add tile, drawn
 * before the cells, lands in the first row and pushes every cell after it into the next
 * row, past the template's last row into an *implicit* one. The explicit
 * `repeat(rows, minmax(0, 1fr))` then has nothing to give height to (`0px 0px`), the
 * implicit row takes the space, and the panel draws cells two pixels tall while one cell
 * sits in a row nobody asked for. Measured on a real render: three cells read as
 * `rows: 346px 346px 70px` with cell 3 in the 70px implicit row.
 *
 * So placement is stated, never inferred: every child answers with a definite square, and
 * the grid's template is the only thing deciding size.
 */
export function gridAreaOf(
  index: number,
  layout: GridLayout,
): { gridColumn: number; gridRow: number } {
  const { column, row } = cellPosition(index, layout);
  return { gridColumn: column + 1, gridRow: row + 1 };
}

/**
 * The cell a direction key moves to, or the same index when there is nowhere to
 * go.
 *
 * **No wrapping and no skipping**: running off an edge stays put, because the
 * grid is a place and a key that teleports from the right edge to the next row's
 * left one makes "where am I" unanswerable. A row that is short at its end simply
 * has nothing to the right of its last cell.
 */
export function moveInGrid(
  index: number,
  direction: "up" | "down" | "left" | "right",
  count: number,
  layout: GridLayout,
): number {
  if (count <= 0)
    return 0;
  const { column, row } = cellPosition(index, layout);
  switch (direction) {
    case "left":
      return column > 0 ? index - 1 : index;
    case "right":
      return column + 1 < layout.columns && index + 1 < count ? index + 1 : index;
    case "up":
      return row > 0 ? index - layout.columns : index;
    case "down": {
      const next = index + layout.columns;
      return next < count ? next : index;
    }
  }
}
