/** @jest-environment node */

import { describe, expect, it } from "@jest/globals";
import { PEER_GLYPHS } from "../peerGlyphs";
import { PEER_ICONS, peerIconId } from "../peerIcons";
import { resources } from "../../i18n/locales";

/**
 * The icon set a paired machine is drawn from.
 *
 * The ids live in `@future-os/peer-icons` and the desktop reads the same list,
 * so what is left to prove here is the phone's half of the drawing: every id in
 * that list has a glyph on this platform.
 *
 * The set used to be emoji, and an emoji is drawn by whichever colour-emoji
 * font the device ships — Noto's picture on Android, Apple's on iOS, a
 * monochrome text glyph wherever the stack found neither — so one machine
 * looked like three different machines. A line icon drawn from the same path
 * data as the desktop's ends that; a glyph that went back to text would still
 * render, which is why the mapping is checked id by id rather than by
 * "something appeared".
 */

/** The `desktops.icons` namespace of one bundle, as the picker reads it. */
function iconLabels(locale: "en" | "zh"): Record<string, unknown> {
  const bundle: Record<string, unknown> = resources[locale].translation;
  const desktops = bundle.desktops as { icons?: Record<string, unknown> } | undefined;
  return desktops?.icons ?? {};
}

describe("peerIcons", () => {
  it("has a glyph of its own for every id in the shared set", () => {
    for (const icon of PEER_ICONS)
      expect({ id: icon.id, drawn: Boolean(PEER_GLYPHS[icon.id]) }).toEqual({ id: icon.id, drawn: true });
  });

  it("resolves a stored id to itself", () => {
    expect(peerIconId("rocket")).toBe("rocket");
    expect(peerIconId("home")).toBe("home");
  });

  /**
   * A label file from another build may hold an id this one does not know, and
   * this value is drawn in place of a machine's name: it must never reach the
   * screen as itself, and must not leave the row blank either.
   */
  it("falls back to the default for an unknown or empty id", () => {
    for (const value of [null, undefined, "", "not-an-icon", "<script>"])
      expect(peerIconId(value)).toBe(PEER_ICONS[0]!.id);
  });

  it("has no duplicate ids and no empty labels", () => {
    const ids = PEER_ICONS.map(icon => icon.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const icon of PEER_ICONS)
      expect(icon.labelKey.length).toBeGreaterThan(0);
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
