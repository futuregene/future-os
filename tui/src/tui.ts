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
  start(onInput: (data: string) => void, onResize: () => void): void;
  stop(): void;
  drainInput(maxMs?: number, idleMs?: number): Promise<void>;
  write(data: string): void;
  get columns(): number;
  get rows(): number;
  get kittyProtocolActive(): boolean;
  moveBy(lines: number): void;
  hideCursor(): void;
  showCursor(): void;
  clearLine(): void;
  clearFromCursor(): void;
  clearScreen(): void;
  setTitle(title: string): void;
  setProgress(active: boolean): void;
}

import { StdinBuffer } from "./stdin-buffer.js";
import { setKittyProtocolActive } from "./keys.js";
import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";

const TERMINAL_PROGRESS_KEEPALIVE_MS = 1000;
const TERMINAL_PROGRESS_ACTIVE_SEQUENCE = "\x1b]9;4;3\x07";
const TERMINAL_PROGRESS_CLEAR_SEQUENCE = "\x1b]9;4;0;\x07";

export class NodeTerminal implements Terminal {
  private wasRaw = false;
  private inputHandler?: (data: string) => void;
  private resizeHandler?: () => void;
  private _kittyProtocolActive = false;
  private _modifyOtherKeysActive = false;
  private stdinBuffer?: StdinBuffer;
  private stdinDataHandler?: (data: string) => void;
  private progressInterval?: ReturnType<typeof setInterval>;
  private exitHandler?: () => void;
  private stopped = false;

  get kittyProtocolActive(): boolean {
    return this._kittyProtocolActive;
  }

  start(onInput: (data: string) => void, onResize: () => void): void {
    this.inputHandler = onInput;
    this.resizeHandler = onResize;
    this.stopped = false;

    this.wasRaw = process.stdin.isRaw || false;
    if (process.stdin.setRawMode) {
      process.stdin.setRawMode(true);
    }
    process.stdin.setEncoding("utf8");
    process.stdin.resume();

    // Alternate screen buffer — isolates TUI from terminal scrollback
    process.stdout.write("\x1b[?1049h");

    // Enable bracketed paste mode
    process.stdout.write("\x1b[?2004h");

    // Failsafe: restore terminal on unexpected exit (crash, uncaught exception, etc.)
    // Must be synchronous — process.exit event doesn't support async.
    this.exitHandler = () => {
      if (this.stopped) return;
      // Show cursor (may be hidden by TUI)
      process.stdout.write("\x1b[?25h");
      // Disable bracketed paste mode
      process.stdout.write("\x1b[?2004l");
      // Disable Kitty keyboard protocol
      if (this._kittyProtocolActive) {
        process.stdout.write("\x1b[<u");
        this._kittyProtocolActive = false;
      }
      // Disable modifyOtherKeys fallback
      if (this._modifyOtherKeysActive) {
        process.stdout.write("\x1b[>4;0m");
        this._modifyOtherKeysActive = false;
      }
      // Clear progress indicator
      if (this.progressInterval) {
        clearInterval(this.progressInterval);
        this.progressInterval = undefined;
        process.stdout.write(TERMINAL_PROGRESS_CLEAR_SEQUENCE);
      }
      // Restore raw mode
      if (process.stdin.setRawMode) process.stdin.setRawMode(this.wasRaw);
      // Newline so shell prompt starts clean
      process.stdout.write("\r\n");
    };
    process.on("exit", this.exitHandler);

    // Set up resize handler
    process.stdout.on("resize", this.resizeHandler);

    // Refresh terminal dimensions after suspend/resume
    if (process.platform !== "win32") {
      process.kill(process.pid, "SIGWINCH");
    }

    // Query and enable Kitty keyboard protocol
    this.queryAndEnableKittyProtocol();
  }

