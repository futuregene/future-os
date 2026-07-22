/**
 * Input component — multi-line text input with history.
 * Enter submits, Alt+Enter / Shift+Enter inserts a newline.
 * Up/Down navigates visual lines (soft-wrapped + hard newlines); history at bounds.
 * Paste preserves newlines (multi-line paste).
 * Implements Component + Focusable.
 */

import { type Component, CURSOR_MARKER, type Focusable } from "../tui.js";
import { extractAnsiCode, getSegmenter, isPunctuationChar, isWhitespaceChar, stripAnsiCodes, visibleWidth, wrapTextWithAnsi } from "../utils.js";

const segmenter = getSegmenter();

/** Info about where the cursor sits in the visual (wrapped) layout. */
interface CursorVisualInfo {
  /** Zero-based index of the visual render line that contains the cursor. */
  visualLine: number;
  /** Byte offset of the cursor within the wrapped sub-line text. */
  colInWrapped: number;
  /** The wrapped sub-line text (without prompt prefix). */
  subLineText: string;
}

export class Input implements Component, Focusable {
  private value: string = "";
  private cursor: number = 0;
  public onSubmit?: (value: string) => void;
  public onEscape?: () => void;
  public onChange?: (value: string) => void;

  // Input history — up/down to recall previous submissions
  private history: string[] = [];
  private historyIndex = -1;
  private historyDraft = "";

  focused: boolean = false;

  // Bracketed paste mode buffering
  private pasteBuffer: string = "";
  private isInPaste: boolean = false;

  // ─── Cached visual layout (invalidated on edit / size change) ─────
  private cachedVisualWidth = -1;
  private cachedVisualLines: string[] = [];
  private cachedLineMap: number[] = []; // visualLine → logical line index
  private cachedValueForLayout = "";

  getValue(): string {
    return this.value;
  }

  setValue(value: string, cursorPos?: number): void {
    this.value = value;
    this.cursor = cursorPos !== undefined
      ? Math.max(0, Math.min(cursorPos, value.length))
      : value.length;
    this.cachedVisualWidth = -1;
  }

  insertText(text: string): void {
    if (!text) return;
    // Normalize line endings (preserve newlines), replace tabs
    const clean = text.replace(/\r\n/g, "\n").replace(/\r/g, "\n").replace(/\t/g, "    ");
    this.insertAtCursor(clean);
  }

  handleInput(data: string): void {
    if (data.includes("\x1b[200~")) {
      this.isInPaste = true;
      this.pasteBuffer = "";
      data = data.replace("\x1b[200~", "");
    }

    if (this.isInPaste) {
      this.pasteBuffer += data;
      const endIndex = this.pasteBuffer.indexOf("\x1b[201~");
      if (endIndex !== -1) {
        const pasteContent = this.pasteBuffer.substring(0, endIndex);
        this.handlePaste(pasteContent);
        this.isInPaste = false;
        const remaining = this.pasteBuffer.substring(endIndex + 6);
        this.pasteBuffer = "";
        if (remaining) this.handleInput(remaining);
      }
      return;
    }
  }

