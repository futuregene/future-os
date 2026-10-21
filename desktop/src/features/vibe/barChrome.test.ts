import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { BAR_HEIGHT, barStart, reserveTrafficLights } from "./barChrome";

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

/**
 * The bar's height, checked against the app rather than against a literal.
 *
 * The report was "the text after the three dots looks misaligned", and the cause was that
 * the panel's bar was shorter than every other top bar: `h-9` centres its text at 18px,
 * `h-12` at 24px — the line the rail's header, the thread header, the context panel, the
 * new-conversation header and the traffic lights beside them all share. So the assertion
 * that matters is not "it is h-12" (that is a copy of the constant); it is "it is the same
 * height as the app's other headers", which stays true only if both move together. Like
 * `fullscreenCapability.test.ts`, this reads the source, because the invariant spans files.
 */
describe("the bar's height", () => {
  const repo = resolve(__dirname, "../../..");
  const heightOf = (path: string, marker: RegExp): string => {
    const source = readFileSync(resolve(repo, path), "utf8");
    const line = source.split("\n").find(text => marker.test(text));
    expect(line, `${path} should still have a top bar matching ${marker}`).toBeTruthy();
    const token = line!.match(/\bh-\d+\b/);
    expect(token, `${path}'s header should still declare a height`).toBeTruthy();
    return token![0];
  };

  it("is the height of the rail's header, the surface the lights already agree with", () => {
    expect(BAR_HEIGHT).toBe(heightOf("src/components/layout/ActivityRail.tsx", /relative flex h-\d+ shrink-0 select-none items-center/));
  });

  it("is the height of a conversation's header and the context panel's", () => {
    expect(BAR_HEIGHT).toBe(heightOf("src/features/agent/ThreadHeader.tsx", /flex h-\d+ shrink-0 select-none items-center border-b/));
    expect(BAR_HEIGHT).toBe(heightOf("src/components/layout/ContextPanel.tsx", /flex h-\d+ shrink-0 select-none items-center justify-between/));
  });
});
