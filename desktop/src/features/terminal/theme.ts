/**
 * Terminal palette.
 *
 * The app is light-only, so these are the app's own tokens
 * (`desktop/tailwind.config.js`) rather than xterm's dark default — a dark
 * terminal inside a white window reads as a rendering bug.
 *
 * ANSI colors are re-mapped for a light background (the same approach VS Code
 * Light+ takes): programs write colors assuming a dark terminal — white/
 * bright-* text is the norm on Windows (PowerShell prompt, PSReadLine syntax
 * highlighting) and xterm.js draws bold text in bright colors by default — so
 * the light spellings must land on dark-enough swatches to stay readable on
 * white. `theme.test.ts` pins a minimum WCAG contrast for every entry.
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
