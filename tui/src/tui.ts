/**
 * Core TUI types and utilities.
 * Terminal writes to process.stdout (matches pi's ProcessTerminal).
 */

// ─── ANSI Escape Sequences ─────────────────────────────────────────────────

export const ESC = "\x1b";
export const CSI = ESC + "[";
export const CLEAR = `${CSI}2J`;
export const CLEAR_LINE = `${CSI}2K`;
export const CURSOR_HIDE = `${CSI}?25l`;
export const CURSOR_SHOW = `${CSI}?25h`;
export const RESET = `${CSI}m`;
export const BOLD = `${CSI}1m`;
export const DIM = `${CSI}2m`;

// Synchronized output — terminal buffers all writes between begin/end
// and flushes atomically, preventing flicker and tearing.
export const SYNC_BEGIN = "\x1b[?2026h";
export const SYNC_END = "\x1b[?2026l";

// SGR mouse tracking: sends \x1b[<N;X;YM for all events including wheel
export const MOUSE_TRACK_ON = `${CSI}?1003h`;
export const MOUSE_TRACK_OFF = `${CSI}?1003l`;

// Mouse wheel event codes in SGR protocol
export const MOUSE_SCROLL_UP = 64;
export const MOUSE_SCROLL_DOWN = 65;

export function cursorPos(row: number, col: number): string {
  return `${CSI}${row};${col}H`;
}

export function setFg(c: number): string {
  return `${CSI}38;5;${c}m`;
}
export function setBg(c: number): string {
  return `${CSI}48;5;${c}m`;
}

// ─── Terminal ──────────────────────────────────────────────────────────────

export interface Terminal {
  write(data: string): void;
  clear(): void;
  getWidth(): number;
  getHeight(): number;
  hideCursor(): void;
  showCursor(): void;
  enableMouse(): void;
  disableMouse(): void;
  close(): void;
  moveBy(lines: number): void;
  clearLine(): void;
  clearFromCursor(): void;
}

export class NodeTerminal implements Terminal {

  write(data: string): void {
    process.stdout.write(data);
  }

  clear(): void {
    this.write("\x1b[2J\x1b[H");
  }

  getWidth(): number {
    return process.stdout.columns || 80;
  }

  getHeight(): number {
    return process.stdout.rows || 24;
  }

  hideCursor(): void { this.write("\x1b[?25l"); }
  showCursor(): void { this.write("\x1b[?25h"); }
  enableMouse(): void { this.write(MOUSE_TRACK_ON); }
  disableMouse(): void { this.write(MOUSE_TRACK_OFF); }
  close(): void { /* no-op with stdout */ }

  moveBy(lines: number): void {
    if (lines > 0) this.write(`\x1b[${lines}B`);
    else if (lines < 0) this.write(`\x1b[${-lines}A`);
  }

  clearLine(): void {
    this.write("\x1b[2K");
  }

  clearFromCursor(): void {
    this.write("\x1b[J");
  }
}

// ─── Theme ─────────────────────────────────────────────────────────────────

export interface Theme {
  bg: number;
  fg: number;
  accent: number;
  border: number;
  selectedBg: number;
  selectedFg: number;
  dimFg: number;
  error: number;
  success: number;
}

export const DEFAULT_THEME: Theme = {
  bg: 235,
  fg: 252,
  accent: 39,
  border: 240,
  selectedBg: 38,
  selectedFg: 255,
  dimFg: 245,
  error: 160,
  success: 76,
};

// ─── Component Architecture ─────────────────────────────────────────────────

export interface Component {
  render(width: number): string[];
  handleInput?(data: string): void;
  wantsKeyRelease?: boolean;
  invalidate(): void;
}

export interface Focusable {
  focused: boolean;
}

export function isFocusable(component: Component | null): component is Component & Focusable {
  return component !== null && "focused" in component;
}

export const CURSOR_MARKER = "\x1b_pi:c\x07";

export type InputListenerResult = { consume?: boolean; data?: string } | undefined;
export type InputListener = (data: string) => InputListenerResult;

export class Container implements Component {
  children: Component[] = [];

  addChild(component: Component): void {
    this.children.push(component);
  }

  removeChild(component: Component): void {
    const index = this.children.indexOf(component);
    if (index !== -1) this.children.splice(index, 1);
  }

