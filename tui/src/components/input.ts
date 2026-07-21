/**
 * Input component — multi-line text input with history.
 * Enter submits, Alt+Enter / Shift+Enter inserts a newline.
 * Up/Down navigates lines when multi-line; navigates history when single-line.
 * Paste preserves newlines (multi-line paste).
 * Implements Component + Focusable.
 */

import { type Component, CURSOR_MARKER, type Focusable } from "../tui.js";
import { getSegmenter, isPunctuationChar, isWhitespaceChar, sliceByColumn, visibleWidth } from "../utils.js";

const segmenter = getSegmenter();

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

  getValue(): string {
    return this.value;
  }

  setValue(value: string, cursorPos?: number): void {
    this.value = value;
    this.cursor = cursorPos !== undefined
      ? Math.max(0, Math.min(cursorPos, value.length))
      : value.length;
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

    const multiline = this.isMultiline();

    if (key === "up") {
      // Multi-line: navigate lines first, history only at first line
      if (multiline) {
        const { start } = this.getLineBounds(this.cursor);
        if (start === 0) {
          // At first line — navigate history
          return this.historyUp();
        }
        this.moveUpLine();
        return true;
      }
      // Single-line: history only
      return this.historyUp();
    }

    if (key === "down") {
      if (multiline) {
        const { end } = this.getLineBounds(this.cursor);
        if (end >= this.value.length) {
          // At last line — navigate history
          return this.historyDown();
        }
        this.moveDownLine();
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
      }
      return true;
    }

    if (key === "right" || key === "ctrl+f") {
      if (this.cursor < this.value.length) {
        const afterCursor = this.value.slice(this.cursor);
        const graphemes = [...segmenter.segment(afterCursor)];
        const firstGrapheme = graphemes[0];
        this.cursor += firstGrapheme ? firstGrapheme.segment.length : 1;
      }
      return true;
    }

    // Home/End — line-aware when multi-line, whole-value when single-line
    if (key === "home") {
      if (multiline) {
        const { start } = this.getLineBounds(this.cursor);
        // If already at line start, go to value start
        this.cursor = this.cursor === start ? 0 : start;
      } else {
        this.cursor = 0;
      }
      return true;
    }

    if (key === "end") {
      if (multiline) {
        const { end } = this.getLineBounds(this.cursor);
        // If already at line end, go to value end
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

  // ── Multi-line navigation helpers ──────────────────────────────────

  private isMultiline(): boolean {
    return this.value.includes("\n");
  }

  /** Get start/end offsets of the line containing cursorPos. */
  private getLineBounds(cursorPos: number): { start: number; end: number } {
    const start = this.value.lastIndexOf("\n", cursorPos - 1) + 1;
    const endIdx = this.value.indexOf("\n", cursorPos);
    const end = endIdx === -1 ? this.value.length : endIdx;
    return { start, end };
  }

  /** Visual column of cursor within its current line. */
  private cursorColInLine(cursorPos: number): number {
    const { start } = this.getLineBounds(cursorPos);
    return visibleWidth(this.value.slice(start, cursorPos));
  }

  /** Move cursor to target visual column on a given line (by line start offset). */
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

  private moveUpLine(): void {
    const { start } = this.getLineBounds(this.cursor);
    if (start === 0) return;
    const col = this.cursorColInLine(this.cursor);
    const prevLineEnd = start - 1;
    const prevLineStart = this.value.lastIndexOf("\n", prevLineEnd - 1) + 1;
    this.setCursorToLineCol(prevLineStart, col);
  }

  private moveDownLine(): void {
    const { end } = this.getLineBounds(this.cursor);
    if (end >= this.value.length) return;
    const col = this.cursorColInLine(this.cursor);
    const nextLineStart = end + 1;
    this.setCursorToLineCol(nextLineStart, col);
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
      this.onChange?.(this.value);
    }
  }

  private deleteToLineStart(): void {
    const { start } = this.getLineBounds(this.cursor);
    if (this.cursor === start) {
      // At line start — delete the preceding newline to join with the previous line.
      // This makes holding Ctrl+U delete line after line.
      if (start > 0) {
        // The character before start is always \n (since start is after a newline or 0)
        const beforeNewline = start - 1; // position of \n
        this.value = this.value.slice(0, beforeNewline) + this.value.slice(this.cursor);
        this.cursor = beforeNewline;
        this.onChange?.(this.value);
      }
      return;
    }
    this.value = this.value.slice(0, start) + this.value.slice(this.cursor);
    this.cursor = start;
    this.onChange?.(this.value);
  }

  private deleteToLineEnd(): void {
    const { end } = this.getLineBounds(this.cursor);
    if (this.cursor >= end) return;
    this.value = this.value.slice(0, this.cursor) + this.value.slice(end);
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
    this.onChange?.(this.value);
  }

  private deleteWordForward(): void {
    if (this.cursor >= this.value.length) return;
    const oldCursor = this.cursor;
    this.moveWordForwards();
    const deleteTo = this.cursor;
    this.cursor = oldCursor;
    this.value = this.value.slice(0, this.cursor) + this.value.slice(deleteTo);
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
    // Normalize line endings (preserve newlines), replace tabs
    const cleanText = pastedText
      .replace(/\r\n/g, "\n")
      .replace(/\r/g, "\n")
      .replace(/\t/g, "    ");
    this.insertAtCursor(cleanText);
  }

  invalidate(): void {
    // No cached state to invalidate
  }

  // ── Render ────────────────────────────────────────────────────────

  render(screenWidth: number): string[] {
    const lines: string[] = [];
    const valueLines = this.value.split("\n");
    const cursorLineIndex = this.getCursorLineIndex();

    for (let i = 0; i < valueLines.length; i++) {
      const isFirstLine = i === 0;
      const prompt = isFirstLine ? "> " : "  ";
      const availableWidth = screenWidth - prompt.length;

      if (availableWidth <= 0) {
        lines.push(prompt);
        continue;
      }

      const lineText = valueLines[i]!;
      const isCursorLine = i === cursorLineIndex;
      const cursorInThisLine = isCursorLine ? this.cursor - this.getLineStartOffset(i) : -1;

      const rendered = this.renderLine(lineText, cursorInThisLine, availableWidth, isCursorLine);
      lines.push(prompt + rendered);
    }

    return lines;
  }

  /** Which line (by index) the cursor is on. */
  private getCursorLineIndex(): number {
    let pos = 0;
    const valueLines = this.value.split("\n");
    for (let i = 0; i < valueLines.length; i++) {
      pos += valueLines[i]!.length;
      if (this.cursor <= pos) return i;
      pos += 1; // for the \n
    }
    return valueLines.length - 1;
  }

  /** Get the starting byte offset of line i in the value. */
  private getLineStartOffset(lineIndex: number): number {
    let offset = 0;
    const valueLines = this.value.split("\n");
    for (let i = 0; i < lineIndex; i++) {
      offset += valueLines[i]!.length + 1; // +1 for \n
    }
    return offset;
  }

  /** Render a single line with optional cursor highlighting and horizontal scroll. */
  private renderLine(
    lineText: string,
    cursorInLine: number,
    availableWidth: number,
    isCursorLine: boolean,
  ): string {
    if (!isCursorLine) {
      // Non-cursor line: just slice to fit
      if (visibleWidth(lineText) <= availableWidth) {
        return lineText + " ".repeat(availableWidth - visibleWidth(lineText));
      }
      return sliceByColumn(lineText, 0, availableWidth);
    }

    // Cursor line — full rendering with scroll and cursor highlight
    const totalWidth = visibleWidth(lineText);

    if (totalWidth < availableWidth) {
      // Fits entirely — render with cursor and padding
      return this.renderWithCursor(lineText, cursorInLine, availableWidth, 0);
    }

    // Needs horizontal scrolling
    const cursorCol = visibleWidth(lineText.slice(0, cursorInLine));
    const scrollWidth = cursorInLine === lineText.length ? availableWidth - 1 : availableWidth;

    let startCol = 0;
    if (scrollWidth > 0) {
      const halfWidth = Math.floor(scrollWidth / 2);
      if (cursorCol < halfWidth) {
        startCol = 0;
      } else if (cursorCol > totalWidth - halfWidth) {
        startCol = Math.max(0, totalWidth - scrollWidth);
      } else {
        startCol = Math.max(0, cursorCol - halfWidth);
      }
    }

    const visibleText = sliceByColumn(lineText, startCol, startCol + scrollWidth);
    const cursorDisplay = visibleText.length > 0
      ? sliceByColumn(lineText, startCol, cursorCol).length
      : 0;

    return this.renderWithCursor(visibleText, cursorDisplay, availableWidth, startCol);
  }

  /** Render text with cursor highlight and padding to fill availableWidth. */
  private renderWithCursor(
    text: string,
    cursorOffset: number,
    availableWidth: number,
    _scrollStart: number,
  ): string {
    const graphemes = [...segmenter.segment(text.slice(cursorOffset))];
    const cursorGrapheme = graphemes[0];

    const beforeCursor = text.slice(0, cursorOffset);
    const atCursor = cursorGrapheme?.segment ?? " ";
    const afterCursor = text.slice(cursorOffset + atCursor.length);

    const marker = this.focused ? CURSOR_MARKER : "";
    const cursorChar = this.focused ? `\x1b[7m${atCursor}\x1b[27m` : atCursor;
    const textWithCursor = beforeCursor + marker + cursorChar + afterCursor;

    const renderedContent = beforeCursor + atCursor + afterCursor;
    const visualLength = visibleWidth(renderedContent);
    const padding = " ".repeat(Math.max(0, availableWidth - visualLength));

    return textWithCursor + padding;
  }
}
