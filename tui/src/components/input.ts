/**
 * Input component — single-line text input with horizontal scrolling.
 * Ported from pi's input.ts. Implements Component + Focusable.
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

  setValue(value: string): void {
    this.value = value;
    this.cursor = Math.min(this.cursor, value.length);
  }

  insertText(text: string): void {
    if (!text) return;
    const clean = text.replace(/\r\n/g, "").replace(/\r/g, "").replace(/\n/g, "");
    this.insertCharacter(clean);
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

    // History navigation
    if (key === "up") {
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
    if (key === "down") {
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

    // Yank
    if (key === "ctrl+y") {
      return true;
    }

    // Undo
    if (key === "ctrl+-" || key === "ctrl+/" || key === "ctrl+_" || key === "ctrl+z") {
      return true;
    }

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

    if (key === "home" || key === "ctrl+a") {
      this.cursor = 0;
      return true;
    }

    if (key === "end" || key === "ctrl+e") {
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
      this.insertCharacter(" ");
      return true;
    }

    // Shifted characters: shift+a → A, shift+1 → !, etc.
    if (key.startsWith("shift+") && key.length === 7) {
      const ch = key[6]!;
      if (ch >= "a" && ch <= "z") {
        this.insertCharacter(ch.toUpperCase());
        return true;
      }
      // shift+digit and shift+symbol handled via default printable below
    }

    // Printable single character
    if (key.length === 1) {
      const code = key.charCodeAt(0);
      if (code >= 32) {
        this.insertCharacter(key);
        return true;
      }
    }

    return false;
  }

  private insertCharacter(char: string): void {
    this.value = this.value.slice(0, this.cursor) + char + this.value.slice(this.cursor);
    this.cursor += char.length;
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
    if (this.cursor === 0) return;
    this.value = this.value.slice(this.cursor);
    this.cursor = 0;
    this.onChange?.(this.value);
  }

  private deleteToLineEnd(): void {
    if (this.cursor >= this.value.length) return;
    this.value = this.value.slice(0, this.cursor);
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

  private handlePaste(pastedText: string): void {
    const cleanText = pastedText.replace(/\r\n/g, "").replace(/\r/g, "").replace(/\n/g, "").replace(/\t/g, "    ");
    this.value = this.value.slice(0, this.cursor) + cleanText + this.value.slice(this.cursor);
    this.cursor += cleanText.length;
    this.onChange?.(this.value);
  }

  invalidate(): void {
    // No cached state to invalidate
  }

  render(screenWidth: number): string[] {
    const prompt = "> ";
    const availableWidth = screenWidth - prompt.length;

    if (availableWidth <= 0) {
      return [prompt];
    }

    let visibleText = "";
    let cursorDisplay = this.cursor;
    const totalWidth = visibleWidth(this.value);

    if (totalWidth < availableWidth) {
      visibleText = this.value;
    } else {
      const scrollWidth = this.cursor === this.value.length ? availableWidth - 1 : availableWidth;
      const cursorCol = visibleWidth(this.value.slice(0, this.cursor));

      if (scrollWidth > 0) {
        const halfWidth = Math.floor(scrollWidth / 2);
        let startCol = 0;

        if (cursorCol < halfWidth) {
          startCol = 0;
        } else if (cursorCol > totalWidth - halfWidth) {
          startCol = Math.max(0, totalWidth - scrollWidth);
        } else {
          startCol = Math.max(0, cursorCol - halfWidth);
        }

        visibleText = sliceByColumn(this.value, startCol, startCol + scrollWidth);
        const beforeCursor = sliceByColumn(this.value, startCol, cursorCol);
        cursorDisplay = beforeCursor.length;
      } else {
        visibleText = "";
        cursorDisplay = 0;
      }
    }

    // Build line with fake cursor
    const graphemes = [...segmenter.segment(visibleText.slice(cursorDisplay))];
    const cursorGrapheme = graphemes[0];

    const beforeCursor = visibleText.slice(0, cursorDisplay);
    const atCursor = cursorGrapheme?.segment ?? " ";
    const afterCursor = visibleText.slice(cursorDisplay + atCursor.length);

    const marker = this.focused ? CURSOR_MARKER : "";
    const cursorChar = this.focused ? `\x1b[7m${atCursor}\x1b[27m` : atCursor;
    const textWithCursor = beforeCursor + marker + cursorChar + afterCursor;

    const renderedContent = beforeCursor + atCursor + afterCursor;
    const visualLength = visibleWidth(renderedContent);
    const padding = " ".repeat(Math.max(0, availableWidth - visualLength));
    const line = prompt + textWithCursor + padding;

    return [line];
  }
}
