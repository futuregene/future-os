// @vitest-environment jsdom
import { describe, expect, it } from "vitest";
import {
  canShowLeftPanel,
  canShowRightPanel,
  MIN_CENTER_PANEL_WIDTH,
  MIN_LEFT_PANEL_WIDTH,
  MIN_RIGHT_PANEL_WIDTH,
} from "./panelGeometry";

// The thresholds are derived from the floors, so a future floor change moves
// the behavior with it instead of leaving a stale magic number behind.
const LEFT_FITS = MIN_LEFT_PANEL_WIDTH + MIN_CENTER_PANEL_WIDTH;
const RIGHT_FITS = LEFT_FITS + MIN_RIGHT_PANEL_WIDTH;

describe("panel visibility thresholds", () => {
  it("keeps the rail while it does not squeeze the center below its floor", () => {
    expect(canShowLeftPanel(LEFT_FITS)).toBe(true);
    expect(canShowLeftPanel(LEFT_FITS - 1)).toBe(false);
    expect(canShowLeftPanel(1440)).toBe(true);
  });

  it("only opens the right panel when all three columns fit", () => {
    expect(canShowRightPanel(RIGHT_FITS)).toBe(true);
    expect(canShowRightPanel(RIGHT_FITS - 1)).toBe(false);
    // The rail still fits here, but the right panel must not steal the center's
    // floor to open.
    expect(canShowLeftPanel(RIGHT_FITS - 1)).toBe(true);
  });
});
