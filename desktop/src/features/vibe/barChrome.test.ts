import { describe, expect, it } from "vitest";
import { barStart, reserveTrafficLights } from "./barChrome";

/**
 * Where the panel's bar may put its text.
 *
 * The bug this pins: the panel is `fixed inset-0`, so it is the only surface whose content
 * starts at x=0 — and macOS draws the traffic lights *over* the page there
 * (`trafficLightPosition: { x: 14, y: 26 }`, so the buttons occupy roughly x 14–72). The
 * bar's 12px inset put "Vibe" and the pinned count underneath them.
 */
describe("reserveTrafficLights", () => {
  it("reserves on macOS outside fullscreen", () => {
    expect(reserveTrafficLights(true, false)).toBe(true);
  });

  it("does not reserve in fullscreen, where the system hides the lights", () => {
    expect(reserveTrafficLights(true, true)).toBe(false);
  });

  it("does not reserve on other platforms", () => {
    // Windows and Linux draw their own decorations, or none; the page's top-left is free.
    expect(reserveTrafficLights(false, false)).toBe(false);
    expect(reserveTrafficLights(false, true)).toBe(false);
  });
});

describe("barStart", () => {
  it("clears the lights when they are there", () => {
    // 80px, the rail's own reserve: past the lights' right edge (~72px) with the bar's
    // usual gap left over. Not a magic number picked to look right — it is the number the
    // rest of the app already reserves.
    expect(barStart(true)).toBe("pl-20");
  });

  it("uses the ordinary inset when they are not", () => {
    expect(barStart(false)).toBe("pl-3");
  });
});
