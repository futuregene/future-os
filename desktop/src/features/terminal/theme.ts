import type { ResolvedTheme, ThemePalette } from "../../lib/theme";
import { getResolvedTheme, getThemePalette, useResolvedTheme, useThemePalette } from "../../lib/theme";

/**
 * Terminal palette.
 *
 * The palette tracks the app theme: a dark terminal inside a light window (or a
 * white one inside a dark window) reads as a rendering bug. Each variant uses
 * its theme's own tokens (`desktop/src/styles/globals.css`) rather than xterm's
 * defaults.
 *
 * ANSI colors are re-mapped per background (the same approach VS Code's Light+
 * and Dark+ take): programs write colors assuming a dark terminal — white/
 * bright-* text is the norm on Windows (PowerShell prompt, PSReadLine syntax
 * highlighting) and xterm.js draws bold text in bright colors by default — so
 * the light spellings must land on dark-enough swatches to stay readable on
 * white, and the dark spellings on light-enough ones to stay readable on the
 * dark surface. `theme.test.ts` pins a minimum WCAG contrast for every entry in
 * every palette, in both renditions.
 *
 * Every palette in `lib/theme.ts` needs one entry here for each rendition, or a
 * user who picks it gets a terminal in someone else's colors.
 */
export const TERMINAL_THEME = {
  background: "#ffffff",
  foreground: "#172033",
  cursor: "#2563eb",
  cursorAccent: "#ffffff",
  selectionBackground: "#dbeafe",
  black: "#172033",
  red: "#dc2626",
  green: "#15803d",
  yellow: "#b45309",
  blue: "#2563eb",
  magenta: "#7c3aed",
  cyan: "#0e7490",
  // "white" is what dark-terminal programs use for plain text; on a light
  // background it must read as near-ink, not as white-on-white.
  white: "#4b5563",
  brightBlack: "#5d687a",
  brightRed: "#b91c1c",
  brightGreen: "#166534",
  brightYellow: "#854d0e",
  brightBlue: "#1d4ed8",
  brightMagenta: "#6d28d9",
  brightCyan: "#155e75",
  brightWhite: "#8a94a6",
} as const;

/**
 * Dark counterpart. The ANSI spellings stay close to a standard dark terminal
 * palette, but each one is light enough to clear the contrast floor on the
 * dark `code-surface` background — including `black`, which programs do emit
 * (and which would vanish if it kept the light palette's near-ink value).
 */
export const TERMINAL_DARK_THEME = {
  background: "#0f141a",
  foreground: "#e6ebf2",
  cursor: "#60a5fa",
  cursorAccent: "#0f141a",
  selectionBackground: "#1e3a5f",
  black: "#4a5563",
  red: "#f87171",
  green: "#4ade80",
  yellow: "#fbbf24",
  blue: "#60a5fa",
  magenta: "#c084fc",
  cyan: "#22d3ee",
  // On a dark background "white" is the plain-text color: it must read as
  // near-white, while `brightWhite` may be pure white for emphasis.
  white: "#e6ebf2",
  brightBlack: "#78828f",
  brightRed: "#fca5a5",
  brightGreen: "#86efac",
  brightYellow: "#fcd34d",
  brightBlue: "#93c5fd",
  brightMagenta: "#d8b4fe",
  brightCyan: "#67e8f9",
  brightWhite: "#ffffff",
} as const;

/**
 * xterm's `ITheme` shape, restricted to the keys every palette must fill.
 *
 * A structural type rather than `typeof TERMINAL_THEME`: with `as const` the
 * literal palette's type pins each color to its own hex value, which no other
 * palette could satisfy.
 */
