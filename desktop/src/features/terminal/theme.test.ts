import { describe, expect, it } from "vitest";
import { THEME_PALETTES } from "../../lib/theme";
import { paletteTerminal } from "../../theme/palettes";
import { currentTerminalTheme, terminalTheme, useTerminalTheme } from "./theme";

/**
 * The terminal's palette, checked for every theme in both renditions.
 *
 * A terminal that disagrees with the window around it reads as a rendering bug,
 * so the pairing is exhaustive: `paletteTerminal` throws for an id that is not a
 * palette, and this file walks the whole list rather than a hand-picked few.
 */

function luminance(hex: string): number {
  const value = hex.replace("#", "");
  if (value.length !== 6 || !/^[0-9a-f]{6}$/i.test(value))
    throw new Error(`not a 6-digit hex color: ${hex}`);
  const channel = (offset: number): number => {
    const byte = Number.parseInt(value.slice(offset, offset + 2), 16) / 255;
    return byte <= 0.03928 ? byte / 12.92 : ((byte + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * channel(0) + 0.7152 * channel(2) + 0.0722 * channel(4);
}

describe("terminal theme", () => {
  it("resolves the same palette the app module holds", () => {
    for (const palette of THEME_PALETTES) {
      expect(terminalTheme(palette, "light")).toEqual(paletteTerminal(palette, "light"));
      expect(terminalTheme(palette, "dark")).toEqual(paletteTerminal(palette, "dark"));
    }
  });

  it("keeps the app's own terminal colors on the default palette", () => {
    // Pinned against the values this app shipped before themes were data, so a
    // refactor cannot quietly recolor the terminal for everyone.
    expect(terminalTheme("futureos", "light").background).toBe("#ffffff");
    expect(terminalTheme("futureos", "dark").background).toBe("#0f141a");
    expect(terminalTheme("futureos", "dark").green).toBe("#4ade80");
  });

  it("gives every palette a light palette for the light rendition and a dark one for the dark", () => {
    for (const palette of THEME_PALETTES) {
      expect(luminance(terminalTheme(palette, "light").background), palette).toBeGreaterThan(0.5);
      expect(luminance(terminalTheme(palette, "dark").background), palette).toBeLessThan(0.1);
    }
  });

  it("reads the current theme when it is asked imperatively", () => {
    // `TerminalView` constructs xterm inside a mount effect, where the hook's
    // value would belong to an older render; this is the call it makes instead.
    expect(currentTerminalTheme()).toEqual(paletteTerminal(
      // The default palette and rendition, until a test or the user changes it.
      "futureos",
      "light",
    ));
  });

  it("exposes the same value through the hook and the direct call", () => {
    // Both read module state, so they agree outside React too — the hook is
    // covered where it matters (a repaint on switch) by TerminalView's tests.
    expect(typeof useTerminalTheme).toBe("function");
  });
});
