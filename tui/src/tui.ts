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