  handleKey(key: string): boolean {
    // Escape
    if (key === "escape") {
      if (this.onEscape) this.onEscape();
      return true;
    }

    // Submit
    if (key === "enter") {
      const v = this.value;
      if (v && (this.history.length === 0 || this.history[0] !== v)) {
        this.history.unshift(v);
      }
      this.historyIndex = -1;
      this.historyDraft = "";
      if (this.onSubmit) this.onSubmit(v);
      return true;
    }

    // Insert newline (Alt+Enter is most portable; Shift+Enter needs Kitty/modifyOtherKeys; Ctrl+J as fallback)
    if (key === "alt+enter" || key === "shift+enter" || key === "ctrl+enter" || key === "ctrl+j") {
      this.insertAtCursor("\n");
      return true;
    }

    // ── History vs line navigation ──────────────────────────────────

    const totalVisualLines = this.countVisualLines();

    if (key === "up") {
      if (totalVisualLines > 1) {
        const info = this.getCursorVisualInfo();
        if (info.visualLine === 0) {
          return this.historyUp();
        }
        this.moveUpVisualLine();
        return true;
      }
      return this.historyUp();
    }

    if (key === "down") {
      if (totalVisualLines > 1) {
        const info = this.getCursorVisualInfo();
        if (info.visualLine >= totalVisualLines - 1) {
          return this.historyDown();
        }
        this.moveDownVisualLine();
        return true;
      }
      return this.historyDown();
    }

    // Deletion
    if (key === "backspace" || key === "ctrl+h") {
      this.handleBackspace();
      return true;
    }

    if (key === "delete") {
      this.handleForwardDelete();
      return true;
    }

    if (key === "alt+backspace" || key === "ctrl+w") {
      this.deleteWordBackwards();
      return true;
    }

    if (key === "alt+d" || key === "alt+delete") {
      this.deleteWordForward();
      return true;
    }

    if (key === "ctrl+u") {
      this.deleteToLineStart();
      return true;
    }

    if (key === "ctrl+k") {
      this.deleteToLineEnd();
      return true;
    }

    // Yank / Undo (no-op stubs)
    if (key === "ctrl+y") return true;
    if (key === "ctrl+-" || key === "ctrl+/" || key === "ctrl+_" || key === "ctrl+z") return true;

    // Cursor movement
    if (key === "left" || key === "ctrl+b") {
      if (this.cursor > 0) {
        const beforeCursor = this.value.slice(0, this.cursor);
        const graphemes = [...segmenter.segment(beforeCursor)];
        const lastGrapheme = graphemes[graphemes.length - 1];
        this.cursor -= lastGrapheme ? lastGrapheme.segment.length : 1;
        while (this.cursor > 0 && this.value[this.cursor] === "\n") {
          this.cursor--;
        }
      }
      return true;
    }

    if (key === "right" || key === "ctrl+f") {
      if (this.cursor < this.value.length) {
        const afterCursor = this.value.slice(this.cursor);
        const graphemes = [...segmenter.segment(afterCursor)];
        const firstGrapheme = graphemes[0];
        this.cursor += firstGrapheme ? firstGrapheme.segment.length : 1;
        while (this.cursor < this.value.length && this.value[this.cursor] === "\n") {
          this.cursor++;
        }
      }
      return true;
    }

    // Home/End — line-aware when multi-line, whole-value when single-line
    if (key === "home") {
      const multiline = this.value.includes("\n");
      if (multiline) {
        const { start } = this.getLineBounds(this.cursor);
        this.cursor = this.cursor === start ? 0 : start;
      } else {
        this.cursor = 0;
      }
      return true;
    }

    if (key === "end") {
      const multiline = this.value.includes("\n");
      if (multiline) {
        const { end } = this.getLineBounds(this.cursor);
        this.cursor = this.cursor === end ? this.value.length : end;
      } else {
        this.cursor = this.value.length;
      }
      return true;
    }

    if (key === "ctrl+a") {
      this.cursor = 0;
      return true;
    }

    if (key === "ctrl+e") {
      this.cursor = this.value.length;
      return true;
    }

    if (key === "ctrl+left" || key === "alt+b") {
      this.moveWordBackwards();
      return true;
    }

    if (key === "ctrl+right" || key === "alt+f") {
      this.moveWordForwards();
      return true;
    }

    // Space
    if (key === "space") {
      this.insertAtCursor(" ");
      return true;
    }

    // Shifted characters: shift+a → A, shift+1 → !, etc.
    if (key.startsWith("shift+") && key.length === 7) {
      const ch = key[6]!;
      if (ch >= "a" && ch <= "z") {
        this.insertAtCursor(ch.toUpperCase());
        return true;
      }
    }

    // Printable single character
    if (key.length === 1) {
      const code = key.charCodeAt(0);
      if (code >= 32) {
        this.insertAtCursor(key);
        return true;
      }
    }

    return false;
  }

