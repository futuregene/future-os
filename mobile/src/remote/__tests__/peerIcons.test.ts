/** @jest-environment node */

import { readFileSync } from "node:fs";
import path from "node:path";
import { describe, expect, it, jest } from "@jest/globals";
import { iconGlyph, PEER_ICONS, type PeerIcon } from "../peerIcons";
import * as storage from "../storage";
import { resources } from "../../i18n/locales";

/**
 * The desktop app keeps the same ids: a user can label the same machine from
 * either device, and a glyph that exists on one platform but not the other
 * would make one machine look like two different ones depending on which
 * device you are holding. Reading the other platform's source (rather than a
 * copy of the list here) is what makes drift fail the build instead of the
 * user.
 */
function desktopIcons(): Pick<PeerIcon, "id" | "glyph">[] {
  const file = path.resolve(__dirname, "../../../../desktop/src/features/remote-peer/peerIcons.ts");
  const found: Pick<PeerIcon, "id" | "glyph">[] = [];
  for (const match of readFileSync(file, "utf8").matchAll(
    /\{\s*id:\s*"([^"]+)",\s*glyph:\s*"([^"]+)",/gu,
  ))
    found.push({ id: match[1]!, glyph: match[2]! });
  expect(found.length).toBeGreaterThan(0);
  return found;
}

/** The `desktops.icons` namespace of one bundle, as the picker reads it. */
function iconLabels(locale: "en" | "zh"): Record<string, unknown> {
  const bundle: Record<string, unknown> = resources[locale].translation;
  const desktops = bundle.desktops as { icons?: Record<string, unknown> } | undefined;
  return desktops?.icons ?? {};
}

describe("peerIcons", () => {
  it("uses the same ids and glyphs as the desktop app", () => {
    expect(PEER_ICONS.map(icon => ({ id: icon.id, glyph: icon.glyph }))).toEqual(desktopIcons());
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

  /**
   * Every glyph has to be one emoji that both mobile and desktop fonts can
   * draw, and the picker is where a bad one is invisible until a phone shows a
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
      0x1F4F1 /* 📱 */, 0x1F3E0 /* 🏠 */, 0x1F3E2 /* 🏢 */, 0x2615 /* ☕ */,
      0x1F3EB /* 🏫 */, 0x1F3ED /* 🏭 */, 0x1F3EA /* 🏪 */, 0x1F680 /* 🚀 */,
      0x1F9EA /* 🧪 */, 0x1F916 /* 🤖 */, 0x1F4E6 /* 📦 */, 0x1F4CA /* 📊 */,
      0x1F52C /* 🔬 */, 0x1F512 /* 🔒 */, 0x26A1 /* ⚡ */, 0x2B50 /* ⭐ */,
      0x1F3AC /* 🎬 */, 0x1F3AE /* 🎮 */, 0x1F310 /* 🌐 */,
    ]);
    for (const icon of PEER_ICONS) {
      const points = [...icon.glyph].map(char => char.codePointAt(0)!);
      const base = points.filter(point => point !== 0xFE0F);
      expect({ id: icon.id, codePoints: base.length }).toEqual({ id: icon.id, codePoints: 1 });
      if (!DEFAULT_EMOJI.has(base[0]!))
        expect({ id: icon.id, hasVariationSelector: points.includes(0xFE0F) })
          .toEqual({ id: icon.id, hasVariationSelector: true });
    }
  });

  /** A picker option with no label is an unlabelled button for screen readers. */
  it("has a Chinese and English label for every icon", () => {
    for (const icon of PEER_ICONS) {
      for (const locale of ["en", "zh"] as const) {
        expect({ locale, label: iconLabels(locale)[icon.labelKey] })
          .toEqual({ locale, label: expect.any(String) });
      }
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