  clear(): void {
    this.children = [];
  }

  invalidate(): void {
    for (const child of this.children) child.invalidate();
  }

  render(width: number): string[] {
    const lines: string[] = [];
    for (const child of this.children) {
      for (const line of child.render(width)) lines.push(line);
    }
    return lines;
  }
}

// ─── Overlay Positioning ──────────────────────────────────────────────────

export type OverlayAnchor =
  | "center"
  | "top-left" | "top-right" | "top-center"
  | "bottom-left" | "bottom-right" | "bottom-center"
  | "left-center" | "right-center";

export interface OverlayMargin {
  top?: number;
  right?: number;
  bottom?: number;
  left?: number;
}

export type SizeValue = number | `${number}%`;

export function parseSizeValue(v: SizeValue, total: number): number {
  if (typeof v === "number") return v;
  if (v.endsWith("%")) {
    const pct = parseFloat(v);
    if (isNaN(pct)) return 0;
    return Math.floor((pct / 100) * total);
  }
  return 0;
}

export interface OverlayOptions {
  width?: SizeValue;
  minWidth?: number;
  maxHeight?: SizeValue;
  anchor?: OverlayAnchor;
  offsetX?: number;
  offsetY?: number;
  row?: SizeValue;
  col?: SizeValue;
  margin?: OverlayMargin | number;
  visible?: (termWidth: number, termHeight: number) => boolean;
  nonCapturing?: boolean;
}

export interface OverlayLayout {
  row: number;
  col: number;
  width: number;
  maxHeight: number;
}

export function resolveOverlayLayout(
  termWidth: number,
  termHeight: number,
  componentLines: number,
  options?: OverlayOptions,
): OverlayLayout {
  const opt = options ?? {};
  const marginNum = typeof opt.margin === "number" ? opt.margin : 0;
  const margin = typeof opt.margin === "object"
    ? { top: opt.margin.top ?? marginNum, right: opt.margin.right ?? marginNum,
        bottom: opt.margin.bottom ?? marginNum, left: opt.margin.left ?? marginNum }
    : { top: marginNum, right: marginNum, bottom: marginNum, left: marginNum };

  const availW = termWidth - margin.left - margin.right;
  const availH = termHeight - margin.top - margin.bottom;

  let w = opt.width !== undefined ? parseSizeValue(opt.width, termWidth) : Math.min(80, availW);
  if (opt.minWidth !== undefined) w = Math.max(w, opt.minWidth);
  w = Math.max(1, Math.min(w, availW));

  let maxH = opt.maxHeight !== undefined ? parseSizeValue(opt.maxHeight, termHeight) : termHeight;
  maxH = Math.min(maxH, availH);

  const effectiveH = Math.min(componentLines, maxH);
  const anchor = opt.anchor ?? "center";

  let row: number;
  let col: number;

  // Explicit row/col override anchor
  if (opt.row !== undefined) {
    row = parseSizeValue(opt.row, availH);
  } else {
    switch (anchor) {
      case "top-left": case "top-right": case "top-center":
        row = margin.top; break;
      case "bottom-left": case "bottom-right": case "bottom-center":
        row = margin.top + availH - effectiveH; break;
      default: // center, left-center, right-center
        row = margin.top + Math.floor((availH - effectiveH) / 2);
    }
  }

  if (opt.col !== undefined) {
    col = parseSizeValue(opt.col, availW);
  } else {
    switch (anchor) {
      case "top-left": case "bottom-left": case "left-center":
        col = margin.left; break;
      case "top-right": case "bottom-right": case "right-center":
        col = margin.left + availW - w; break;
      default: // center, top-center, bottom-center
        col = margin.left + Math.floor((availW - w) / 2);
    }
  }

  row += opt.offsetY ?? 0;
  col += opt.offsetX ?? 0;
  row = Math.max(0, Math.min(row, termHeight - 1));
  col = Math.max(0, Math.min(col, termWidth - 1));

  return { row, col, width: w, maxHeight: maxH };
}

export interface OverlayHandle {
  hide(): void;
  setHidden(hidden: boolean): void;
  isHidden(): boolean;
  focus(): void;
  unfocus(): void;
  isFocused(): boolean;
}