  // ── Visual layout helpers ─────────────────────────────────────────

  /**
   * Build (and cache) the visual line layout for the current value + width.
   * Returns visual lines without prompt prefix.
   */
  private buildVisualLayout(availableWidth: number): string[] {
    if (availableWidth <= 0) return [""];
    if (this.cachedVisualWidth === availableWidth && this.cachedVisualLines.length > 0 && this.cachedValueForLayout === this.value) {
      return this.cachedVisualLines;
    }

    const lines: string[] = [];
    const lineMap: number[] = [];
    const valueLines = this.value.split("\n");

    for (let li = 0; li < valueLines.length; li++) {
      const logicalLine = valueLines[li]!;
      const wrapped = wrapTextWithAnsi(logicalLine || " ", availableWidth);
      // Skip empty result (shouldn't happen, but guard)
      const subLines = wrapped.length > 0 ? wrapped : [" "];
      for (const sub of subLines) {
        lines.push(sub);
        lineMap.push(li);
      }
    }

    this.cachedVisualWidth = availableWidth;
    this.cachedVisualLines = lines;
    this.cachedLineMap = lineMap;
    this.cachedValueForLayout = this.value;
    return lines;
  }

  /** Count total visual lines for the current width. */
  private countVisualLines(): number {
    // Use a cached width from last render or a reasonable default
    const w = this.cachedVisualWidth > 0 ? this.cachedVisualWidth : 80;
    return this.buildVisualLayout(w).length;
  }

  /**
   * Find which visual line and column the cursor sits on.
   * Uses the last cached layout width.
   */
  private getCursorVisualInfo(): CursorVisualInfo {
    const w = this.cachedVisualWidth > 0 ? this.cachedVisualWidth : 80;
    const lines = this.buildVisualLayout(w);

    let visualLine = 0;
    let consumed = 0;

    for (let vi = 0; vi < lines.length; vi++) {
      const sub = lines[vi]!;
      const plain = stripAnsiCodes(sub);
      const subLen = plain.length;

      if (this.cursor <= consumed + subLen || vi === lines.length - 1) {
        // Cursor is in (or at the end of) this visual sub-line
        const offsetInSub = Math.max(0, this.cursor - consumed);
        const colInWrapped = visibleWidth(plain.slice(0, offsetInSub));
        return { visualLine: vi, colInWrapped, subLineText: sub };
      }

      consumed += subLen;
      visualLine = vi + 1;
    }

    // Fallback: cursor at end of last line
    return { visualLine: lines.length - 1, colInWrapped: 0, subLineText: "" };
  }

  /**
   * Map a (visualLine, column) pair back to a cursor position in the raw value.
   */
  private cursorFromVisual(targetVL: number, targetCol: number, availableWidth: number): number {
    const lines = this.buildVisualLayout(availableWidth);
    const vl = Math.max(0, Math.min(targetVL, lines.length - 1));

    let consumed = 0;
    for (let vi = 0; vi < vl; vi++) {
      consumed += stripAnsiCodes(lines[vi]!).length;
    }

    // Find the byte offset within the target visual line corresponding to targetCol
    const sub = lines[vl]!;
    const plain = stripAnsiCodes(sub);
    let col = 0;
    let byteOff = 0;
    const segs = segmenter.segment(plain);
    for (const seg of segs) {
      const segWidth = visibleWidth(seg.segment);
      if (col + segWidth > targetCol) break;
      col += segWidth;
      byteOff += seg.segment.length;
    }

    return consumed + byteOff;
  }

  // ── Visual line navigation (soft-wrap aware) ─────────────────────

