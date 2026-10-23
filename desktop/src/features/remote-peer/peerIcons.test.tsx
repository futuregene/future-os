// @vitest-environment jsdom
import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import en from "../../i18n/locales/en/remotePeer.json";
import zh from "../../i18n/locales/zh/remotePeer.json";
import { PeerIconGlyph } from "./PeerIconGlyph";
import { PEER_ICONS, peerBadgeText, peerIconId, peerIconTint } from "./peerIcons";

/**
 * The icon set a paired machine is drawn from.
 *
 * The ids live in `@future-os/peer-icons` and the phone reads the same list, so
 * what is left to prove here is the desktop's half: every id in that list
 * draws something, and the stored value never reaches the screen unfiltered.
 *
 * The set used to be emoji, drawn by whichever colour-emoji font the platform
 * ships — Apple's on a Mac, Noto's on Android, a monochrome fallback wherever
 * the stack found neither — so one machine looked like three. The mark is a
 * line icon now, which is what the "draws a shape, not text" case below pins
 * down: a glyph that went back to text would still render, and still pass a
 * "something appeared" check.
 */

(globalThis as Record<string, unknown>).IS_REACT_ACT_ENVIRONMENT = true;

/** Mount one glyph and return what it put in the DOM. */
function renderGlyph(id: string | null | undefined) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<PeerIconGlyph id={id} />));
  const drawn = {
    svg: container.querySelector("svg"),
    text: container.textContent ?? "",
  };
  act(() => root.unmount());
  container.remove();
  return drawn;
}

describe("peerIcons", () => {
  it("draws a glyph for every id in the shared set", () => {
    for (const icon of PEER_ICONS) {
      const drawn = renderGlyph(icon.id);
      // A missing entry in this app's table would render nothing at all.
      expect({ id: icon.id, hasSvg: drawn.svg !== null }).toEqual({ id: icon.id, hasSvg: true });
    }
  });

  /**
   * Every mark is painted, and painted in a colour of its own.
   *
   * The palette lives in the shared package so the two clients paint the same
   * machine the same hue; what this side can break is the wiring — a glyph drawn
   * without its tint, or a chip whose wash is not the glyph's own colour.
   */
  it("paints every glyph in its own colour", () => {
    const colors = new Set<string>();
    for (const icon of PEER_ICONS) {
      const tint = peerIconTint(icon.id);
      expect({ id: icon.id, color: /^#[0-9a-f]{6}$/u.test(tint.color) }).toEqual({ id: icon.id, color: true });
      expect(tint.soft.startsWith("rgba(")).toBe(true);
      colors.add(tint.color);
    }
    // Not one shared colour wearing twenty names: the point of the palette is
    // telling two marks apart.
    expect(colors.size).toBeGreaterThan(10);
  });

  /** An unknown label draws the default picture, so it takes its paint too. */
  it("gives an unknown id the default icon's tint", () => {
    expect(peerIconTint("not-an-icon")).toEqual(peerIconTint(PEER_ICONS[0]!.id));
    expect(peerIconTint(null)).toEqual(peerIconTint(PEER_ICONS[0]!.id));
  });

  /**
   * The mark is a drawing, not a character. This is the regression this file
   * exists for: an emoji here renders as *text*, in a font the other client
   * does not have, which is exactly the mismatch the shared set removed.
   */
  it("draws shapes, not characters", () => {
    for (const icon of PEER_ICONS) {
      expect({ id: icon.id, text: renderGlyph(icon.id).text }).toEqual({ id: icon.id, text: "" });
    }
  });

  it("resolves a stored id to itself", () => {
    expect(peerIconId("rocket")).toBe("rocket");
    expect(peerIconId("home")).toBe("home");
  });

  /**
   * This value comes from a label file another build may have written, and it
   * is drawn in place of a machine's name: an unknown id must draw the default
   * rather than store its way onto the screen, and must not leave the row blank.
   */
  it("falls back to the first icon for an unknown or empty id", () => {
    for (const value of [null, undefined, "", "not-an-icon", "<script>"]) {
      expect(peerIconId(value)).toBe(PEER_ICONS[0]!.id);
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
    for (const icon of PEER_ICONS)
      expect(icon.labelKey.length).toBeGreaterThan(0);
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
