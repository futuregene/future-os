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
export function cursorPos(row, col) {
    return `${CSI}${row};${col}H`;
}
export function setFg(c) {
    return `${CSI}38;5;${c}m`;
}
export function setBg(c) {
    return `${CSI}48;5;${c}m`;
}
import { StdinBuffer } from "./stdin-buffer.js";
import { setKittyProtocolActive } from "./keys.js";
import * as fs from "node:fs";
import * as path from "node:path";
import * as os from "node:os";
const TERMINAL_PROGRESS_KEEPALIVE_MS = 1000;
const TERMINAL_PROGRESS_ACTIVE_SEQUENCE = "\x1b]9;4;3\x07";
const TERMINAL_PROGRESS_CLEAR_SEQUENCE = "\x1b]9;4;0;\x07";
export class NodeTerminal {
    wasRaw = false;
    inputHandler;
    resizeHandler;
    _kittyProtocolActive = false;
    _modifyOtherKeysActive = false;
    stdinBuffer;
    stdinDataHandler;
    progressInterval;
    get kittyProtocolActive() {
        return this._kittyProtocolActive;
    }
    start(onInput, onResize) {
        this.inputHandler = onInput;
        this.resizeHandler = onResize;
        this.wasRaw = process.stdin.isRaw || false;
        if (process.stdin.setRawMode) {
            process.stdin.setRawMode(true);
        }
        process.stdin.setEncoding("utf8");
        process.stdin.resume();
        // Enable bracketed paste mode + SGR mouse tracking
        process.stdout.write("\x1b[?2004h");
        process.stdout.write(MOUSE_TRACK_ON);
        // Set up resize handler
        process.stdout.on("resize", this.resizeHandler);
        // Refresh terminal dimensions after suspend/resume
        if (process.platform !== "win32") {
            process.kill(process.pid, "SIGWINCH");
        }
        // Query and enable Kitty keyboard protocol
        this.queryAndEnableKittyProtocol();
    }
    setupStdinBuffer() {
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
        this.stdinDataHandler = (data) => {
            this.stdinBuffer.process(data);
        };
    }
    queryAndEnableKittyProtocol() {
        this.setupStdinBuffer();
        process.stdin.on("data", this.stdinDataHandler);
        process.stdout.write("\x1b[?u");
        setTimeout(() => {
            if (!this._kittyProtocolActive && !this._modifyOtherKeysActive) {
                process.stdout.write("\x1b[>4;2m");
                this._modifyOtherKeysActive = true;
            }
        }, 150);
    }
    async drainInput(maxMs = 1000, idleMs = 50) {
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
                if (timeLeft <= 0)
                    break;
                if (now - lastDataTime >= idleMs)
                    break;
                await new Promise((resolve) => setTimeout(resolve, Math.min(idleMs, timeLeft)));
            }
        }
        finally {
            process.stdin.removeListener("data", onData);
            this._kittyProtocolActive = prevKittyActive;
            this.inputHandler = previousHandler;
        }
    }
    stop() {
        if (this.clearProgressInterval()) {
            process.stdout.write(TERMINAL_PROGRESS_CLEAR_SEQUENCE);
        }
        // Disable bracketed paste mode + mouse tracking
        process.stdout.write("\x1b[?2004l");
        process.stdout.write(MOUSE_TRACK_OFF);
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
        process.stdin.pause();
        if (process.stdin.setRawMode) {
            process.stdin.setRawMode(this.wasRaw);
        }
    }
    write(data) {
        if (process.env.PI_TUI_WRITE_LOG === "1") {
            const logDir = path.join(os.homedir(), ".xihu_tui");
            const logPath = path.join(logDir, "write.log");
            try {
                fs.mkdirSync(logDir, { recursive: true });
            }
            catch { }
            try {
                fs.appendFileSync(logPath, data, { encoding: "utf8" });
            }
            catch { }
        }
        process.stdout.write(data);
    }
    get columns() {
        return process.stdout.columns || Number(process.env.COLUMNS) || 80;
    }
    get rows() {
        return process.stdout.rows || Number(process.env.LINES) || 24;
    }
    moveBy(lines) {
        if (lines > 0) {
            process.stdout.write(`\x1b[${lines}B`);
        }
        else if (lines < 0) {
            process.stdout.write(`\x1b[${-lines}A`);
        }
    }
    hideCursor() { process.stdout.write("\x1b[?25l"); }
    showCursor() { process.stdout.write("\x1b[?25h"); }
    clearLine() {
        process.stdout.write("\x1b[K");
    }
    clearFromCursor() {
        process.stdout.write("\x1b[J");
    }
    clearScreen() {
        process.stdout.write("\x1b[2J\x1b[H");
    }
    setTitle(title) {
        process.stdout.write(`\x1b]0;${title}\x07`);
    }
    setProgress(active) {
        if (active) {
            process.stdout.write(TERMINAL_PROGRESS_ACTIVE_SEQUENCE);
            if (!this.progressInterval) {
                this.progressInterval = setInterval(() => {
                    process.stdout.write(TERMINAL_PROGRESS_ACTIVE_SEQUENCE);
                }, TERMINAL_PROGRESS_KEEPALIVE_MS);
            }
        }
        else {
            this.clearProgressInterval();
            process.stdout.write(TERMINAL_PROGRESS_CLEAR_SEQUENCE);
        }
    }
    clearProgressInterval() {
        if (!this.progressInterval)
            return false;
        clearInterval(this.progressInterval);
        this.progressInterval = undefined;
        return true;
    }
}
export const DEFAULT_THEME = {
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
export function isFocusable(component) {
    return component !== null && "focused" in component;
}
export const CURSOR_MARKER = "\x1b_pi:c\x07";
export class Container {
    children = [];
    addChild(component) {
        this.children.push(component);
    }
    removeChild(component) {
        const index = this.children.indexOf(component);
        if (index !== -1)
            this.children.splice(index, 1);
    }
    clear() {
        this.children = [];
    }
    invalidate() {
        for (const child of this.children)
            child.invalidate();
    }
    render(width) {
        const lines = [];
        for (const child of this.children) {
            for (const line of child.render(width))
                lines.push(line);
        }
        return lines;
    }
}
export function parseSizeValue(v, total) {
    if (typeof v === "number")
        return v;
    if (v.endsWith("%")) {
        const pct = parseFloat(v);
        if (isNaN(pct))
            return 0;
        return Math.floor((pct / 100) * total);
    }
    return 0;
}
export function resolveOverlayLayout(termWidth, termHeight, componentLines, options) {
    const opt = options ?? {};
    const marginNum = typeof opt.margin === "number" ? opt.margin : 0;
    const margin = typeof opt.margin === "object"
        ? { top: opt.margin.top ?? marginNum, right: opt.margin.right ?? marginNum,
            bottom: opt.margin.bottom ?? marginNum, left: opt.margin.left ?? marginNum }
        : { top: marginNum, right: marginNum, bottom: marginNum, left: marginNum };
    const availW = termWidth - margin.left - margin.right;
    const availH = termHeight - margin.top - margin.bottom;
    let w = opt.width !== undefined ? parseSizeValue(opt.width, termWidth) : Math.min(80, availW);
    if (opt.minWidth !== undefined)
        w = Math.max(w, opt.minWidth);
    w = Math.max(1, Math.min(w, availW));
    let maxH = opt.maxHeight !== undefined ? parseSizeValue(opt.maxHeight, termHeight) : termHeight;
    maxH = Math.min(maxH, availH);
    const effectiveH = Math.min(componentLines, maxH);
    const anchor = opt.anchor ?? "center";
    let row;
    let col;
    // Explicit row/col override anchor
    if (opt.row !== undefined) {
        row = parseSizeValue(opt.row, availH);
    }
    else {
        switch (anchor) {
            case "top-left":
            case "top-right":
            case "top-center":
                row = margin.top;
                break;
            case "bottom-left":
            case "bottom-right":
            case "bottom-center":
                row = margin.top + availH - effectiveH;
                break;
            default: // center, left-center, right-center
                row = margin.top + Math.floor((availH - effectiveH) / 2);
        }
    }
    if (opt.col !== undefined) {
        col = parseSizeValue(opt.col, availW);
    }
    else {
        switch (anchor) {
            case "top-left":
            case "bottom-left":
            case "left-center":
                col = margin.left;
                break;
            case "top-right":
            case "bottom-right":
            case "right-center":
                col = margin.left + availW - w;
                break;
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
