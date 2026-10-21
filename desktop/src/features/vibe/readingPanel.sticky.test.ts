import { describe, expect, it } from "vitest";
import { nextStickyDefault, readPanelPreference } from "./readingPanel";

/**
 * The sticky half of the second column's state.
 *
 * Reported from use: close the column in one cell, move to the next cell, and it was open
 * again. The fix is that the *last* `hidden`/`auto` choice is the fallback for a
 * conversation with no entry of its own — and that a pin does not travel (a column nobody
 * asked for is worse than one extra press).
 */
describe("readPanelPreference", () => {
  it("uses the conversation's own entry when there is one", () => {
    expect(readPanelPreference({ "local:a": "hidden" }, "local:a", "auto")).toBe("hidden");
    expect(readPanelPreference({ "local:a": "auto" }, "local:a", "hidden")).toBe("auto");
  });

  it("falls back to the panel's habit for a conversation with no entry", () => {
    // This is the whole fix: `hidden` carries over, so the next cell does not reopen a
    // column the reader just closed.
    expect(readPanelPreference({}, "local:b", "hidden")).toBe("hidden");
  });

  it("defaults to automatic when nothing has been decided", () => {
    expect(readPanelPreference({}, "local:b")).toBe("auto");
  });

  it("treats an unknown stored value as no entry", () => {
    expect(readPanelPreference({ "local:a": "sideways" }, "local:a", "hidden")).toBe("hidden");
  });
});

describe("nextStickyDefault", () => {
  it("records hidden and auto", () => {
    expect(nextStickyDefault("auto", "hidden")).toBe("hidden");
    expect(nextStickyDefault("hidden", "auto")).toBe("auto");
  });

  it("leaves the habit alone when a conversation is pinned", () => {
    // Pinning says nothing about the other conversations.
    expect(nextStickyDefault("hidden", "pinned")).toBe("hidden");
    expect(nextStickyDefault("auto", "pinned")).toBe("auto");
  });
});
