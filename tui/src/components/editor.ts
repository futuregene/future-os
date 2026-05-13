/**
 * Editor component — multi-line text input with Emacs-style keybindings.
 * Ported from pi's editor.ts. Implements Component + Focusable.
 */

import { CSI, RESET, CURSOR_MARKER } from "../tui.js";
import type { Component, Focusable } from "../tui.js";
import { visibleWidth, stripAnsiCodes } from "../utils.js";

// ─── Types ─────────────────────────────────────────────────────────────────

export interface EditorTheme {
  prompt: number;
  text: number;
  cursor: number;
  cursorText: number;
  bg: number;
  border?: number;
}

export interface EditorCallbacks {
  onSubmit?: (value: string) => void;
  onChange?: (value: string) => void;
}

interface EditorState {
  value: string;
  cursorPos: number;
}

interface TextChunk {
  text: string;
  width: number;
  isWrap: boolean; // true if this chunk is a wrap continuation (no prefix needed)
}

// ─── Undo Stack ────────────────────────────────────────────────────────────

class UndoStack<T> {
  private stack: T[] = [];
  private index = -1;
  private maxSize: number;

  constructor(maxSize = 100) {
    this.maxSize = maxSize;
  }

  push(state: T): void {
    this.stack = this.stack.slice(0, this.index + 1);
    this.stack.push(state);
    if (this.stack.length > this.maxSize) this.stack.shift();
    this.index = this.stack.length - 1;
  }

  undo(): T | null {
    if (this.index <= 0) return null;
    this.index--;
    return this.stack[this.index] ?? null;
  }

  redo(): T | null {
    if (this.index >= this.stack.length - 1) return null;
    this.index++;
    return this.stack[this.index] ?? null;
  }

  clear(): void {
    this.stack = [];
    this.index = -1;
  }
}

// ─── Kill Ring ─────────────────────────────────────────────────────────────

class KillRing {
  private entries: string[] = [];
  private index = -1;
  private accumulating = false;
  private maxEntries: number;

  constructor(maxEntries = 60) {
    this.maxEntries = maxEntries;
  }

  kill(text: string, accumulate: boolean): void {
    if (text.length === 0) return;
    if (accumulate && this.accumulating && this.entries.length > 0) {
      const last = this.entries[0];
      if (last) {
        this.entries[0] = last + text;
      }
    } else {
      this.entries.unshift(text);
      if (this.entries.length > this.maxEntries) this.entries.pop();
    }
    this.accumulating = accumulate;
    this.index = -1;
  }

  yank(): string {
    this.index = 0;
    return this.entries[0] ?? "";
  }

  yankPop(): string {
    if (this.entries.length === 0) return "";
    this.index = (this.index + 1) % this.entries.length;
    return this.entries[this.index] ?? "";
  }

  resetYank(): void {
    this.index = -1;
  }

  breakAccumulate(): void {
    this.accumulating = false;
  }
}

// ─── Word Break Helpers ────────────────────────────────────────────────────

function isWordChar(c: string): boolean {
  const code = c.codePointAt(0);
  if (code === undefined) return false;
  return (code >= 48 && code <= 57) || // digits
    (code >= 65 && code <= 90) || // A-Z
    (code >= 97 && code <= 122) || // a-z
    code === 95 || code === 45; // _ -
}

function findWordLeft(value: string, pos: number): number {
  if (pos <= 0) return 0;
  // Skip non-word chars
  let i = pos - 1;
  while (i > 0 && !isWordChar(value[i] ?? "")) i--;
  // Skip word chars
  while (i > 0 && isWordChar(value[i - 1] ?? "")) i--;
  return i;
}

function findWordRight(value: string, pos: number): number {
  const len = value.length;
  if (pos >= len) return len;
  let i = pos;
  // Skip current word chars
  while (i < len && isWordChar(value[i] ?? "")) i++;
  // Skip non-word chars
  while (i < len && !isWordChar(value[i] ?? "")) i++;
  return i;
}

// ─── Paste Markers ─────────────────────────────────────────────────────────

const PASTE_MARKER_START = "\x1b_pi:ps\x07";
const PASTE_MARKER_END = "\x1b_pi:pe\x07";

