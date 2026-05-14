/**
 * xihu_tui dark theme — matches pi-mono dark theme colors.
 */
import { CSI, RESET } from "./tui.js";
// 256-color palette (approximate to pi-mono hex values)
export const C = {
    // Accent colors (pi-mono 256-color indices)
    cyan: 45, // #00d7ff
    blue: 69, // #5f87ff
    green: 143, // #b5bd68
    red: 204, // #cc6666
    yellow: 226, // #ffff00
    gray: 244, // #808080
    dimGray: 241, // #626262
    darkGray: 240, // #505050
    accent: 109, // #8abeb7
    selectedBg: 237, // #3a3a4a
    userMsgBg: 59, // #343541
    toolPendingBg: 17, // #00005f
    toolSuccessBg: 22, // #005f00
    toolErrorBg: 52, // #5f0000
    // Markdown
    mdHeading: 221, // #f0c674 (gold)
    mdLink: 117, // #81a2be (light blue)
    mdLinkUrl: 102, // #666666
    mdCode: 151, // #8abeb7 (accent)
    mdCodeBlock: 142, // #b5bd68 (green)
    mdCodeBlockBorder: 244, // gray
    mdQuote: 244, // gray
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
export const DARK_THEME = {
    bg: 235,
    fg: 252,
    accent: 39, // matches pi DEFAULT_THEME accent
    border: 240, // matches pi DEFAULT_THEME border
    selectedBg: 38, // matches pi DEFAULT_THEME selectedBg
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
export function fg(c, text) {
    return `${CSI}38;5;${c}m${text}${RESET}`;
}
export function bg(c, text) {
    return `${CSI}48;5;${c}m${text}${RESET}`;
}
export function bold(text) {
    return `${CSI}1m${text}${RESET}`;
}
export function dim(text) {
    return `${CSI}2m${text}${RESET}`;
}
export function italic(text) {
    return `${CSI}3m${text}${RESET}`;
}
export function underline(text) {
    return `${CSI}4m${text}${RESET}`;
}
export function strikethrough(text) {
    return `${CSI}9m${text}${RESET}`;
}
export function reset(text) {
    return `${RESET}${text}${RESET}`;
}
// ─── Raw style primitives (no auto-RESET, for composable theme building) ──
/** Apply foreground color without trailing RESET. */
export function fgRaw(c, text) {
    return `${CSI}38;5;${c}m${text}`;
}
/** Apply background color without trailing RESET. */
export function bgRaw(c, text) {
    return `${CSI}48;5;${c}m${text}`;
}
/** Apply bold without trailing RESET. */
export function boldRaw(text) {
    return `${CSI}1m${text}`;
}
/** Apply dim without trailing RESET. */
export function dimRaw(text) {
    return `${CSI}2m${text}`;
}
/** Apply italic without trailing RESET. */
export function italicRaw(text) {
    return `${CSI}3m${text}`;
}
/** Apply underline without trailing RESET. */
export function underlineRaw(text) {
    return `${CSI}4m${text}`;
}
/** Apply strikethrough without trailing RESET. */
export function strikethroughRaw(text) {
    return `${CSI}9m${text}`;
}
/** Reverse video without trailing RESET. */
export function reverseRaw(text) {
    return `${CSI}7m${text}`;
}
/**
 * Compose multiple style functions into one.
 * Each fn receives text and returns styled text WITHOUT reset codes —
 * the caller appends the final reset.
 *
 * Example: style("hello", c => fg(151, c), c => bold(c))
 */
export function style(text, ...fns) {
    let result = text;
    for (const fn of fns) {
        result = fn(result);
    }
    return result + RESET;
}
// ─── Thinking ────────────────────────────────────────────────────────────
export function thinkingColor(level) {
    switch (level) {
        case "minimal": return C.thinkingMinimal;
        case "low": return C.thinkingLow;
        case "medium": return C.thinkingMedium;
        case "high": return C.thinkingHigh;
        case "xhigh": return C.thinkingXhigh;
        default: return C.thinkingOff;
    }
}
