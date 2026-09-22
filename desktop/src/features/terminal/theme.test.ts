import { describe, expect, it } from "vitest";
import { TERMINAL_THEME } from "./theme";

/**
 * WCAG 2.x relative luminance (sRGB) and contrast ratio.
 *
 * Every ANSI foreground must stay readable on the terminal's white
 * background: programs write colors assuming a dark terminal (PowerShell's
 * prompt and PSReadLine output white / bright-* text), and a too-light
 * spelling renders as white-on-white. These bounds pin the fix so a palette
 * edit cannot silently reintroduce invisible text.
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

describe("terminal theme", () => {
  it("keeps the terminal light to match the light-only app", () => {
    expect(luminance(TERMINAL_THEME.background)).toBeGreaterThan(0.5);
  });

  it("every ANSI foreground stays readable on the background", () => {
    const foregrounds = {
      black: TERMINAL_THEME.black,
      red: TERMINAL_THEME.red,
      green: TERMINAL_THEME.green,
      yellow: TERMINAL_THEME.yellow,
      blue: TERMINAL_THEME.blue,
      magenta: TERMINAL_THEME.magenta,
      cyan: TERMINAL_THEME.cyan,
      white: TERMINAL_THEME.white,
      brightBlack: TERMINAL_THEME.brightBlack,
      brightRed: TERMINAL_THEME.brightRed,
      brightGreen: TERMINAL_THEME.brightGreen,
      brightYellow: TERMINAL_THEME.brightYellow,
      brightBlue: TERMINAL_THEME.brightBlue,
      brightMagenta: TERMINAL_THEME.brightMagenta,
      brightCyan: TERMINAL_THEME.brightCyan,
      brightWhite: TERMINAL_THEME.brightWhite,
    };
    for (const [name, color] of Object.entries(foregrounds)) {
      // 3:1 is the floor for the dimmest deliberate text; the plain-text
      // spellings (white/black, what programs use for body text) must meet
      // the 4.5:1 normal-text bar.
      const floor = name === "white" || name === "black" ? 4.5 : 3;
      const ratio = contrast(color, TERMINAL_THEME.background);
      expect(
        ratio,
        `ANSI ${name} (${color}) contrast ${ratio.toFixed(2)}:1 is below ${floor}:1 on ${TERMINAL_THEME.background}`,
      ).toBeGreaterThanOrEqual(floor);
    }
  });

  it("keeps the ANSI ordering: white darker than brightWhite, black darker than brightBlack", () => {
    expect(luminance(TERMINAL_THEME.white)).toBeLessThan(
      luminance(TERMINAL_THEME.brightWhite),
    );
    expect(luminance(TERMINAL_THEME.black)).toBeLessThan(
      luminance(TERMINAL_THEME.brightBlack),
    );
  });
});