  private moveUpVisualLine(): void {
    const info = this.getCursorVisualInfo();
    if (info.visualLine <= 0) return;
    const w = this.cachedVisualWidth > 0 ? this.cachedVisualWidth : 80;
    this.cursor = this.cursorFromVisual(info.visualLine - 1, info.colInWrapped, w);
  }

  private moveDownVisualLine(): void {
    const info = this.getCursorVisualInfo();
    const w = this.cachedVisualWidth > 0 ? this.cachedVisualWidth : 80;
    const total = this.buildVisualLayout(w).length;
    if (info.visualLine >= total - 1) return;
    this.cursor = this.cursorFromVisual(info.visualLine + 1, info.colInWrapped, w);
  }

  // ── Logical line helpers (hard \n boundaries) ────────────────────

  /** Get start/end offsets of the logical line containing cursorPos. */
  private getLineBounds(cursorPos: number): { start: number; end: number } {
    const start = this.value.lastIndexOf("\n", cursorPos - 1) + 1;
    const endIdx = this.value.indexOf("\n", cursorPos);
    const end = endIdx === -1 ? this.value.length : endIdx;
    return { start, end };
  }

  /** Visual column of cursor within its current logical line. */
  private cursorColInLine(cursorPos: number): number {
    const { start } = this.getLineBounds(cursorPos);
    return visibleWidth(this.value.slice(start, cursorPos));
  }

  /** Move cursor to target visual column within a logical line. */
  private setCursorToLineCol(lineStart: number, visualCol: number): void {
    const lineEnd = this.value.indexOf("\n", lineStart);
    const end = lineEnd === -1 ? this.value.length : lineEnd;
    const line = this.value.slice(lineStart, end);

    let col = 0;
    let offset = 0;
    const segs = segmenter.segment(line);
    for (const seg of segs) {
      const segWidth = visibleWidth(seg.segment);
      if (col + segWidth > visualCol) break;
      col += segWidth;
      offset += seg.segment.length;
    }
    this.cursor = lineStart + offset;
  }

  // ── History navigation ────────────────────────────────────────────

  private historyUp(): boolean {
    if (this.history.length === 0) return true;
    if (this.historyIndex === -1) {
      this.historyDraft = this.value;
      this.historyIndex = 0;
    } else if (this.historyIndex < this.history.length - 1) {
      this.historyIndex++;
    }
    this.value = this.history[this.historyIndex] ?? this.historyDraft;
    this.cursor = this.value.length;
    this.onChange?.(this.value);
    return true;
  }

  private historyDown(): boolean {
    if (this.historyIndex === -1) return true;
    if (this.historyIndex > 0) {
      this.historyIndex--;
      this.value = this.history[this.historyIndex] ?? this.historyDraft;
    } else {
      this.historyIndex = -1;
      this.value = this.historyDraft;
    }
    this.cursor = this.value.length;
    this.onChange?.(this.value);
    return true;
  }

  // ── Text manipulation ─────────────────────────────────────────────

  private insertAtCursor(text: string): void {
    this.value = this.value.slice(0, this.cursor) + text + this.value.slice(this.cursor);
    this.cursor += text.length;
    this.cachedVisualWidth = -1;
    this.onChange?.(this.value);
  }

  private handleBackspace(): void {
    if (this.cursor > 0) {
      const beforeCursor = this.value.slice(0, this.cursor);
      const graphemes = [...segmenter.segment(beforeCursor)];
      const lastGrapheme = graphemes[graphemes.length - 1];
      const graphemeLength = lastGrapheme ? lastGrapheme.segment.length : 1;
      this.value = this.value.slice(0, this.cursor - graphemeLength) + this.value.slice(this.cursor);
      this.cursor -= graphemeLength;
      this.cachedVisualWidth = -1;
      this.onChange?.(this.value);
    }
  }

