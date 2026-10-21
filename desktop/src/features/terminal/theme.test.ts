import type { TerminalTheme } from "./theme";
import { describe, expect, it } from "vitest";
import { THEME_PALETTES } from "../../lib/theme";
import {
  CATPPUCCIN_DARK_TERMINAL_THEME,
  CATPPUCCIN_TERMINAL_THEME,
  GRUVBOX_DARK_TERMINAL_THEME,
  GRUVBOX_TERMINAL_THEME,
  NORD_DARK_TERMINAL_THEME,
  NORD_TERMINAL_THEME,
  ROSEPINE_DARK_TERMINAL_THEME,
  ROSEPINE_TERMINAL_THEME,
  SOLARIZED_DARK_TERMINAL_THEME,
  SOLARIZED_TERMINAL_THEME,
  TERMINAL_DARK_THEME,
  TERMINAL_THEME,
  terminalTheme,
} from "./theme";

/**
 * WCAG 2.x relative luminance (sRGB) and contrast ratio.
 *
 * Every ANSI foreground must stay readable on its palette's background:
 * programs write colors assuming a dark terminal (PowerShell's prompt and
 * PSReadLine output white / bright-* text), and a too-light spelling on white —
 * or a too-dark one on the dark surface — renders as invisible text. These
 * bounds pin every palette, so an edit (or a new theme) cannot silently
 * reintroduce that.
 */

function luminance(hex: string): number {
  const value = hex.replace("#", "");
  if (value.length !== 6 || !/^[0-9a-f]{6}$/i.test(value)) {
    throw new Error(`not a 6-digit hex color: ${hex}`);
  }
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

interface PaletteCase {
  name: string;
  theme: TerminalTheme;
  /** ANSI spellings programs use for plain body text (4.5:1 floor). */
  bodyText: string[];
  /** Spellings that are deliberately invisible on this background. */
  exempt: string[];
}

const LIGHT = { bodyText: ["black", "white"], exempt: [] };
// On a dark terminal `black` is a background / deliberately-invisible color
// (ANSI 40), exactly as in every dark terminal palette.
const DARK = { bodyText: ["white"], exempt: ["black"] };

const palettes: PaletteCase[] = [
  { name: "futureos light", theme: TERMINAL_THEME, ...LIGHT },
  { name: "futureos dark", theme: TERMINAL_DARK_THEME, ...DARK },
  { name: "nord light", theme: NORD_TERMINAL_THEME, ...LIGHT },
  { name: "nord dark", theme: NORD_DARK_TERMINAL_THEME, ...DARK },
  { name: "solarized light", theme: SOLARIZED_TERMINAL_THEME, ...LIGHT },
  { name: "solarized dark", theme: SOLARIZED_DARK_TERMINAL_THEME, ...DARK },
  { name: "gruvbox light", theme: GRUVBOX_TERMINAL_THEME, ...LIGHT },
  { name: "gruvbox dark", theme: GRUVBOX_DARK_TERMINAL_THEME, ...DARK },
  { name: "rose-pine light", theme: ROSEPINE_TERMINAL_THEME, ...LIGHT },
  { name: "rose-pine dark", theme: ROSEPINE_DARK_TERMINAL_THEME, ...DARK },
  { name: "catppuccin light", theme: CATPPUCCIN_TERMINAL_THEME, ...LIGHT },
  { name: "catppuccin dark", theme: CATPPUCCIN_DARK_TERMINAL_THEME, ...DARK },
];

describe("terminal theme", () => {
  it("ships a light palette for the light app theme and a dark one for the dark", () => {
    expect(luminance(TERMINAL_THEME.background)).toBeGreaterThan(0.5);
    expect(luminance(TERMINAL_DARK_THEME.background)).toBeLessThan(0.1);
    expect(terminalTheme("futureos", "light")).toBe(TERMINAL_THEME);
    expect(terminalTheme("futureos", "dark")).toBe(TERMINAL_DARK_THEME);
  });

  it("defines a terminal palette for every app palette, in both renditions", () => {
    for (const palette of THEME_PALETTES) {
      const light = terminalTheme(palette, "light");
      const dark = terminalTheme(palette, "dark");
      // A palette that fell back to another theme's colors would paint the
      // terminal in someone else's hues.
      expect(light, `${palette} light`).toBeDefined();
      expect(dark, `${palette} dark`).toBeDefined();
      expect(light.background, `${palette} light background`).not.toBe(dark.background);
    }
    // The default palette keeps its long-standing colors.
    expect(terminalTheme("futureos", "light")).toBe(TERMINAL_THEME);
    expect(terminalTheme("futureos", "dark")).toBe(TERMINAL_DARK_THEME);
  });

  it.each(palettes)("puts the light and dark background on the right side of the palette", ({ name, theme }) => {
    const light = name.endsWith("light");
    expect(luminance(theme.background) > 0.5, name).toBe(light);
  });

  it.each(palettes)("keeps every ANSI foreground readable in the $name palette", ({ theme, bodyText, exempt }) => {
    const foregrounds = {
      black: theme.black,
      red: theme.red,
      green: theme.green,
      yellow: theme.yellow,
      blue: theme.blue,
      magenta: theme.magenta,
      cyan: theme.cyan,
      white: theme.white,
      brightBlack: theme.brightBlack,
      brightRed: theme.brightRed,
      brightGreen: theme.brightGreen,
      brightYellow: theme.brightYellow,
      brightBlue: theme.brightBlue,
      brightMagenta: theme.brightMagenta,
      brightCyan: theme.brightCyan,
      brightWhite: theme.brightWhite,
    };
    for (const [name, color] of Object.entries(foregrounds)) {
      if (exempt.includes(name))
        continue;
      // 3:1 is the floor for the dimmest deliberate text; the plain-text
      // spellings (what programs use for body text) must meet the 4.5:1
      // normal-text bar.
      const floor = bodyText.includes(name) ? 4.5 : 3;
      const ratio = contrast(color, theme.background);
      expect(
        ratio,
        `ANSI ${name} (${color}) contrast ${ratio.toFixed(2)}:1 is below ${floor}:1 on ${theme.background}`,
      ).toBeGreaterThanOrEqual(floor);
    }
  });

  it.each(palettes)("keeps the ANSI ordering in the $name palette", ({ theme }) => {
    expect(luminance(theme.white)).toBeLessThan(
      luminance(theme.brightWhite),
    );
    expect(luminance(theme.black)).toBeLessThan(
      luminance(theme.brightBlack),
    );
  });

  it.each(palettes)("keeps the cursor and the body text readable in the $name palette", ({ theme }) => {
    // The cursor is a 1px bar over the text it sits on, so it needs the same
    // 3:1 floor as the dimmest text rather than none at all.
    expect(contrast(theme.cursor, theme.background)).toBeGreaterThanOrEqual(3);
    expect(contrast(theme.foreground, theme.background)).toBeGreaterThanOrEqual(4.5);
    // A selection is a background: the text drawn on it must survive.
    expect(contrast(theme.foreground, theme.selectionBackground)).toBeGreaterThanOrEqual(3);
  });
});