  private setupStdinBuffer(): void {
    this.stdinBuffer = new StdinBuffer({ timeout: 10 });

    const kittyResponsePattern = /^\x1b\[\?(\d+)u$/;

    this.stdinBuffer.on("data", (sequence) => {
      // Check for Kitty protocol response
      if (!this._kittyProtocolActive) {
        const match = sequence.match(kittyResponsePattern);
        if (match) {
          this._kittyProtocolActive = true;
          setKittyProtocolActive(true);
          process.stdout.write("\x1b[>7u");
          return;
        }
      }

      if (this.inputHandler) {
        this.inputHandler(sequence);
      }
    });

    // Re-wrap paste content with bracketed paste markers
    this.stdinBuffer.on("paste", (content) => {
      if (this.inputHandler) {
        this.inputHandler(`\x1b[200~${content}\x1b[201~`);
      }
    });

    this.stdinDataHandler = (data: string) => {
      this.stdinBuffer!.process(data);
    };
  }

  private queryAndEnableKittyProtocol(): void {
    this.setupStdinBuffer();
    process.stdin.on("data", this.stdinDataHandler!);
    process.stdout.write("\x1b[?u");
    setTimeout(() => {
      if (!this._kittyProtocolActive && !this._modifyOtherKeysActive) {
        process.stdout.write("\x1b[>4;2m");
        this._modifyOtherKeysActive = true;
      }
    }, 150);
  }

  async drainInput(maxMs = 1000, idleMs = 50): Promise<void> {
    if (this._kittyProtocolActive) {
      process.stdout.write("\x1b[<u");
      this._kittyProtocolActive = false;
      setKittyProtocolActive(false);
    }
    if (this._modifyOtherKeysActive) {
      process.stdout.write("\x1b[>4;0m");
      this._modifyOtherKeysActive = false;
    }

    // Suppress Kitty response detection during drain: the deactivation
    // response \x1b[?u would otherwise be interpreted as an activation
    // response and re-enable the protocol.
    const prevKittyActive = this._kittyProtocolActive;
    this._kittyProtocolActive = true; // sentinel to block re-enable

    const previousHandler = this.inputHandler;
    this.inputHandler = undefined;

    let lastDataTime = Date.now();
    const onData = () => { lastDataTime = Date.now(); };

    process.stdin.on("data", onData);
    const endTime = Date.now() + maxMs;

    try {
      while (true) {
        const now = Date.now();
        const timeLeft = endTime - now;
        if (timeLeft <= 0) break;
        if (now - lastDataTime >= idleMs) break;
        await new Promise((resolve) => setTimeout(resolve, Math.min(idleMs, timeLeft)));
      }
    } finally {
      process.stdin.removeListener("data", onData);
      this._kittyProtocolActive = prevKittyActive;
      this.inputHandler = previousHandler;
    }
  }

  stop(): void {
    this.stopped = true;
    if (this.exitHandler) {
      process.removeListener("exit", this.exitHandler);
      this.exitHandler = undefined;
    }

    if (this.clearProgressInterval()) {
      process.stdout.write(TERMINAL_PROGRESS_CLEAR_SEQUENCE);
    }

    // Disable bracketed paste mode
    process.stdout.write("\x1b[?2004l");

    // Disable Kitty keyboard protocol
    if (this._kittyProtocolActive) {
      process.stdout.write("\x1b[<u");
      this._kittyProtocolActive = false;
      setKittyProtocolActive(false);
    }
    if (this._modifyOtherKeysActive) {
      process.stdout.write("\x1b[>4;0m");
      this._modifyOtherKeysActive = false;
    }

    // Clean up StdinBuffer
    if (this.stdinBuffer) {
      this.stdinBuffer.destroy();
      this.stdinBuffer = undefined;
    }

    if (this.stdinDataHandler) {
      process.stdin.removeListener("data", this.stdinDataHandler);
      this.stdinDataHandler = undefined;
    }
    this.inputHandler = undefined;
    if (this.resizeHandler) {
      process.stdout.removeListener("resize", this.resizeHandler);
      this.resizeHandler = undefined;
    }

    // Exit alternate screen buffer
    process.stdout.write("\x1b[?1049l");

    process.stdin.pause();

    if (process.stdin.setRawMode) {
      process.stdin.setRawMode(this.wasRaw);
    }
  }

  write(data: string): void {
    if (process.env.PI_TUI_WRITE_LOG === "1") {
      const logDir = path.join(os.homedir(), ".future", "tui");
      const logPath = path.join(logDir, "write.log");
      try { fs.mkdirSync(logDir, { recursive: true }); } catch {}
      try { fs.appendFileSync(logPath, data, { encoding: "utf8" }); } catch {}
    }
    process.stdout.write(data);
  }

  get columns(): number {
    return process.stdout.columns || Number(process.env.COLUMNS) || 80;
  }

  get rows(): number {
    return process.stdout.rows || Number(process.env.LINES) || 24;
  }

  moveBy(lines: number): void {
    if (lines > 0) {
      process.stdout.write(`\x1b[${lines}B`);
    } else if (lines < 0) {
      process.stdout.write(`\x1b[${-lines}A`);
    }
  }

  hideCursor(): void { process.stdout.write("\x1b[?25l"); }
  showCursor(): void { process.stdout.write("\x1b[?25h"); }

  clearLine(): void {
    process.stdout.write("\x1b[K");
  }

  clearFromCursor(): void {
    process.stdout.write("\x1b[J");
  }

  clearScreen(): void {
    process.stdout.write("\x1b[2J\x1b[H");
  }

  setTitle(title: string): void {
    process.stdout.write(`\x1b]0;${title}\x07`);
  }

  setProgress(active: boolean): void {
    if (active) {
      process.stdout.write(TERMINAL_PROGRESS_ACTIVE_SEQUENCE);
      if (!this.progressInterval) {
        this.progressInterval = setInterval(() => {
          process.stdout.write(TERMINAL_PROGRESS_ACTIVE_SEQUENCE);
        }, TERMINAL_PROGRESS_KEEPALIVE_MS);
      }
    } else {
      this.clearProgressInterval();
      process.stdout.write(TERMINAL_PROGRESS_CLEAR_SEQUENCE);
    }
  }

  private clearProgressInterval(): boolean {
    if (!this.progressInterval) return false;
    clearInterval(this.progressInterval);
    this.progressInterval = undefined;
    return true;
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
  bg: -1,
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
