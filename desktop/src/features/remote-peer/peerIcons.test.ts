import type { RemotePeer } from "./remotePeerClient";
import { describe, expect, it } from "vitest";
import { iconGlyph, PEER_ICONS, peerBadgeText } from "./peerIcons";

describe("peerIcons", () => {
  it("resolves a stored id to its glyph", () => {
    expect(iconGlyph("rocket")).toBe("🚀");
    expect(iconGlyph("home")).toBe("🏠");
  });

  /**
   * The rail prints this next to a title. An unknown id (a label from a newer
   * build, or a hand-edited file) must not put arbitrary text on screen, and it
   * must not leave the row blank either.
   */
  it("falls back to the first icon for an unknown or empty id", () => {
    for (const value of [null, undefined, "", "not-an-icon", "<script>"]) {
      expect(iconGlyph(value)).toBe(PEER_ICONS[0]!.glyph);
    }
  });

  it("uses the user's name when they set one", () => {
    expect(peerBadgeText({ icon: "home", name: "Studio iMac", desktopId: "desktop_abcdef" }, "x"))
      .toBe("Studio iMac");
  });

  /**
   * Without a name the row shows an identity, and the stable part of a desktop
   * id is its tail: two machines on one account share the `desktop_` prefix, so
   * showing the head would make them indistinguishable.
   */
  it("falls back to the tail of the desktop id, not its prefix", () => {
    const short = peerBadgeText({ icon: null, name: null, desktopId: "desktop_ab12cd34ef" }, "fallback");
    expect(short).toBe("ab12cd");
    expect(short).not.toContain("desktop");
  });

  it("uses the supplied fallback when the id has nothing to show", () => {
    expect(peerBadgeText({ icon: null, name: null, desktopId: "desktop_" }, "已配对桌面"))
      .toBe("已配对桌面");
    expect(peerBadgeText({ icon: null, name: null, desktopId: "" }, "已配对桌面"))
      .toBe("已配对桌面");
  });

  it("keeps the icon set free of duplicates and empty labels", () => {
    const ids = PEER_ICONS.map(icon => icon.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const icon of PEER_ICONS) {
      expect(icon.glyph.length).toBeGreaterThan(0);
      expect(icon.labelKey.length).toBeGreaterThan(0);
    }
  });
});

/** Compile-time proof the picker's props match what the view passes. */
export function _typecheck(peer: RemotePeer): string {
  return peerBadgeText({ icon: peer.icon, name: peer.name, desktopId: peer.desktopId }, "fallback");
}