  private handleForwardDelete(): void {
    if (this.cursor < this.value.length) {
      const afterCursor = this.value.slice(this.cursor);
      const graphemes = [...segmenter.segment(afterCursor)];
      const firstGrapheme = graphemes[0];
      const graphemeLength = firstGrapheme ? firstGrapheme.segment.length : 1;
      this.value = this.value.slice(0, this.cursor) + this.value.slice(this.cursor + graphemeLength);
      this.cachedVisualWidth = -1;
      this.onChange?.(this.value);
    }
  }

  private deleteToLineStart(): void {
    const { start } = this.getLineBounds(this.cursor);
    if (this.cursor === start) {
      if (start > 0) {
        const beforeNewline = start - 1;
        this.value = this.value.slice(0, beforeNewline) + this.value.slice(this.cursor);
        this.cursor = beforeNewline;
        this.cachedVisualWidth = -1;
        this.onChange?.(this.value);
      }
      return;
    }
    this.value = this.value.slice(0, start) + this.value.slice(this.cursor);
    this.cursor = start;
    this.cachedVisualWidth = -1;
    this.onChange?.(this.value);
  }

  private deleteToLineEnd(): void {
    const { end } = this.getLineBounds(this.cursor);
    if (this.cursor >= end) return;
    this.value = this.value.slice(0, this.cursor) + this.value.slice(end);
    this.cachedVisualWidth = -1;
    this.onChange?.(this.value);
  }

  private deleteWordBackwards(): void {
    if (this.cursor === 0) return;
    const oldCursor = this.cursor;
    this.moveWordBackwards();
    const deleteFrom = this.cursor;
    this.cursor = oldCursor;
    this.value = this.value.slice(0, deleteFrom) + this.value.slice(this.cursor);
    this.cursor = deleteFrom;
    this.cachedVisualWidth = -1;
    this.onChange?.(this.value);
  }

  private deleteWordForward(): void {
    if (this.cursor >= this.value.length) return;
    const oldCursor = this.cursor;
    this.moveWordForwards();
    const deleteTo = this.cursor;
    this.cursor = oldCursor;
    this.value = this.value.slice(0, this.cursor) + this.value.slice(deleteTo);
    this.cachedVisualWidth = -1;
    this.onChange?.(this.value);
  }

  private moveWordBackwards(): void {
    if (this.cursor === 0) return;
    const textBeforeCursor = this.value.slice(0, this.cursor);
    const graphemes = [...segmenter.segment(textBeforeCursor)];

    while (graphemes.length > 0 && isWhitespaceChar(graphemes[graphemes.length - 1]?.segment || "")) {
      this.cursor -= graphemes.pop()?.segment.length || 0;
    }

    if (graphemes.length > 0) {
      const lastGrapheme = graphemes[graphemes.length - 1]?.segment || "";
      if (isPunctuationChar(lastGrapheme)) {
        while (graphemes.length > 0 && isPunctuationChar(graphemes[graphemes.length - 1]?.segment || "")) {
          this.cursor -= graphemes.pop()?.segment.length || 0;
        }
      } else {
        while (
          graphemes.length > 0 &&
          !isWhitespaceChar(graphemes[graphemes.length - 1]?.segment || "") &&
          !isPunctuationChar(graphemes[graphemes.length - 1]?.segment || "")
        ) {
          this.cursor -= graphemes.pop()?.segment.length || 0;
        }
      }
    }
  }

  private moveWordForwards(): void {
    if (this.cursor >= this.value.length) return;
    const textAfterCursor = this.value.slice(this.cursor);
    const segments = segmenter.segment(textAfterCursor);
    const iterator = segments[Symbol.iterator]();
    let next = iterator.next();

    while (!next.done && isWhitespaceChar(next.value.segment)) {
      this.cursor += next.value.segment.length;
      next = iterator.next();
    }

    if (!next.done) {
      const firstGrapheme = next.value.segment;
      if (isPunctuationChar(firstGrapheme)) {
        while (!next.done && isPunctuationChar(next.value.segment)) {
          this.cursor += next.value.segment.length;
          next = iterator.next();
        }
      } else {
        while (!next.done && !isWhitespaceChar(next.value.segment) && !isPunctuationChar(next.value.segment)) {
          this.cursor += next.value.segment.length;
          next = iterator.next();
        }
      }
    }
  }

