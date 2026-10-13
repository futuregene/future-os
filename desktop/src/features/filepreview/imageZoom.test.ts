import { describe, expect, it } from "vitest";
import {
  clampScale,
  clampTranslation,
  fitScale,
  MAX_SCALE,
  MIN_SCALE,
  resetZoom,
  zoomAtAnchor,
} from "./imageZoom";

describe("imageZoom", () => {
  describe("fitScale", () => {
    it("returns 1 when image fits without enlarging", () => {
      expect(fitScale({ width: 1000, height: 800 }, { width: 500, height: 400 })).toBe(1);
    });

    it("scales down wide images to fit width", () => {
      expect(fitScale({ width: 100, height: 100 }, { width: 200, height: 50 })).toBe(0.5);
    });

    it("scales down tall images to fit height", () => {
      expect(fitScale({ width: 100, height: 100 }, { width: 50, height: 200 })).toBe(0.5);
    });

    it("returns 1 for invalid image sizes", () => {
      expect(fitScale({ width: 100, height: 100 }, { width: 0, height: 0 })).toBe(1);
      expect(fitScale({ width: 100, height: 100 }, { width: -1, height: -1 })).toBe(1);
    });
  });

  describe("clampScale", () => {
    it("keeps values inside bounds", () => {
      expect(clampScale(2)).toBe(2);
      expect(clampScale(1)).toBe(1);
      expect(clampScale(6)).toBe(6);
    });

    it("clamps below MIN_SCALE and above MAX_SCALE", () => {
      expect(clampScale(0.5)).toBe(MIN_SCALE);
      expect(clampScale(10)).toBe(MAX_SCALE);
    });

    it("falls back to MIN_SCALE for non-finite values", () => {
      expect(clampScale(Number.NaN)).toBe(MIN_SCALE);
      expect(clampScale(Number.POSITIVE_INFINITY)).toBe(MAX_SCALE);
    });
  });

  describe("clampTranslation", () => {
    it("locks translation to 0 when image fits", () => {
      expect(clampTranslation(50, 1, 1000, 500)).toBe(0);
      expect(clampTranslation(-50, 1, 1000, 500)).toBe(0);
    });

    it("allows panning by at most the overhang", () => {
      // image scaled to 1200 px in 1000 px container => 100 px overhang each side
      expect(clampTranslation(200, 1, 1000, 1200)).toBe(100);
      expect(clampTranslation(-200, 1, 1000, 1200)).toBe(-100);
    });

    it("returns 0 for non-finite values", () => {
      expect(clampTranslation(Number.NaN, 2, 100, 200)).toBe(0);
    });
  });

  describe("zoomAtAnchor", () => {
    it("zooms in while keeping the anchor point fixed", () => {
      const container = { width: 1000, height: 800 };
      const image = { width: 2000, height: 1600 };
      const state = { scale: 1, x: 0, y: 0 };
      const anchor = { x: 500, y: 400 }; // container center => no translation change
      const next = zoomAtAnchor(container, image, state, anchor, 2);
      expect(next.scale).toBe(2);
      expect(next.x).toBeCloseTo(0, 5);
      expect(next.y).toBeCloseTo(0, 5);
    });

    it("clamps new scale to [MIN, MAX]", () => {
      const container = { width: 100, height: 100 };
      const image = { width: 50, height: 50 };
      const state = { scale: 1, x: 0, y: 0 };
      expect(zoomAtAnchor(container, image, state, { x: 50, y: 50 }, 0.5).scale).toBe(MIN_SCALE);
      expect(zoomAtAnchor(container, image, state, { x: 50, y: 50 }, 100).scale).toBe(MAX_SCALE);
    });

    it("does not allow panning beyond image bounds after zoom", () => {
      const container = { width: 100, height: 100 };
      const image = { width: 200, height: 200 };
      const state = { scale: 1, x: 0, y: 0 };
      const next = zoomAtAnchor(container, image, state, { x: 10, y: 10 }, 3);
      expect(next.scale).toBe(3);
      // With image at 600 px in a 100 px container, overhang is 250 px.
      expect(next.x).toBeGreaterThanOrEqual(-250);
      expect(next.x).toBeLessThanOrEqual(250);
      expect(next.y).toBeGreaterThanOrEqual(-250);
      expect(next.y).toBeLessThanOrEqual(250);
    });
  });

  describe("resetZoom", () => {
    it("returns the fitted state", () => {
      expect(resetZoom()).toEqual({ scale: MIN_SCALE, x: 0, y: 0 });
    });
  });
});
