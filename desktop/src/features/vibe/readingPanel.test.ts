import { describe, expect, it } from "vitest";
import {
  cyclePreference,
  PANEL_AUTO_WIDTH,
  readPanelPreference,
  showReadingPanel,
  toggleHidden,
  togglePinned,
} from "./readingPanel";

/**
 * The reading layer's second column: a table of cases, because "is it there" is asked
 * from three places (the render, the two keys, the menu row) and they must not disagree.
 */
describe("showReadingPanel", () => {
  it("is off when hidden, whatever the window", () => {
    for (const width of [800, PANEL_AUTO_WIDTH, 2400])
      expect(showReadingPanel("hidden", width), String(width)).toBe(false);
  });

  it("is on when pinned, whatever the window", () => {
    // Pinning is a decision about this conversation: a narrow window must not undo it.
    for (const width of [800, PANEL_AUTO_WIDTH, 2400])
      expect(showReadingPanel("pinned", width), String(width)).toBe(true);
  });

  it("follows the window when automatic", () => {
    expect(showReadingPanel("auto", PANEL_AUTO_WIDTH - 1)).toBe(false);
    expect(showReadingPanel("auto", PANEL_AUTO_WIDTH)).toBe(true);
  });
});

describe("toggleHidden", () => {
  it("hides, and comes back to automatic rather than to pinned", () => {
    // "Bring it back the way it was" means `auto`: a key that sometimes returns the reader
    // to a *pinned* column they did not ask for is a key they stop pressing.
    expect(toggleHidden("auto")).toBe("hidden");
    expect(toggleHidden("pinned")).toBe("hidden");
    expect(toggleHidden("hidden")).toBe("auto");
  });
});

describe("togglePinned", () => {
  it("pins, and unpins back to automatic", () => {
    expect(togglePinned("auto")).toBe("pinned");
    expect(togglePinned("hidden")).toBe("pinned");
    expect(togglePinned("pinned")).toBe("auto");
  });
});

describe("cyclePreference", () => {
  it("walks the three states in both directions, wrapping", () => {
    expect(cyclePreference("hidden", 1)).toBe("auto");
    expect(cyclePreference("auto", 1)).toBe("pinned");
    expect(cyclePreference("pinned", 1)).toBe("hidden");
    expect(cyclePreference("hidden", -1)).toBe("pinned");
  });
});

describe("readPanelPreference", () => {
  it("reads a stored value and falls back to automatic", () => {
    expect(readPanelPreference({ "local:a": "pinned" }, "local:a")).toBe("pinned");
    // An unknown value (an older build, a hand-edited profile) does not break the column;
    // it simply behaves like the default.
    expect(readPanelPreference({ "local:a": "sideways" }, "local:a")).toBe("auto");
    expect(readPanelPreference({}, "local:a")).toBe("auto");
  });
});
