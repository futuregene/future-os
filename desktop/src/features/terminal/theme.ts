import type { ResolvedTheme, ThemePalette } from "../../lib/theme";
import type { TerminalPalette } from "../../theme/palettes";
import { getResolvedTheme, getThemePalette, useResolvedTheme, useThemePalette } from "../../lib/theme";
import { paletteTerminal } from "../../theme/palettes";

/**
 * The embedded terminal's colors.
 *
 * xterm keeps its own palette, so the CSS-variable flip that repaints the rest
 * of the window does not reach it: the terminal has to be handed colors. They
 * come from the same data as the theme itself (`src/theme/palettes.json`), so a
 * color theme cannot leave the terminal in some other theme's colors — the
 * entries are generated alongside the CSS, and `theme.test.ts` checks every
 * palette's contrast.
 *
 * Each palette has a light and a dark rendition, mirroring the app: a dark
 * terminal inside a light window (or the reverse) reads as a rendering bug.
 * The ANSI spellings are re-mapped per background (the approach VS Code's
 * Light+ / Dark+ take): programs write colors assuming a dark terminal — white
 * and bright-* text is the norm on Windows (PowerShell prompt, PSReadLine
 * syntax highlighting) and xterm.js draws bold text in bright colors — so on a
 * light background `white` has to be near-ink and every other spelling dark
 * enough to read, while on a dark one `white` has to be near-white.
 */
export type TerminalTheme = TerminalPalette;

/** The terminal colors for a palette in a rendition. */
export function terminalTheme(palette: ThemePalette, theme: ResolvedTheme): TerminalTheme {
  return paletteTerminal(palette, theme);
}

/**
 * The terminal colors to paint right now, read imperatively.
 *
 * A mount effect cannot use the hook values (they belong to the render that
 * created the effect), so `TerminalView` calls this for xterm's construction and
 * `useTerminalTheme` for the repaint when the palette or rendition changes.
 */
export function currentTerminalTheme(): TerminalTheme {
  return paletteTerminal(getThemePalette(), getResolvedTheme());
}

/** The terminal colors the open terminal should repaint with. */
export function useTerminalTheme(): TerminalTheme {
  const theme = useResolvedTheme();
  const palette = useThemePalette();
  return paletteTerminal(palette, theme);
}
