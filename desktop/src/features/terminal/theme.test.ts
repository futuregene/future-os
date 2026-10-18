import { describe, expect, it } from "vitest";
import { TERMINAL_DARK_THEME, TERMINAL_THEME, terminalTheme } from "./theme";

/**
 * WCAG 2.x relative luminance (sRGB) and contrast ratio.
 *
 * Every ANSI foreground must stay readable on its palette's background:
 * programs write colors assuming a dark terminal (PowerShell's prompt and
 * PSReadLine output white / bright-* text), and a too-light spelling on white —
 * or a too-dark one on the dark surface — renders as invisible text. These
 * bounds pin both palettes so an edit cannot silently reintroduce that.
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

const palettes: Array<{
  name: string;
  theme: typeof TERMINAL_THEME | typeof TERMINAL_DARK_THEME;
  /** ANSI spellings programs use for plain body text (4.5:1 floor). */
  bodyText: string[];
  /** Spellings that are deliberately invisible on this background. */
  exempt: string[];
}> = [
  {
    name: "light",
    theme: TERMINAL_THEME,
    bodyText: ["black", "white"],
    exempt: [],
  },
  {
    name: "dark",
    theme: TERMINAL_DARK_THEME,
    // On a dark terminal `black` is a background / deliberately-invisible
    // color (ANSI 40), exactly as in every dark terminal palette.
    bodyText: ["white"],
    exempt: ["black"],
  },
];

describe("terminal theme", () => {
  it("ships a light palette for the light app theme and a dark one for the dark", () => {
    expect(luminance(TERMINAL_THEME.background)).toBeGreaterThan(0.5);
    expect(luminance(TERMINAL_DARK_THEME.background)).toBeLessThan(0.1);
    expect(terminalTheme("light")).toBe(TERMINAL_THEME);
    expect(terminalTheme("dark")).toBe(TERMINAL_DARK_THEME);
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
});
