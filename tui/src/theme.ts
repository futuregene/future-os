/**
 * xihu dark theme — matches pi-mono dark theme colors.
 */

import { CSI, RESET } from "./tui.js";

// 256-color palette (approximate to pi-mono hex values)
export const C = {
  // Accent colors (pi-mono 256-color indices)
  cyan:    45,    // #00d7ff
  blue:    69,    // #5f87ff
  green:   143,   // #b5bd68
  red:     204,   // #cc6666
  yellow:  226,   // #ffff00
  gray:    244,   // #808080
  dimGray: 241,   // #626262
  darkGray: 240,  // #505050
  accent:  109,   // #8abeb7
  selectedBg: 237, // #3a3a4a
  userMsgBg: 59,   // #343541
  toolPendingBg: 17,  // #00005f
  toolSuccessBg: 22,  // #005f00
  toolErrorBg: 52,   // #5f0000

  // Markdown
  mdHeading: 221,    // #f0c674 (gold)
  mdLink: 117,       // #81a2be (light blue)
  mdLinkUrl: 102,     // #666666
  mdCode: 151,        // #8abeb7 (accent)
  mdCodeBlock: 142,   // #b5bd68 (green)
  mdCodeBlockBorder: 244, // gray
  mdQuote: 244,       // gray

  // Thinking levels
  thinkingOff: 240,
  thinkingMinimal: 110,
  thinkingLow: 68,
  thinkingMedium: 117,
  thinkingHigh: 182,
  thinkingXhigh: 213,

  // Text
  fg: 252,
  dim: 245,
};

export interface Theme {
  bg: number;
  fg: number;
  accent: number;
  border: number;
  selectedBg: number;
  selectedFg: number;
  dim: number;
  error: number;
  success: number;

  // Markdown
  mdHeading: number;
  mdLink: number;
  mdCode: number;
  mdCodeBlock: number;
  mdCodeBlockBorder: number;
  mdQuote: number;

  // Tool
  toolPendingBg: number;
  toolSuccessBg: number;
  toolErrorBg: number;
  toolTitle: number;
  toolOutput: number;

  // Thinking
  thinkingOff: number;
  thinkingMinimal: number;
  thinkingLow: number;
  thinkingMedium: number;
  thinkingHigh: number;
  thinkingXhigh: number;
  thinkingText: number;

  // User/assistant messages
  userBg: number;
  assistantBg: number;
}

export const DARK_THEME: Theme = {
  bg: 235,
  fg: 252,
  accent: C.accent,
  border: C.blue,
  selectedBg: C.selectedBg,
  selectedFg: 255,
  dim: C.dim,
  error: C.red,
  success: C.green,

  mdHeading: C.mdHeading,
  mdLink: C.mdLink,
  mdCode: C.mdCode,
  mdCodeBlock: C.mdCodeBlock,
  mdCodeBlockBorder: C.mdCodeBlockBorder,
  mdQuote: C.mdQuote,

  toolPendingBg: C.toolPendingBg,
  toolSuccessBg: C.toolSuccessBg,
  toolErrorBg: C.toolErrorBg,
  toolTitle: C.accent,
  toolOutput: C.gray,

  thinkingOff: C.thinkingOff,
  thinkingMinimal: C.thinkingMinimal,
  thinkingLow: C.thinkingLow,
  thinkingMedium: C.thinkingMedium,
  thinkingHigh: C.thinkingHigh,
  thinkingXhigh: C.thinkingXhigh,
  thinkingText: C.gray,

  userBg: C.userMsgBg,
  assistantBg: 235,
};

// ─── Color helpers ─────────────────────────────────────────────────────

export function fg(c: number, text: string): string {
  return `${CSI}38;5;${c}m${text}${RESET}`;
}

export function bg(c: number, text: string): string {
  return `${CSI}48;5;${c}m${text}${RESET}`;
}

export function bold(text: string): string {
  return `${CSI}1m${text}${RESET}`;
}

export function dim(text: string): string {
  return `${CSI}2m${text}${RESET}`;
}

export function italic(text: string): string {
  return `${CSI}3m${text}${RESET}`;
}

export function underline(text: string): string {
  return `${CSI}4m${text}${RESET}`;
}

export function strikethrough(text: string): string {
  return `${CSI}9m${text}${RESET}`;
}

export function reset(text: string): string {
  return `${RESET}${text}${RESET}`;
}

// ─── Raw style primitives (no auto-RESET, for composable theme building) ──

/** Apply foreground color without trailing RESET. */
export function fgRaw(c: number, text: string): string {
  return `${CSI}38;5;${c}m${text}`;
}

/** Apply background color without trailing RESET. */
export function bgRaw(c: number, text: string): string {
  return `${CSI}48;5;${c}m${text}`;
}

/** Apply bold without trailing RESET. */
export function boldRaw(text: string): string {
  return `${CSI}1m${text}`;
}

/** Apply dim without trailing RESET. */
export function dimRaw(text: string): string {
  return `${CSI}2m${text}`;
}

/** Apply italic without trailing RESET. */
export function italicRaw(text: string): string {
  return `${CSI}3m${text}`;
}

/** Apply underline without trailing RESET. */
export function underlineRaw(text: string): string {
  return `${CSI}4m${text}`;
}

/** Apply strikethrough without trailing RESET. */
export function strikethroughRaw(text: string): string {
  return `${CSI}9m${text}`;
}

/** Reverse video without trailing RESET. */
export function reverseRaw(text: string): string {
  return `${CSI}7m${text}`;
}

/**
 * Compose multiple style functions into one.
 * Each fn receives text and returns styled text WITHOUT reset codes —
 * the caller appends the final reset.
 *
 * Example: style("hello", c => fg(151, c), c => bold(c))
 */
export function style(text: string, ...fns: ((t: string) => string)[]): string {
  let result = text;
  for (const fn of fns) {
    result = fn(result);
  }
  return result + RESET;
}

// ─── Thinking ────────────────────────────────────────────────────────────

export function thinkingColor(level: string): number {
  switch (level) {
    case "minimal": return C.thinkingMinimal;
    case "low":     return C.thinkingLow;
    case "medium":  return C.thinkingMedium;
    case "high":    return C.thinkingHigh;
    case "xhigh":  return C.thinkingXhigh;
    default:        return C.thinkingOff;
  }
}
