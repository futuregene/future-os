// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import { cellPosition, gridOf, MAX_CELLS, moveInGrid } from "./layout";

/** A typical 16-inch laptop window, in CSS pixels. */
const WIDE = { height: 900, width: 1512 };
/** The same window narrowed to half its width, where the floors start to bite. */
const NARROW = { height: 900, width: 800 };

function shape(count: number, area = WIDE, columns?: number) {
  const layout = gridOf(count, area.width, area.height, columns);
  return `${layout.columns}x${layout.rows}`;
}

describe("gridOf", () => {
  it("matches the documented table", () => {
    // The design's table, asserted directly: a change to the rule that does not
    // reproduce these is a change to the product, not a refactor.
    expect(shape(1)).toBe("1x1");
    expect(shape(2)).toBe("2x1");
    expect(shape(3)).toBe("2x2");
    expect(shape(4)).toBe("2x2");
    expect(shape(5)).toBe("3x2");
    expect(shape(6)).toBe("3x2");
    expect(shape(7)).toBe("3x3");
    expect(shape(8)).toBe("3x3");
    expect(shape(9)).toBe("3x3");
    expect(shape(10)).toBe("4x3");
    expect(shape(11)).toBe("4x3");
    expect(shape(12)).toBe("4x3");
  });

  it("never exceeds the cell cap's column count", () => {
    expect(gridOf(MAX_CELLS, WIDE.width, WIDE.height).columns).toBeLessThanOrEqual(4);
  });

  it("stacks two cells on a narrow window instead of halving them", () => {
    // 2×1 at 800px is two 400px slivers; 1×2 is a readable stack.
    expect(shape(2, NARROW)).toBe("1x2");
    expect(shape(2)).toBe("2x1");
  });

  it("keeps the columns when giving one up would not buy a cell its height", () => {
    // Five cells in a narrow window: dropping to one column would stack five
    // 180px strips, so it keeps the narrower-but-fits arrangement.
    expect(shape(5, NARROW)).toBe("3x2");
  });

  it("respects an explicit column count, clamped to what can be drawn", () => {
    expect(shape(6, WIDE, 1)).toBe("1x6");
    expect(shape(6, WIDE, 2)).toBe("2x3");
    // More columns than cells is one column per cell, not empty columns.
    expect(shape(3, WIDE, 9)).toBe("3x1");
    expect(shape(3, WIDE, 0)).toBe("2x2");
  });

  it("treats a nonsense count as one cell", () => {
    expect(shape(0)).toBe("1x1");
    expect(shape(-3)).toBe("1x1");
  });
});

describe("cellPosition", () => {
  it("fills left to right, top to bottom", () => {
    const layout = { columns: 3, rows: 2 };
    expect(cellPosition(0, layout)).toEqual({ column: 0, row: 0 });
    expect(cellPosition(2, layout)).toEqual({ column: 2, row: 0 });
    expect(cellPosition(3, layout)).toEqual({ column: 0, row: 1 });
    expect(cellPosition(4, layout)).toEqual({ column: 1, row: 1 });
  });
});

describe("moveInGrid", () => {
  const threeByTwo = { columns: 3, rows: 2 };

  it("moves one place in the direction asked", () => {
    expect(moveInGrid(4, "left", 6, threeByTwo)).toBe(3);
    expect(moveInGrid(4, "right", 6, threeByTwo)).toBe(5);
    expect(moveInGrid(4, "up", 6, threeByTwo)).toBe(1);
    expect(moveInGrid(1, "down", 6, threeByTwo)).toBe(4);
  });

  it("does not wrap at an edge", () => {
    // Right from the last column stays; right from the row's last cell stays even
    // when a later row continues (the grid is a place, not a ring).
    expect(moveInGrid(2, "right", 6, threeByTwo)).toBe(2);
    expect(moveInGrid(5, "right", 6, threeByTwo)).toBe(5);
    expect(moveInGrid(0, "left", 6, threeByTwo)).toBe(0);
    expect(moveInGrid(0, "up", 6, threeByTwo)).toBe(0);
  });

  it("has nothing to the right of a short last row", () => {
    // Five cells in three columns: index 4 is the second column of row two, and
    // there is no sixth cell to move onto.
    expect(moveInGrid(4, "right", 5, threeByTwo)).toBe(4);
  });

  it("has nothing below the last row", () => {
    expect(moveInGrid(3, "down", 6, threeByTwo)).toBe(3);
  });

  it("survives an empty grid", () => {
    expect(moveInGrid(0, "down", 0, threeByTwo)).toBe(0);
  });
});
