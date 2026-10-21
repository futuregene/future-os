import type { PeerIcon } from "./peerIcons";
import type { RemotePeer } from "./remotePeerClient";
import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it } from "vitest";
import en from "../../i18n/locales/en/remotePeer.json";
import zh from "../../i18n/locales/zh/remotePeer.json";
import { iconGlyph, PEER_ICONS, peerBadgeText } from "./peerIcons";

/**
 * The phone keeps the same ids: a user can label the same machine from either
 * device, and a glyph that exists on one platform but not the other would make
 * one machine look like two different ones depending on which device you are
 * holding. Reading the other platform's source (rather than a copy of the list
 * here) is what makes drift fail the build instead of the user.
 */
function mobileIcons(): Pick<PeerIcon, "id" | "glyph">[] {
  const file = path.resolve(import.meta.dirname, "../../../../mobile/src/remote/peerIcons.ts");
  const found: Pick<PeerIcon, "id" | "glyph">[] = [];
  for (const match of readFileSync(file, "utf8").matchAll(
    /\{\s*id:\s*"([^"]+)",\s*glyph:\s*"([^"]+)",/gu,
  ))
    found.push({ id: match[1]!, glyph: match[2]! });
  expect(found.length).toBeGreaterThan(0);
  return found;
}

describe("peerIcons", () => {
  it("uses the same ids and glyphs as the phone", () => {
    expect(PEER_ICONS.map(icon => ({ id: icon.id, glyph: icon.glyph }))).toEqual(mobileIcons());
  });

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

  /**
   * Every glyph has to be one emoji that both this app and the phone can draw,
   * and the picker is where a bad one stays invisible until a phone shows a
   * tofu box: the old `server` glyph was `U+1F5A7` (three networked computers),
   * which is not an emoji at all, and `U+1F5A5`/`U+2601` without `U+FE0F`
   * selected their monochrome text glyph. So: exactly one code point (no
   * skin-tone or ZWJ sequences, which older fonts split or drop), plus `U+FE0F`
   * unless Unicode gives that code point an emoji presentation by default.
   */
  it("uses glyphs every platform renders as emoji", () => {
    // Default-emoji-presentation code points, from `emoji-test.txt`: their
    // fully-qualified sequence has no `U+FE0F`.
    const DEFAULT_EMOJI = new Set([
      0x1F4BB /* 💻 */,
      0x1F4F1 /* 📱 */,
      0x1F3E0 /* 🏠 */,
      0x1F3E2 /* 🏢 */,
      0x2615 /* ☕ */,
      0x1F3EB /* 🏫 */,
      0x1F680 /* 🚀 */,
      0x1F9EA /* 🧪 */,
      0x1F916 /* 🤖 */,
      0x1F4CA /* 📊 */,
      0x1F4BE /* 💾 */,
      0x1F4FA /* 📺 */,
      0x1F3A8 /* 🎨 */,
      0x1F9E0 /* 🧠 */,
      0x1F52C /* 🔬 */,
      0x1F512 /* 🔒 */,
      0x26A1 /* ⚡ */,
      0x2B50 /* ⭐ */,
      0x1F3AC /* 🎬 */,
      0x1F3AE /* 🎮 */,
      0x1F310 /* 🌐 */,
      0x1FA9F /* 🪟 */,
      0x1F34E /* 🍎 */,
      0x1F427 /* 🐧 */,
    ]);
    for (const icon of PEER_ICONS) {
      const points = [...icon.glyph].map(char => char.codePointAt(0)!);
      const base = points.filter(point => point !== 0xFE0F);
      expect({ id: icon.id, codePoints: base.length }).toEqual({ id: icon.id, codePoints: 1 });
      if (!DEFAULT_EMOJI.has(base[0]!)) {
        expect({ id: icon.id, hasVariationSelector: points.includes(0xFE0F) })
          .toEqual({ id: icon.id, hasVariationSelector: true });
      }
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

  /** A picker option with no label is an unlabelled button for screen readers. */
  it("has an English and Chinese label for every icon", () => {
    for (const icon of PEER_ICONS) {
      const key = icon.labelKey as keyof typeof en.icons;
      for (const [locale, bundle] of [["en", en], ["zh", zh]] as const) {
        expect({ locale, label: bundle.icons[key] })
          .toEqual({ locale, label: expect.any(String) });
      }
    }
  });
});

/** Compile-time proof the picker's props match what the view passes. */
export function _typecheck(peer: RemotePeer): string {
  return peerBadgeText({ icon: peer.icon, name: peer.name, desktopId: peer.desktopId }, "fallback");
}
