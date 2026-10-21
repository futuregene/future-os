import { bundledThemesInfo } from "shiki";
import { describe, expect, it } from "vitest";
import { THEME_PALETTES as MODES } from "../lib/theme";
import { PALETTE_IDS, paletteById, paletteShiki, paletteTerminal, THEME_PALETTES } from "./palettes";

/**
 * The theme data itself: completeness, the two axes, and the contrast floors.
 *
 * These are the checks that let a palette be *data* rather than hand-written
 * CSS in three places. `styles/globals.test.ts` covers the generated stylesheet;
 * this file covers everything the stylesheet cannot: the terminal palettes, the
 * Shiki mapping, and the floors every theme has to clear to be shippable at all.
 */

function luminance(hex: string): number {
  const value = hex.replace("#", "");
  const channel = (offset: number): number => {
    const byte = Number.parseInt(value.slice(offset, offset + 2), 16) / 255;
    return byte <= 0.03928 ? byte / 12.92 : ((byte + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4);
}

function contrast(foreground: string, background: string): number {
  const a = luminance(foreground);
  const b = luminance(background);
  const [light, dark] = a > b ? [a, b] : [b, a];
  return (light + 0.05) / (dark + 0.05);
}

/** The tokens every theme must define, both renditions. */
const REQUIRED_TOKENS = [
  "canvas",
  "surface",
  "surface-panel",
  "surface-subtle",
  "code-surface",
  "line",
  "line-soft",
  "ink",
  "ink-soft",
  "ink-muted",
  "ink-strong",
  "accent",
  "accent-soft",
  "accent-fill",
  "accent-fill-hover",
  "accent-fill-disabled",
  "accent-pulse",
  "focus",
  "success",
  "success-soft",
  "success-line",
  "danger",
  "danger-soft",
  "danger-line",
  "danger-fill",
  "warning",
  "warning-soft",
  "warning-line",
  "info",
  "info-soft",
  "info-line",
  "generating",
  "scrollbar",
  "scrollbar-hover",
  "diff-add",
  "diff-add-line",
  "diff-remove",
  "diff-remove-line",
  "overlay",
];

/**
 * The floors a theme has to clear. They are the reason a palette is *adjusted*
 * rather than copied: several upstream themes are deliberately low-contrast,
 * which reads as broken text at our type sizes.
 */
const FLOORS: Array<[string, string, number]> = [
  ["ink", "canvas", 7],
  ["ink", "surface", 7],
  ["ink", "surface-subtle", 6],
  ["ink", "code-surface", 6],
  ["ink-soft", "surface", 4.5],
  ["ink-soft", "surface-subtle", 4],
  ["ink-muted", "surface", 3],
  ["accent", "surface", 4.5],
  ["success", "surface", 4.5],
  ["danger", "surface", 4.5],
  ["warning", "surface", 4.5],
  ["info", "surface", 4.5],
];

describe("theme palettes", () => {
  it("lists the palettes in the same order the theme module does", () => {
    // The ids are a closed union in `lib/theme.ts` (so a typo cannot become a
    // silently ignored preference) and data here; this keeps them one list.
    expect(PALETTE_IDS).toEqual([...MODES]);
  });

  it("gives every palette an id that is a CSS attribute value", () => {
    for (const id of PALETTE_IDS) {
      expect(id).toBe(id.toLowerCase());
      expect(id).toMatch(/^[a-z0-9-]+$/);
    }
  });

  it.each(PALETTE_IDS)("defines every token in both renditions for %s", (id) => {
    const palette = paletteById(id)!;
    for (const mode of ["light", "dark"] as const) {
      expect(Object.keys(palette[mode]).sort(), `${id} ${mode}`).toEqual([...REQUIRED_TOKENS].sort());
    }
    // No theme restates the search tokens: a hit is a bright mark with dark text
    // in every palette, so they stay shared.
    for (const token of Object.keys(palette.light)) {
      expect(token.startsWith("search-")).toBe(false);
    }
  });

  it.each(PALETTE_IDS)("keeps %s readable against its own surfaces", (id) => {
    const palette = paletteById(id)!;
    for (const mode of ["light", "dark"] as const) {
      const tokens = palette[mode];
      for (const [fg, bg, floor] of FLOORS) {
        const ratio = contrast(tokens[fg]!, tokens[bg]!);
        expect(ratio, `${id} ${mode}: ${fg} on ${bg} is ${ratio.toFixed(2)}:1`)
          .toBeGreaterThanOrEqual(floor);
      }
      // A solid fill carries white text in both renditions.
      for (const fill of ["accent-fill", "danger-fill"]) {
        const ratio = contrast("#ffffff", tokens[fill]!);
        expect(ratio, `${id} ${mode}: white on ${fill} is ${ratio.toFixed(2)}:1`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });

  it.each(PALETTE_IDS)("gives %s a terminal palette that stays readable", (id) => {
    for (const mode of ["light", "dark"] as const) {
      const term = paletteTerminal(id, mode);
      const dark = luminance(term.background) < 0.4;
      // On a dark terminal `black` is a background color (ANSI 40), exactly as in
      // every dark terminal palette; `white` is body text, so it needs the bar.
      const bodyText = dark ? ["white"] : ["black", "white"];
      const exempt = dark ? ["black"] : [];
      for (const key of ["black", "red", "green", "yellow", "blue", "magenta", "cyan", "white", "brightBlack", "brightRed", "brightGreen", "brightYellow", "brightBlue", "brightMagenta", "brightCyan", "brightWhite"] as const) {
        if (exempt.includes(key))
          continue;
        const ratio = contrast(term[key], term.background);
        expect(ratio, `${id} ${mode}: ANSI ${key} is ${ratio.toFixed(2)}:1`)
          .toBeGreaterThanOrEqual(bodyText.includes(key) ? 4.5 : 3);
      }
      expect(contrast(term.foreground, term.background)).toBeGreaterThanOrEqual(4.5);
      expect(contrast(term.cursor, term.background)).toBeGreaterThanOrEqual(3);
      expect(contrast(term.foreground, term.selectionBackground)).toBeGreaterThanOrEqual(3);
    }
  });

  it.each(PALETTE_IDS)("gives %s a syntax theme that exists in Shiki", (id) => {
    const bundled = new Set(bundledThemesInfo.map(theme => theme.id));
    for (const mode of ["light", "dark"] as const) {
      const theme = paletteShiki(id, mode);
      // A typo here would not throw at runtime in every path; it would silently
      // fall back to the default theme.
      expect(bundled.has(theme), `${id} ${mode}: ${theme}`).toBe(true);
    }
  });

  it("ships a light and a dark rendition for every palette", () => {
    for (const palette of THEME_PALETTES) {
      expect(luminance(palette.light.canvas!), palette.id).toBeGreaterThan(0.5);
      expect(luminance(palette.dark.canvas!), palette.id).toBeLessThan(0.4);
      expect(palette.terminal.light.background).not.toBe(palette.terminal.dark.background);
    }
  });

  it("keeps every theme visually distinct from the default", () => {
    const base = paletteById("futureos")!;
    for (const palette of THEME_PALETTES) {
      if (palette.id === "futureos")
        continue;
      expect(palette.light.canvas, palette.id).not.toBe(base.light.canvas);
      expect(palette.light.accent, palette.id).not.toBe(base.light.accent);
      expect(palette.dark.canvas, palette.id).not.toBe(base.dark.canvas);
    }
  });

  it("rejects an unknown palette id rather than painting a wrong theme", () => {
    expect(paletteById("not-a-palette")).toBeUndefined();
    expect(() => paletteTerminal("not-a-palette", "light")).toThrow(/unknown theme palette/);
  });
});
