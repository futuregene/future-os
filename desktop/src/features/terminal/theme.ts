import type { ResolvedTheme } from "../../lib/theme";

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
 * both palettes.
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

export function terminalTheme(theme: ResolvedTheme) {
  return theme === "dark" ? TERMINAL_DARK_THEME : TERMINAL_THEME;
}