export interface TerminalTheme {
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
 * Nord: Frost blues on Snow Storm (light) / Polar Night (dark). `white` is the
 * body-text spelling, so it follows the app's `ink` role in both renditions
 * (Nord's Snow Storm is explicit that "white" text on light is Polar Night).
 */
export const NORD_TERMINAL_THEME = {
  background: "#ffffff",
  foreground: "#2e3440",
  cursor: "#46708f",
  cursorAccent: "#ffffff",
  selectionBackground: "#dbe6f2",
  black: "#2e3440",
  red: "#a63d48",
  green: "#4a7340",
  yellow: "#8a6215",
  blue: "#46708f",
  magenta: "#7a5a94",
  cyan: "#2f7282",
  white: "#4c566a",
  brightBlack: "#6f7c93",
  brightRed: "#8f2f3a",
  brightGreen: "#3b6334",
  brightYellow: "#77540f",
  brightBlue: "#395c78",
  brightMagenta: "#684a83",
  brightCyan: "#21616f",
  brightWhite: "#77839a",
} as const;

export const NORD_DARK_TERMINAL_THEME = {
  background: "#2b303b",
  foreground: "#e5e9f0",
  cursor: "#88c0d0",
  cursorAccent: "#2b303b",
  selectionBackground: "#4c566a",
  black: "#59647a",
  red: "#e8a291",
  green: "#a3be8c",
  yellow: "#ebcb8b",
  blue: "#88c0d0",
  magenta: "#b48ead",
  cyan: "#8fbcbb",
  white: "#e5e9f0",
  brightBlack: "#8590a5",
  brightRed: "#f0b8a9",
  brightGreen: "#b9d0a7",
  brightYellow: "#f2dab0",
  brightBlue: "#a3d3e0",
  brightMagenta: "#c8a6c2",
  brightCyan: "#a8d3d2",
  brightWhite: "#ffffff",
} as const;

/** Solarized: Ethan Schoonover's own ANSI spellings, on base3 / base03. */
export const SOLARIZED_TERMINAL_THEME = {
  background: "#fdf6e3",
  foreground: "#073642",
  cursor: "#1e6fa8",
  cursorAccent: "#fdf6e3",
  selectionBackground: "#e2ddc7",
  black: "#073642",
  red: "#b52a27",
  green: "#5f6f00",
  yellow: "#8b6a00",
  blue: "#1e6fa8",
  magenta: "#8a4b8f",
  cyan: "#2a7c85",
  white: "#586e75",
  brightBlack: "#6d7f80",
  brightRed: "#9a2420",
  brightGreen: "#525f00",
  brightYellow: "#785b00",
  brightBlue: "#1a5f92",
  brightMagenta: "#79417d",
  brightCyan: "#246b73",
  brightWhite: "#7c8a87",
} as const;

export const SOLARIZED_DARK_TERMINAL_THEME = {
  background: "#002b36",
  foreground: "#eee8d5",
  cursor: "#5fb3e8",
  cursorAccent: "#002b36",
  selectionBackground: "#12505f",
  black: "#5b7176",
  red: "#f08a7f",
  green: "#b5bb63",
  yellow: "#e2c261",
  blue: "#6cb6e8",
  magenta: "#d69fd4",
  cyan: "#7fd0d8",
  white: "#eee8d5",
  brightBlack: "#8ba09c",
  brightRed: "#f7aca3",
  brightGreen: "#cad08a",
  brightYellow: "#eed287",
  brightBlue: "#9bcdf0",
  brightMagenta: "#e3bde1",
  brightCyan: "#a6e2e8",
  brightWhite: "#fdf6e3",
} as const;

/** Gruvbox: Pavel Pertsev's palette on bg0 (dark) / bg1 (light). */
export const GRUVBOX_TERMINAL_THEME = {
  background: "#fbf1c7",
  foreground: "#3c3836",
  cursor: "#076678",
  cursorAccent: "#fbf1c7",
  selectionBackground: "#e2d5ab",
  black: "#3c3836",
  red: "#9d0006",
  green: "#4f6a12",
  yellow: "#9a6207",
  blue: "#076678",
  magenta: "#8f3f71",
  cyan: "#427b58",
  white: "#665c54",
  brightBlack: "#7e7364",
  brightRed: "#870005",
  brightGreen: "#445c0f",
  brightYellow: "#865506",
  brightBlue: "#065969",
  brightMagenta: "#7c3662",
  brightCyan: "#396b4c",
  brightWhite: "#928374",
} as const;

export const GRUVBOX_DARK_TERMINAL_THEME = {
  background: "#1b1b1b",
  foreground: "#ebdbb2",
  cursor: "#83a598",
  cursorAccent: "#1b1b1b",
  selectionBackground: "#504945",
  black: "#665c54",
  red: "#fb7564",
  green: "#b8bb26",
  yellow: "#fabd2f",
  blue: "#83a598",
  magenta: "#d3869b",
  cyan: "#8ec07c",
  white: "#ebdbb2",
  brightBlack: "#a89984",
  brightRed: "#fd9385",
  brightGreen: "#c9cc5e",
  brightYellow: "#fbcb62",
  brightBlue: "#a6c1b7",
  brightMagenta: "#dfa8b8",
  brightCyan: "#addaa2",
  brightWhite: "#fbf1c7",
} as const;

/** Rosé Pine Dawn / Moon, using the theme's own `terminal.ansi*` values. */
export const ROSEPINE_TERMINAL_THEME = {
  background: "#fffaf3",
  foreground: "#575279",
  cursor: "#7c6595",
  cursorAccent: "#fffaf3",
  selectionBackground: "#f2e9e1",
  black: "#575279",
  red: "#a4475e",
  green: "#286983",
  yellow: "#9a5f10",
  blue: "#4a6fa5",
  magenta: "#7c6595",
  cyan: "#3f7180",
  white: "#6e6a86",
  brightBlack: "#87829b",
  brightRed: "#8f3d52",
  brightGreen: "#225b72",
  brightYellow: "#85520e",
  brightBlue: "#3f5f8e",
  brightMagenta: "#6b5682",
  brightCyan: "#36626f",
  brightWhite: "#7f7a93",
} as const;

export const ROSEPINE_DARK_TERMINAL_THEME = {
  background: "#1e1c2c",
  foreground: "#e0def4",
  cursor: "#c4a7e7",
  cursorAccent: "#1e1c2c",
  selectionBackground: "#403d52",
  black: "#6e6a86",
  red: "#eb6f92",
  green: "#9ccfd8",
  yellow: "#f6c177",
  blue: "#a3b8e8",
  magenta: "#c4a7e7",
  cyan: "#9ccfd8",
  white: "#e0def4",
  brightBlack: "#908caa",
  brightRed: "#f092ac",
  brightGreen: "#b6dee4",
  brightYellow: "#f8d29a",
  brightBlue: "#bacaf0",
  brightMagenta: "#d6c1f0",
  brightCyan: "#b6dee4",
  brightWhite: "#f6f4ff",
} as const;

/** Catppuccin Latte / Mocha, using the theme's own `terminal.ansi*` values. */
export const CATPPUCCIN_TERMINAL_THEME = {
  background: "#ffffff",
  foreground: "#4c4f69",
  cursor: "#3b5bdb",
  cursorAccent: "#ffffff",
  selectionBackground: "#d9e0f5",
  black: "#5c5f77",
  red: "#c10f33",
  green: "#2f7d1f",
  yellow: "#9c6300",
  blue: "#3b5bdb",
  magenta: "#8839ef",
  cyan: "#1f7087",
  white: "#6c6f85",
  brightBlack: "#898ca4",
  brightRed: "#a80d2c",
  brightGreen: "#286b1a",
  brightYellow: "#875600",
  brightBlue: "#324ec2",
  brightMagenta: "#7730d1",
  brightCyan: "#1a6074",
  brightWhite: "#83879a",
} as const;

export const CATPPUCCIN_DARK_TERMINAL_THEME = {
  background: "#181825",
  foreground: "#cdd6f4",
  cursor: "#89b4fa",
  cursorAccent: "#181825",
  selectionBackground: "#45475a",
  black: "#585b70",
  red: "#f38ba8",
  green: "#a6e3a1",
  yellow: "#f9e2af",
  blue: "#89b4fa",
  magenta: "#f5c2e7",
  cyan: "#94e2d5",
  white: "#cdd6f4",
  brightBlack: "#7f849c",
  brightRed: "#f7aec1",
  brightGreen: "#c1ecbd",
  brightYellow: "#fbecc5",
  brightBlue: "#a8c8fb",
  brightMagenta: "#f8d6ee",
  brightCyan: "#b3ebe1",
  brightWhite: "#ffffff",
} as const;

/**
 * Palette + rendition → terminal colors. Exhaustive over `ThemePalette`, so a
 * palette added to `lib/theme.ts` fails to compile here until its terminal
 * colors exist.
 */
const TERMINAL_THEMES: Record<ThemePalette, Record<ResolvedTheme, TerminalTheme>> = {
  "futureos": { light: TERMINAL_THEME, dark: TERMINAL_DARK_THEME },
  "nord": { light: NORD_TERMINAL_THEME, dark: NORD_DARK_TERMINAL_THEME },
  "solarized": { light: SOLARIZED_TERMINAL_THEME, dark: SOLARIZED_DARK_TERMINAL_THEME },
  "gruvbox": { light: GRUVBOX_TERMINAL_THEME, dark: GRUVBOX_DARK_TERMINAL_THEME },
  "rose-pine": { light: ROSEPINE_TERMINAL_THEME, dark: ROSEPINE_DARK_TERMINAL_THEME },
  "catppuccin": { light: CATPPUCCIN_TERMINAL_THEME, dark: CATPPUCCIN_DARK_TERMINAL_THEME },
};

export function terminalTheme(palette: ThemePalette, theme: ResolvedTheme): TerminalTheme {
  return TERMINAL_THEMES[palette][theme];
}

/**
 * The terminal colors to paint right now, read imperatively.
 *
 * A mount effect cannot use the hook values (they belong to the render that
 * created the effect), so `TerminalView` calls this for xterm's construction and
 * `useTerminalTheme` for the repaint when the palette or rendition changes.
 */
export function currentTerminalTheme(): TerminalTheme {
  return TERMINAL_THEMES[getThemePalette()][getResolvedTheme()];
}

/** The terminal colors the open terminal should repaint with. */
export function useTerminalTheme(): TerminalTheme {
  const theme = useResolvedTheme();
  const palette = useThemePalette();
  return TERMINAL_THEMES[palette][theme];
}