function escapeRegex(s: string): string {
  return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

/** Split text into segments, keeping paste markers as atomic units. */
function segmentWithMarkers(text: string): { text: string; isMarker: boolean }[] {
  const segments: { text: string; isMarker: boolean }[] = [];
  const markerRegex = new RegExp(
    `${escapeRegex(PASTE_MARKER_START)}paste_\\d+${escapeRegex(PASTE_MARKER_END)}\\[Paste: [^\\]]+\\]`,
    "g"
  );
  let lastIdx = 0;
  for (const match of text.matchAll(markerRegex)) {
    if (match.index !== undefined && match.index > lastIdx) {
      segments.push({ text: text.slice(lastIdx, match.index), isMarker: false });
    }
    segments.push({ text: match[0], isMarker: true });
    lastIdx = (match.index ?? 0) + match[0].length;
  }
  if (lastIdx < text.length) {
    segments.push({ text: text.slice(lastIdx), isMarker: false });
  }
  return segments.length > 0 ? segments : [{ text: "", isMarker: false }];
}

// ─── Editor ────────────────────────────────────────────────────────────────

export class Editor implements Component, Focusable {
  private value = "";
  private cursorPos = 0;
  private theme: EditorTheme;
  private callbacks: EditorCallbacks;
  focused = true;
  private history: string[] = [];
  private historyIndex = -1;
  private prefix = "❯ ";
  private borderColor?: number;
  private paddingX = 1;

  // Multi-line state
  private visualLines: { chunks: TextChunk[]; startOffset: number; endOffset: number }[] = [];
  private viewportTop = 0;
  private maxVisualLines = 8; // max visual lines before scroll

  // Undo / Kill Ring
  private undoStack = new UndoStack<EditorState>(100);
  private killRing = new KillRing();
  private lastKillWasAccumulate = false;

  // Paste markers: map of paste-id → full content
  private pasteMarkerId = 0;
  private pasteStorage = new Map<string, string>();
  private pasteMarkersInValue = new Set<string>(); // paste-ids present in current value

  constructor(prefix = "❯ ", theme?: Partial<EditorTheme>, callbacks?: EditorCallbacks) {
    this.prefix = prefix;
    this.theme = {
      prompt: 39,
      text: 252,
      cursor: 39,
      cursorText: 17,
      bg: 235,
      border: 240,
      ...theme,
    };
    this.borderColor = this.theme.border;
    this.callbacks = callbacks ?? {};
  }

  // ─── Public API ────────────────────────────────────────────────────────────

  setValue(value: string): void {
    this.saveUndo();
    this.value = value;
    this.cursorPos = value.length;
    this.historyIndex = -1;
    this.killRing.breakAccumulate();
    this.pasteStorage.clear();
    this.pasteMarkersInValue.clear();
    this.callbacks.onChange?.(value);
  }

  getValue(): string {
    return this.value;
  }

  insertText(text: string): void {
    this.saveUndo();
    // Fold large pastes into collapse markers to avoid rendering issues
    const lineCount = (text.match(/\n/g) || []).length + 1;
    if (lineCount > 10 || text.length > 1000) {
      const id = `paste_${++this.pasteMarkerId}`;
      this.pasteStorage.set(id, text);
      this.pasteMarkersInValue.add(id);
      const marker = `${PASTE_MARKER_START}${id}${PASTE_MARKER_END}[Paste: ${lineCount} lines, ${text.length} chars]`;
      this.insertAtCursor(marker);
    } else {
      this.insertAtCursor(text);
    }
    this.killRing.breakAccumulate();
    this.callbacks.onChange?.(this.value);
  }

  /** Get value with paste markers expanded to original content. */
  getValueExpanded(): string {
    let result = this.value;
    // Expand paste markers: \x1b_pi:ps\x07<id>\x1b_pi:pe\x07 → stored content
    const markerRegex = new RegExp(`${escapeRegex(PASTE_MARKER_START)}(paste_\\d+)${escapeRegex(PASTE_MARKER_END)}\\[Paste: [^\\]]+\\]`, "g");
    result = result.replace(markerRegex, (_match, id: string) => {
      return this.pasteStorage.get(id) ?? _match;
    });
    return result;
  }

  setPrefix(prefix: string): void {
    this.prefix = prefix;
  }

  setBorderColor(color: number | undefined): void {
    this.borderColor = color;
  }

  setPaddingX(px: number): void {
    this.paddingX = Math.max(0, px);
  }

  handleInput(data: string): void {
    this.handleKey(data);
  }

  invalidate(): void {
    this.visualLines = [];
  }

  // ─── Text Manipulation ────────────────────────────────────────────────────

  private insertAtCursor(text: string): void {
    this.value = this.value.slice(0, this.cursorPos) + text + this.value.slice(this.cursorPos);
    this.cursorPos += text.length;
  }

  private deleteRange(start: number, end: number): string {
    const deleted = this.value.slice(start, end);
    this.value = this.value.slice(0, start) + this.value.slice(end);
    return deleted;
  }

  private saveUndo(): void {
    this.undoStack.push({ value: this.value, cursorPos: this.cursorPos });
  }

  // ─── Key Handling ─────────────────────────────────────────────────────────

  handleKey(key: string): boolean {
    switch (key) {
      // Movement
      case "left": return this.moveLeft();
      case "right": return this.moveRight();
      case "up": return this.moveUp();
      case "down": return this.moveDown();
      case "home": return this.moveHome();
      case "end": return this.moveEnd();
      case "pageUp": return this.movePageUp();
      case "pageDown": return this.movePageDown();

      // Word jump
      case "ctrl+left":
      case "alt+b": return this.moveWordLeft();
      case "ctrl+right":
      case "alt+f": return this.moveWordRight();

      // Deletion
      case "backspace": return this.deleteBackward();
      case "delete": return this.deleteForward();
      case "ctrl+h": return this.deleteBackward();

      // Kill / Yank
      case "ctrl+k": return this.killToEndOfLine();
      case "alt+d": return this.killWordForward();
      case "alt+backspace":
      case "ctrl+w": return this.killWordBackward();
      case "ctrl+y": return this.yank();
      case "alt+y": return this.yankPop();

      // Undo
      case "ctrl+/":
      case "ctrl+z":
      case "ctrl+_": return this.undo();
      case "alt+/":
      case "ctrl+shift+z": return this.redo();

      // Navigation shortcuts
      case "ctrl+a": return this.moveHome();
      case "ctrl+e": return this.moveEnd();
      case "ctrl+u": return this.killToStartOfLine();

      // Submit / Newline
      case "enter": return this.submit();
      case "shift+enter": return this.insertNewline();

      // Tab
      case "tab": return true; // handled by App

      default:
        // Printable character
        if (key.length === 1) {
          const code = key.charCodeAt(0);
          if (code >= 32) {
            return this.insertChar(key);
          }
        }
        return false;
    }
  }

  // ─── Movement ─────────────────────────────────────────────────────────────

  private moveLeft(): boolean {
    if (this.cursorPos > 0) { this.cursorPos--; return true; }
    return false;
  }
  private moveRight(): boolean {
    if (this.cursorPos < this.value.length) { this.cursorPos++; return true; }
    return false;
  }
  private moveHome(): boolean {
    this.cursorPos = 0; return true;
  }
  private moveEnd(): boolean {
    this.cursorPos = this.value.length; return true;
  }

  private moveUp(): boolean {
    // If cursor is on first logical line, navigate history (only when empty)
    if (this.value.length === 0 || this.historyIndex >= 0) {
      return this.historyUp();
    }
    // Move up one visual line
    const visIdx = this.findVisualLine(this.cursorPos);
    if (visIdx <= 0) {
      // At first visual line, go to start of line
      this.cursorPos = 0;
      return true;
    }
    const prevLine = this.visualLines[visIdx - 1];
    if (!prevLine) return false;
    const colInLine = this.cursorPos - (this.visualLines[visIdx]?.startOffset ?? 0);
    this.cursorPos = Math.min(prevLine.endOffset, prevLine.startOffset + colInLine);
    return true;
  }

  private moveDown(): boolean {
    if (this.historyIndex > 0) {
      return this.historyDown();
    }
    const visIdx = this.findVisualLine(this.cursorPos);
    const lastIdx = this.visualLines.length - 1;
    if (visIdx >= lastIdx) {
      // At or below last visual line, go to end
      this.cursorPos = this.value.length;
      return true;
    }
    const nextLine = this.visualLines[visIdx + 1];
    if (!nextLine) return false;
    const curLine = this.visualLines[visIdx];
    const colInLine = curLine ? this.cursorPos - curLine.startOffset : 0;
    this.cursorPos = Math.min(nextLine.endOffset, nextLine.startOffset + colInLine);
    return true;
  }

  private movePageUp(): boolean {
    const visIdx = this.findVisualLine(this.cursorPos);
    const target = Math.max(0, visIdx - this.maxVisualLines);
    const line = this.visualLines[target];
    if (line) this.cursorPos = line.startOffset;
    else this.cursorPos = 0;
    return true;
  }

  private movePageDown(): boolean {
    const visIdx = this.findVisualLine(this.cursorPos);
    const target = Math.min(this.visualLines.length - 1, visIdx + this.maxVisualLines);
    const line = this.visualLines[target];
    if (line) this.cursorPos = line.endOffset;
    else this.cursorPos = this.value.length;
    return true;
  }

  private moveWordLeft(): boolean {
    this.cursorPos = findWordLeft(this.value, this.cursorPos);
    return true;
  }

  private moveWordRight(): boolean {
    this.cursorPos = findWordRight(this.value, this.cursorPos);
    return true;
  }

  // ─── Editing ──────────────────────────────────────────────────────────────

  private insertChar(char: string): boolean {
    this.saveUndo();
    this.insertAtCursor(char);
    this.callbacks.onChange?.(this.value);
    return true;
  }

  private insertNewline(): boolean {
    this.saveUndo();
    this.insertAtCursor("\n");
    this.callbacks.onChange?.(this.value);
    return true;
  }

  private deleteBackward(): boolean {
    if (this.cursorPos <= 0) return false;
    this.saveUndo();
    this.value = this.value.slice(0, this.cursorPos - 1) + this.value.slice(this.cursorPos);
    this.cursorPos--;
    this.callbacks.onChange?.(this.value);
    return true;
  }

  private deleteForward(): boolean {
    if (this.cursorPos >= this.value.length) return false;
    this.saveUndo();
    this.value = this.value.slice(0, this.cursorPos) + this.value.slice(this.cursorPos + 1);
    this.callbacks.onChange?.(this.value);
    return true;
  }

  // ─── Kill / Yank ──────────────────────────────────────────────────────────

  private killToEndOfLine(): boolean {
    this.killRing.breakAccumulate();
    this.saveUndo();
    const newlineIdx = this.value.indexOf("\n", this.cursorPos);
    const end = newlineIdx === -1 ? this.value.length : newlineIdx;
    if (this.cursorPos === end && this.cursorPos < this.value.length) {
      // Kill the newline itself
      const deleted = this.value.slice(this.cursorPos, end + 1);
      this.value = this.value.slice(0, this.cursorPos) + this.value.slice(end + 1);
      this.killRing.kill(deleted, true);
      this.callbacks.onChange?.(this.value);
      return true;
    }
    const deleted = this.deleteRange(this.cursorPos, end);
    this.killRing.kill(deleted, true);
    this.callbacks.onChange?.(this.value);
    return true;
  }

  private killToStartOfLine(): boolean {
    this.saveUndo();
    const lineStart = this.value.lastIndexOf("\n", this.cursorPos - 1);
    const start = lineStart === -1 ? 0 : lineStart + 1;
    const deleted = this.deleteRange(start, this.cursorPos);
    this.cursorPos = start;
    this.killRing.kill(deleted, false);
    this.callbacks.onChange?.(this.value);
    return true;
  }

  private killWordForward(): boolean {
    this.saveUndo();
    const end = findWordRight(this.value, this.cursorPos);
    const deleted = this.deleteRange(this.cursorPos, end);
    this.killRing.kill(deleted || " ", true);
    this.callbacks.onChange?.(this.value);
    return true;
  }

  private killWordBackward(): boolean {
    this.saveUndo();
    const start = findWordLeft(this.value, this.cursorPos);
    const deleted = this.deleteRange(start, this.cursorPos);
    this.cursorPos = start;
    this.killRing.kill(deleted || " ", true);
    this.callbacks.onChange?.(this.value);
    return true;
  }

  private yank(): boolean {
    const text = this.killRing.yank();
    if (!text) return false;
    this.saveUndo();
    this.insertAtCursor(text);
    this.callbacks.onChange?.(this.value);
    return true;
  }

  private yankPop(): boolean {
    this.saveUndo();
    // Replace last yanked text
    const prevYank = this.killRing.yankPop();
    this.killRing.resetYank();
    this.insertAtCursor(prevYank);
    this.callbacks.onChange?.(this.value);
    return true;
  }

  // ─── Undo / Redo ──────────────────────────────────────────────────────────

  private undo(): boolean {
    const state = this.undoStack.undo();
    if (!state) return false;
    this.value = state.value;
    this.cursorPos = state.cursorPos;
    this.callbacks.onChange?.(this.value);
    return true;
  }

  private redo(): boolean {
    const state = this.undoStack.redo();
    if (!state) return false;
    this.value = state.value;
    this.cursorPos = state.cursorPos;
    this.callbacks.onChange?.(this.value);
    return true;
  }

  // ─── Submit ───────────────────────────────────────────────────────────────

  private submit(): boolean {
    const expandedValue = this.getValueExpanded();
    if (expandedValue.trim()) {
      this.history.unshift(expandedValue);
      if (this.history.length > 100) this.history.pop();
      this.historyIndex = -1;
      this.callbacks.onSubmit?.(expandedValue);
      this.value = "";
      this.cursorPos = 0;
      this.undoStack.clear();
      this.killRing = new KillRing();
      this.pasteStorage.clear();
      this.pasteMarkersInValue.clear();
      this.visualLines = [];
    }
    return true;
  }

  // ─── History ──────────────────────────────────────────────────────────────

  private historyUp(): boolean {
    if (this.historyIndex < this.history.length - 1) {
      if (this.historyIndex === -1) {
        // Save current value before navigating
        this.history.push(this.value);
      }
      this.historyIndex++;
      const h = this.history[this.historyIndex];
      if (h !== undefined) {
        this.value = h;
        this.cursorPos = h.length;
      }
      this.callbacks.onChange?.(this.value);
    }
    return true;
  }

  private historyDown(): boolean {
    if (this.historyIndex > 0) {
      this.historyIndex--;
      const h = this.history[this.historyIndex];
      if (h !== undefined) {
        this.value = h;
        this.cursorPos = h.length;
      }
      this.callbacks.onChange?.(this.value);
    } else if (this.historyIndex === 0) {
      this.historyIndex = -1;
      this.value = "";
      this.cursorPos = 0;
      this.callbacks.onChange?.(this.value);
    }
    return true;
  }

  // ─── Visual Line Mapping ──────────────────────────────────────────────────

  private findVisualLine(offset: number): number {
    for (let i = 0; i < this.visualLines.length; i++) {
      const vl = this.visualLines[i];
      if (!vl) continue;
      if (offset >= vl.startOffset && offset <= vl.endOffset) return i;
    }
    // If beyond last line, return last
    return Math.max(0, this.visualLines.length - 1);
  }

  private rebuildVisualLines(availableWidth: number): void {
    this.visualLines = [];
    if (availableWidth <= 0) return;

    const prefixWidth = visibleWidth(this.prefix);
    const borderPad = this.borderColor !== undefined ? 1 : 0;
    const firstLineWidth = availableWidth - prefixWidth - this.paddingX - borderPad;
    const continuationWidth = availableWidth - this.paddingX - borderPad;

    // Split by logical newlines first, then segment each line by paste markers
    const logicalLines = this.value.split("\n");
    const allSegments: { text: string; isMarker: boolean; newlineAfter: boolean }[] = [];
    for (let li = 0; li < logicalLines.length; li++) {
      const segs = segmentWithMarkers(logicalLines[li] ?? "");
      for (let si = 0; si < segs.length; si++) {
        const seg = segs[si];
        allSegments.push({
          text: seg?.text ?? "",
          isMarker: seg?.isMarker ?? false,
          newlineAfter: si === segs.length - 1 && li < logicalLines.length - 1,
        });
      }
    }

    let offset = 0;
    let firstVisualLine = true;
    let currentLineChunks: TextChunk[] = [];
    let currentLineStart = 0;
    let currentLineWidth = 0;
    const lineWidth = firstVisualLine ? firstLineWidth : continuationWidth;

    const self = this;
    function flushVisualLine(endOffset: number): void {
      if (currentLineChunks.length === 0) {
        currentLineChunks = [{ text: "", width: 0, isWrap: !firstVisualLine }];
      }
      self.visualLines.push({
        chunks: currentLineChunks,
        startOffset: currentLineStart,
        endOffset,
      });
      currentLineChunks = [];
      currentLineStart = endOffset;
      currentLineWidth = 0;
      firstVisualLine = false;
    }

    const flushVL = (eo: number) => flushVisualLine(eo);
    const contW = continuationWidth;

    for (const seg of allSegments) {
      const segWidth = seg.isMarker ? 20 : visibleWidth(seg.text); // approximate marker width
      const effectiveWidth = firstVisualLine ? firstLineWidth : contW;

      if (seg.text === "" && !seg.newlineAfter) {
        // Empty segment within a line
        offset += seg.text.length;
        if (seg.newlineAfter) offset++;
        continue;
      }

      if (seg.isMarker) {
        // Paste markers are atomic — don't split. If it doesn't fit, start new line.
        if (currentLineChunks.length > 0 && currentLineWidth + segWidth > effectiveWidth) {
          flushVL(offset);
        }
        currentLineChunks.push({ text: seg.text, width: segWidth, isWrap: !firstVisualLine });
        currentLineWidth += segWidth;
        offset += seg.text.length;
      } else if (seg.text.includes("\n") || seg.newlineAfter) {
        // Handle text up to logical newline
        let remaining = seg.text;
        while (remaining.length > 0) {
          const availW = (firstVisualLine ? firstLineWidth : contW) - currentLineWidth;
          if (availW <= 0) {
            flushVL(offset);
            continue;
          }
          const slice = sliceWithWidth(remaining, availW);
          const chunkText = slice.text || remaining.slice(0, 1);
          const chunkWidth = slice.width || 1;
          currentLineChunks.push({ text: chunkText, width: chunkWidth, isWrap: !firstVisualLine });
          currentLineWidth += chunkWidth;
          offset += chunkText.length;
          remaining = remaining.slice(chunkText.length);

          if (remaining.length > 0) {
            flushVL(offset);
          }
        }
        // Handle logical newline
        if (seg.newlineAfter) {
          flushVL(offset + 1); // +1 for newline char
          offset++;
          firstVisualLine = true; // reset for next logical line
        }
      } else {
        // Plain text segment — word-wrap
        let remaining = seg.text;
        while (remaining.length > 0) {
          const availW = (firstVisualLine ? firstLineWidth : contW) - currentLineWidth;
          if (availW <= 0) {
            flushVL(offset);
            continue;
          }
          const slice = sliceWithWidth(remaining, availW);
          const chunkText = slice.text || remaining.slice(0, 1);
          const chunkWidth = slice.width || 1;
          currentLineChunks.push({ text: chunkText, width: chunkWidth, isWrap: !firstVisualLine });
          currentLineWidth += chunkWidth;
          offset += chunkText.length;
          remaining = remaining.slice(chunkText.length);

          if (remaining.length > 0) {
            flushVL(offset);
          }
        }
      }
    }

    // Flush remaining
    if (currentLineChunks.length > 0 || this.visualLines.length === 0) {
      flushVL(offset);
    }
  }

  // ─── Render ───────────────────────────────────────────────────────────────

  render(screenWidth: number): string[] {
    const lines: string[] = [];
    const borderChar = this.borderColor !== undefined
      ? `${CSI}38;5;${this.borderColor}m│${RESET}`
      : "";
    const padStr = " ".repeat(this.paddingX);
    const borderPad = this.borderColor !== undefined ? 1 : 0;

    const prefixStr = `${CSI}38;5;${this.theme.prompt}m${this.prefix}${RESET}`;
    const prefixWidth = visibleWidth(this.prefix);
    const firstLineTextWidth = screenWidth - prefixWidth - this.paddingX - borderPad;
    const contTextWidth = screenWidth - this.paddingX - borderPad;

    if (firstLineTextWidth <= 0) return [""];

    // Rebuild visual lines
    this.rebuildVisualLines(screenWidth);

    if (this.visualLines.length === 0) {
      this.visualLines.push({ chunks: [{ text: "", width: 0, isWrap: false }], startOffset: 0, endOffset: 0 });
    }

    const totalVL = this.visualLines.length;

    // Adjust viewport to keep cursor visible
    const cursorVL = this.findVisualLine(this.cursorPos);
    const maxVisible = this.maxVisualLines;
    if (cursorVL < this.viewportTop) {
      this.viewportTop = cursorVL;
    } else if (cursorVL >= this.viewportTop + maxVisible) {
      this.viewportTop = cursorVL - maxVisible + 1;
    }
    this.viewportTop = Math.max(0, Math.min(this.viewportTop, Math.max(0, totalVL - maxVisible)));

    const visibleEnd = Math.min(totalVL, this.viewportTop + maxVisible);

    // Scroll indicators
    if (this.viewportTop > 0) {
      const indicator = `${CSI}38;5;${this.theme.border ?? 240}m↑ ${this.viewportTop} more${RESET}`;
      lines.push(borderChar + indicator);
    }

    for (let vi = this.viewportTop; vi < visibleEnd; vi++) {
      const vl = this.visualLines[vi];
      if (!vl) continue;

      const isFirst = vi === 0;
      const linePrefix = isFirst ? prefixStr : " ".repeat(prefixWidth);
      const textWidth = isFirst ? firstLineTextWidth : contTextWidth;

      // Build line content
      let content = "";
      let contentWidth = 0;
      for (const chunk of vl.chunks) {
        content += chunk.text;
        contentWidth += chunk.width;
      }

      // Pad to fill width
      const pad = Math.max(0, textWidth - contentWidth);

      let display = linePrefix + padStr;
      display += `${CSI}38;5;${this.theme.text}m${content}${RESET}`;
      display += " ".repeat(pad);

      // Render cursor in this visual line
      if (this.focused && this.cursorPos >= vl.startOffset && this.cursorPos <= vl.endOffset) {
        const posInLine = this.cursorPos - vl.startOffset;
        const beforeCursor = content.slice(0, posInLine);
        const atCursor = content[posInLine] || " ";
        const afterCursor = content.slice(posInLine + 1);

        const beforeWidth = visibleWidth(beforeCursor);
        const cursorWidth = visibleWidth(atCursor);

        display = linePrefix + padStr;
        display += `${CSI}38;5;${this.theme.text}m${beforeCursor}${RESET}`;
        display += CURSOR_MARKER;
        display += `${CSI}38;5;${this.theme.cursorText}m${CSI}48;5;${this.theme.cursor}m${atCursor}${RESET}`;
        display += `${CSI}38;5;${this.theme.text}m${afterCursor}${RESET}`;

        const afterWidth = visibleWidth(afterCursor);
        const remPad = Math.max(0, textWidth - beforeWidth - cursorWidth - afterWidth);
        display += " ".repeat(remPad);
      } else if (!this.focused) {
        display = linePrefix + padStr;
        display += `${CSI}38;5;245m${content}${RESET}`;
        display += " ".repeat(pad);
      }

      if (this.borderColor !== undefined) {
        display = borderChar + display + borderChar;
      }

      lines.push(display);
    }

    if (visibleEnd < totalVL) {
      const remaining = totalVL - visibleEnd;
      const indicator = `${CSI}38;5;${this.theme.border ?? 240}m↓ ${remaining} more${RESET}`;
      lines.push(borderChar + indicator);
    }

    return lines;
  }
}

// ─── Slice With Width Helper ───────────────────────────────────────────────

function sliceWithWidth(s: string, maxWidth: number): { text: string; width: number } {
  if (maxWidth <= 0) return { text: "", width: 0 };
  const segmenter = new Intl.Segmenter("en", { granularity: "grapheme" });
  let result = "";
  let width = 0;
  for (const { segment } of segmenter.segment(s)) {
    const w = graphemeWidthForSlice(segment);
    if (width + w > maxWidth) break;
    result += segment;
    width += w;
  }
  return { text: result, width };
}

function graphemeWidthForSlice(g: string): number {
  const code = g.codePointAt(0);
  if (code === undefined) return 0;
  // Zero-width
  if (
    code === 0x200B || code === 0x200C || code === 0x200D || code === 0xFEFF ||
    (code >= 0x0300 && code <= 0x036F) || (code >= 0x20D0 && code <= 0x20FF)
  ) return 0;
  // CJK
  if (
    (code >= 0x1100 && code <= 0x115F) ||
    (code >= 0x2E80 && code <= 0xA4CF) ||
    (code >= 0xAC00 && code <= 0xD7A3) ||
    (code >= 0xF900 && code <= 0xFAFF) ||
    (code >= 0xFE30 && code <= 0xFE6F) ||
    (code >= 0xFF01 && code <= 0xFF60) ||
    (code >= 0xFFE0 && code <= 0xFFE6) ||
    (code >= 0x1F300 && code <= 0x1F9FF) ||
    (code >= 0x20000 && code <= 0x2FFFF)
  ) return 2;
  return 1;
}
