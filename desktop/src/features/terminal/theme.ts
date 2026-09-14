/**
 * Terminal palette.
 *
 * The app is light-only, so these are the app's own tokens
 * (`desktop/tailwind.config.js`) rather than xterm's dark default — a dark
 * terminal inside a white window reads as a rendering bug.
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
  white: "#e8edf4",
  brightBlack: "#5d687a",
  brightRed: "#ef4444",
  brightGreen: "#16a34a",
  brightYellow: "#d97706",
  brightBlue: "#3b82f6",
  brightMagenta: "#8b5cf6",
  brightCyan: "#0891b2",
  brightWhite: "#f8faff",
} as const;
