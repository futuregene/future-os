/**
 * Core TUI types and utilities.
 */

import * as readline from "node:readline";
import * as fs from "node:fs";
import * as os from "node:os";

export interface Terminal {
  write(data: string): void;
  clear(): void;
  getWidth(): number;
  getHeight(): number;
  hideCursor(): void;
  showCursor(): void;
  enterAlternateScreen(): void;
  exitAlternateScreen(): void;
  close(): void;
}

// ─── ANSI Escape Sequences ─────────────────────────────────────────────────

export const ESC = "\x1b";
export const CSI = ESC + "[";
export const CLEAR_LINE = `${CSI}2K`;
export const CLEAR = `${CSI}2J`;
export const CURSOR_HIDE = `${CSI}?25l`;
export const CURSOR_SHOW = `${CSI}?25h`;
export const ALT_SCREEN_ON = `${CSI}?1049h`;
export const ALT_SCREEN_OFF = `${CSI}?1049l`;
export const RESET = `${CSI}m`;
export const BOLD = `${CSI}1m`;
export const DIM = `${CSI}2m`;

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

export class NodeTerminal implements Terminal {
  private width = 80;
  private height = 24;
  private fd: number;

  constructor() {
    this.fd = fs.openSync("/dev/tty", "rs+");
    this.updateSize();
  }

  write(data: string): void {
    fs.writeSync(this.fd, data);
  }

  clear(): void {
    this.write(CLEAR + cursorPos(1, 1));
  }

  getWidth(): number {
    this.updateSize();
    return this.width;
  }

  getHeight(): number {
    this.updateSize();
    return this.height;
  }

  hideCursor(): void { this.write(CURSOR_HIDE); }
  showCursor(): void { this.write(CURSOR_SHOW); }
  enterAlternateScreen(): void { this.write(ALT_SCREEN_ON); }
  exitAlternateScreen(): void { this.write(ALT_SCREEN_OFF); }
  close(): void { fs.closeSync(this.fd); }

  private updateSize(): void {
    try {
      const winsz = Buffer.alloc(8);
      // @ts-expect-error TIOCGWINSZ
      const result = fs.fcntl?.(this.fd, 21523, winsz); // 0x5413
      if (result >= 0) {
        this.height = winsz.readInt16LE(0) || 24;
        this.width = winsz.readInt16LE(2) || 80;
      }
    } catch {
      // Use defaults
    }
  }
}

// ─── Readline (raw mode) ──────────────────────────────────────────────────

export function enableRawMode(): void {
  // Use Node.js escape sequence to enable raw mode
  process.stdin.setRawMode?.(true);
  process.stdin.resume();
  process.stdin.setEncoding("utf-8");
}

export function disableRawMode(): void {
  process.stdin.setRawMode?.(false);
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

// ─── Mouse ─────────────────────────────────────────────────────────────

export const MOUSE_ON = `${CSI}?1000h${CSI}?1002h${CSI}?1015h`;
export const MOUSE_OFF = `${CSI}?1015l${CSI}?1002l${CSI}?1000l`;

export interface MouseEvent {
  button: number; // 0=left, 1=middle, 2=right, 64=wheel up, 65=wheel down
  x: number;
  y: number;
}

export function parseMouseEvent(seq: string): MouseEvent | null {
  // X10: ESC [ M <btn+32> <x+32> <y+32>
  const m = seq.match(/^\x1b\[M([\s\S])([\s\S])([\s\S])$/);
  if (!m) return null;
  const btn = m[1].charCodeAt(0) - 32;
  const x = m[2].charCodeAt(0) - 32 - 1;
  const y = m[3].charCodeAt(0) - 32 - 1;
  return { button: btn, x, y };
}
