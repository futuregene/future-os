import type { BundledTheme } from "shiki";
import type { ResolvedTheme } from "../lib/theme";
import palettes from "./palettes.json";

/**
 * The color themes, as data.
 *
 * `palettes.json` is the single source of truth: it holds each theme's token
 * values for both renditions, its terminal palette and its syntax-highlighting
 * theme. The `[data-palette="…"]` rules in `styles/globals.css` are *generated*
 * from it (`scripts/gen-theme-palettes.mjs`, checked by `styles/globals.test.ts`),
 * and the two places that cannot use CSS variables — the embedded terminal and
 * Shiki — read it here. Adding a theme therefore means editing one JSON object
 * and re-running the generator; nothing else can silently disagree.
 *
 * The shape mirrors opencode's theme files (a palette plus semantic roles, each
 * with a light and a dark value), which is what makes "a theme is data" workable
 * at this many themes. See `docs/internals/desktop/COLOR.md`.
 */

/** A terminal palette: xterm's colors for one theme in one rendition. */
export interface TerminalPalette {
  background: string;
  foreground: string;
  cursor: string;
  cursorAccent: string;
  selectionBackground: string;
  black: string;
  red: string;
  green: string;
  yellow: string;
  blue: string;
  magenta: string;
  cyan: string;
  white: string;
  brightBlack: string;
  brightRed: string;
  brightGreen: string;
  brightYellow: string;
  brightBlue: string;
  brightMagenta: string;
  brightCyan: string;
  brightWhite: string;
}

/**
 * One theme's token values. The keys are the `--color-*` names from `@theme`
 * minus the `search-*` tokens, which every theme shares on purpose (a search
 * hit is a bright mark with dark text regardless of the palette).
 */
export type PaletteTokens = Record<string, string>;

export interface ThemePalette {
  id: string;
  /** One line naming the upstream project, used in the generated CSS banner. */
  origin: string;
  light: PaletteTokens;
  dark: PaletteTokens;
  terminal: Record<ResolvedTheme, TerminalPalette>;
  shiki: Record<ResolvedTheme, BundledTheme>;
}

export const THEME_PALETTES = palettes.themes as ThemePalette[];

/** Palette ids in picker order (the shipped default first, then alphabetical). */
export const PALETTE_IDS = THEME_PALETTES.map(palette => palette.id);

const BY_ID = new Map(THEME_PALETTES.map(palette => [palette.id, palette]));

export function paletteById(id: string): ThemePalette | undefined {
  return BY_ID.get(id);
}

/**
 * A theme's terminal colors. Total for a valid id — a theme with no data is a
 * bug the compiler catches through `PALETTE_IDS`, not a runtime fallback.
 */
export function paletteTerminal(id: string, theme: ResolvedTheme): TerminalPalette {
  const palette = BY_ID.get(id);
  /* v8 ignore next 2 -- only reachable with an id that is not a palette */
  if (!palette)
    throw new Error(`unknown theme palette: ${id}`);
  return palette.terminal[theme];
}

/** The Shiki theme for a palette in a rendition. */
export function paletteShiki(id: string, theme: ResolvedTheme): BundledTheme {
  const palette = BY_ID.get(id);
  /* v8 ignore next 2 -- only reachable with an id that is not a palette */
  if (!palette)
    throw new Error(`unknown theme palette: ${id}`);
  return palette.shiki[theme];
}
