import { describe, expect, it, jest } from "@jest/globals";
import { iconGlyph, PEER_ICONS } from "../peerIcons";
import * as storage from "../storage";

/**
 * These two must agree: the desktop app stores the same icon ids, and a glyph
 * that exists on one platform but not the other would make one machine look
 * like two different ones depending on which device you are holding.
 */
const DESKTOP_ICON_IDS = ["desktop", "laptop", "server", "cloud", "home", "office", "rocket", "flask"];

describe("peerIcons", () => {
  it("uses the same ids as the desktop app", () => {
    expect(PEER_ICONS.map(icon => icon.id)).toEqual(DESKTOP_ICON_IDS);
  });

  it("resolves a stored id to its glyph", () => {
    expect(iconGlyph("rocket")).toBe("🚀");
    expect(iconGlyph("home")).toBe("🏠");
  });

  /**
   * An unknown or empty id must render the default: this value is drawn beside
   * a session title, so it can never be arbitrary stored text, and a blank row
   * would look like a broken list.
   */
  it("falls back to the default for an unknown or empty id", () => {
    for (const value of [null, undefined, "", "not-an-icon", "<script>"]) {
      expect(iconGlyph(value)).toBe(PEER_ICONS[0]!.glyph);
    }
  });

  it("has no duplicate ids and no empty labels", () => {
    const ids = PEER_ICONS.map(icon => icon.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const icon of PEER_ICONS) {
      expect(icon.glyph.length).toBeGreaterThan(0);
      expect(icon.labelKey.length).toBeGreaterThan(0);
    }
  });
});

describe("labelDesktop", () => {
  const setItem = jest.spyOn(storage, "labelDesktop");

  it("is the storage entry point a desktop's icon is written through", () => {
    // Guards against the label editor being wired to a function that does not
    // exist (a rename that leaves the icon behind, say) — the failure mode is
    // silent, because a missing write just leaves the old value in place.
    expect(typeof storage.labelDesktop).toBe("function");
    setItem.mockRestore();
  });
});
