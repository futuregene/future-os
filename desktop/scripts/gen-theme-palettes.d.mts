/**
 * Types for `gen-theme-palettes.mjs`.
 *
 * The generator stays a plain node script (it must run without a bundler), and
 * `src/styles/globals.test.ts` imports its render functions to compare the
 * stylesheet with the theme data. A declaration file is how TypeScript can type
 * that import; `allowJs` stays off, so the script itself is not type-checked.
 */
export declare const STYLESHEET: string;
export declare const PALETTES_JSON: string;
export declare const OPEN_MARKER: string;
export declare const CLOSE_MARKER: string;

export interface PaletteDocument {
  themes: Array<{
    id: string;
    origin: string;
    light: Record<string, string>;
    dark: Record<string, string>;
  }>;
}

export declare function renderPaletteSection(palettes: PaletteDocument): string;
export declare function extractPaletteSection(css: string): string;
export declare function applyPaletteSection(css: string, section: string): string;