  // ── Paste handling ────────────────────────────────────────────────

  private handlePaste(pastedText: string): void {
    const cleanText = pastedText
      .replace(/\r\n/g, "\n")
      .replace(/\r/g, "\n")
      .replace(/\t/g, "    ");
    this.insertAtCursor(cleanText);
  }

  invalidate(): void {
    this.cachedVisualWidth = -1;
  }

  // ── Render ────────────────────────────────────────────────────────

  render(screenWidth: number): string[] {
    const promptWidth = 2; // "> " or "  "
    const availableWidth = Math.max(0, screenWidth - promptWidth);

    const visualLines = this.buildVisualLayout(availableWidth);
    const cursorInfo = (availableWidth > 0) ? this.getCursorVisualInfo() : null;

    const output: string[] = [];

    for (let vi = 0; vi < visualLines.length; vi++) {
      const isFirstLine = vi === 0;
      const prompt = isFirstLine ? "> " : "  ";
      const subText = visualLines[vi]!;
      const isCursorLine = cursorInfo !== null && vi === cursorInfo.visualLine;

      if (isCursorLine && availableWidth > 0) {
        output.push(prompt + this.renderCursorInLine(subText, cursorInfo!.colInWrapped, availableWidth));
      } else {
        const plain = stripAnsiCodes(subText);
        const visW = visibleWidth(plain);
        if (availableWidth > 0) {
          output.push(prompt + subText + " ".repeat(Math.max(0, availableWidth - visW)));
        } else {
          output.push(prompt);
        }
      }
    }

    return output;
  }

  /** Render a wrapped sub-line with cursor highlighting at the given visual column. */
  private renderCursorInLine(text: string, cursorVisCol: number, availableWidth: number): string {
    // Find byte offset in the original text (may contain ANSI codes from wrapping)
    let byteOff = 0;
    let col = 0;
    let i = 0;

    while (i < text.length && col < cursorVisCol) {
      // Skip ANSI escape sequences (CSI, OSC, etc.) — they have no visual width
      const ansi = extractAnsiCode(text, i);
      if (ansi) {
        i += ansi.length;
        continue;
      }
      // Regular character — advance by one grapheme
      const segIter = segmenter.segment(text.slice(i))[Symbol.iterator]();
      const segResult = segIter.next();
      if (segResult.done) break;
      const grapheme = segResult.value.segment;
      col += visibleWidth(grapheme);
      i += grapheme.length;
    }
    byteOff = i;

    // Find the character at the cursor position (skip any ANSI codes just after byteOff)
    let j = byteOff;
    while (j < text.length) {
      const ansi = extractAnsiCode(text, j);
      if (ansi) { j += ansi.length; continue; }
      break;
    }
    const segIter2 = segmenter.segment(text.slice(j))[Symbol.iterator]();
    const segResult2 = segIter2.next();
    const atCursor = segResult2.done ? " " : segResult2.value.segment;
    const afterCursorStart = j + atCursor.length;

    const beforeCursor = text.slice(0, byteOff);
    const afterCursor = text.slice(afterCursorStart);

    const marker = this.focused ? CURSOR_MARKER : "";
    const cursorChar = this.focused ? `\x1b[7m${atCursor}\x1b[27m` : atCursor;
    const textWithCursor = beforeCursor + marker + cursorChar + afterCursor;

    // Compute visual length of the rendered content (without cursor marker)
    const renderedContent = beforeCursor + atCursor + afterCursor;
    const visualLength = visibleWidth(stripAnsiCodes(renderedContent));
    const padding = " ".repeat(Math.max(0, availableWidth - visualLength));

    return textWithCursor + padding;
  }
}