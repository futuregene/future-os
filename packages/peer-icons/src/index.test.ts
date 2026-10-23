import { describe, expect, it } from "vitest";
import { DEFAULT_PEER_ICON_ID, PEER_ICONS, peerIconId, peerIconTint } from "./index";

/**
 * The shared half of the peer-icon set.
 *
 * Both apps draw these ids and neither owns the list, so what is worth pinning
 * here is what only this package can break: the ids a label file may contain,
 * how a stored value is resolved, and the palette both clients paint from.
 *
 * The glyph tables (`desktop/src/features/remote-peer/peerGlyphs.ts` and
 * `mobile/src/remote/peerGlyphs.ts`) are deliberately not imported: they need
 * `lucide-react` / `lucide-react-native`, which this package does not depend
 * on, and their `Record<PeerIconId, LucideIcon>` type already fails each app's
 * own typecheck when an id here has no picture there — an earlier check than
 * asserting the same thing at runtime in a third package.
 */

/** `#rrggbb` → the decimal channels the wash is written with. */
function hexChannels(hex: string): number[] {
  return [1, 3, 5].map(offset => Number.parseInt(hex.slice(offset, offset + 2), 16));
}

describe("PEER_ICONS", () => {
  it("lists each id once, in a form a label file can carry", () => {
    const ids = PEER_ICONS.map(icon => icon.id);
    expect(new Set(ids).size).toBe(ids.length);
    for (const id of ids) {
      // A label file written by another build round-trips through this list,
      // and lookup is exact, so the shape has to stay boring: lowercase, no
      // spaces, nothing that survives only until a hand edit re-cases it.
      expect({ id, valid: /^[a-z][a-z0-9-]*$/u.test(id) }).toEqual({ id, valid: true });
    }
  });

  it("gives every mark its own label key", () => {
    // Two entries sharing a key would put the same tooltip under two different
    // pictures, which is indistinguishable from a label that never got written.
    const keys = PEER_ICONS.map(icon => icon.labelKey);
    for (const key of keys) expect(key.length).toBeGreaterThan(0);
    expect(new Set(keys).size).toBe(keys.length);
  });
});

describe("peerIconId", () => {
  it("returns every id in the set unchanged", () => {
    for (const icon of PEER_ICONS) expect(peerIconId(icon.id)).toBe(icon.id);
  });

  it("never hands back a value that is not in the set", () => {
    // Stored text is not trusted: an id from an older build, a hand-edited
    // label, a value that differs only in case, and the object-prototype names
    // a bare `Record` lookup would happily answer are all drawn as the default
    // instead of on screen as themselves.
    for (const stored of ["", " ", "Desktop", "desktop ", "rocket-ship", "constructor", "__proto__", "toString", "0"]) {
      expect({ stored, id: peerIconId(stored) }).toEqual({ stored, id: DEFAULT_PEER_ICON_ID });
    }
    for (const stored of [null, undefined]) expect(peerIconId(stored)).toBe(DEFAULT_PEER_ICON_ID);
  });

  it("defaults to a real entry, and to the first one", () => {
    // The default is drawn in place of a machine's name in a list, so it has to
    // be a glyph and not a blank — and keeping it equal to the first entry is
    // what stops the fallback and the set's own order from drifting apart.
    expect(PEER_ICONS.map(icon => icon.id)).toContain(DEFAULT_PEER_ICON_ID);
    expect(PEER_ICONS[0]?.id).toBe(DEFAULT_PEER_ICON_ID);
  });
});

describe("peerIconTint", () => {
  it("paints every mark, deriving its wash from its own ink", () => {
    for (const icon of PEER_ICONS) {
      const tint = peerIconTint(icon.id);
      expect({ id: icon.id, color: /^#[0-9a-f]{6}$/u.test(tint.color) }).toEqual({ id: icon.id, color: true });
      // The chip behind a mark is that mark's own hue at low alpha, not a
      // second palette entry: a hand-written pair is what lets a wash drift
      // away from the glyph it sits behind.
      const channels = hexChannels(tint.color);
      expect({ id: icon.id, soft: new RegExp(`^rgba\\(${channels.join(", ")}, 0\\.\\d+\\)$`).test(tint.soft) })
        .toEqual({ id: icon.id, soft: true });
    }
  });

  it("paints an unknown id with the default mark's tint", () => {
    // Same resolution as the glyph: an unreadable stored id must not become a
    // bare glyph on an untinted chip.
    const fallback = peerIconTint(DEFAULT_PEER_ICON_ID);
    for (const stored of ["not-an-icon", "", "Desktop", null, undefined, "constructor"]) {
      expect({ stored, tint: peerIconTint(stored) }).toEqual({ stored, tint: fallback });
    }
  });

  it("tells the marks apart by colour rather than naming one hue twice", () => {
    // Decoration with a job: twenty marks in a grid stay quick to scan only if
    // the palette actually varies. A few hues are shared on purpose (the
    // machines that are the same kind of box), so this is a floor, not a count.
    const colors = new Set(PEER_ICONS.map(icon => peerIconTint(icon.id).color));
    expect(colors.size).toBeGreaterThan(10);
  });
});
