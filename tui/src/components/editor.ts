/**
 * Editor component - the main text input at the bottom of the screen.
 * Mimics the pi-tui EditorComponent.
 */

import { CSI, RESET, CURSOR_SHOW, CURSOR_HIDE, CURSOR_MARKER } from "../tui.js";
import type { Component, Focusable } from "../tui.js";

export interface EditorTheme {
  prompt: number;        // prompt character color
  text: number;         // input text color
  cursor: number;       // cursor color
  cursorText: number;   // selected text color
  bg: number;           // background color
}

export interface EditorCallbacks {
  onSubmit?: (value: string) => void;
  onChange?: (value: string) => void;
  onFocus?: () => void;
  onBlur?: () => void;
}

export class Editor implements Component, Focusable {
  private value = "";
  private cursorPos = 0;
  private inserting = true; // overwrite vs insert mode
  private theme: EditorTheme;
  private callbacks: EditorCallbacks;
  focused = true;
  private dim = 245;
  private history: string[] = [];
  private historyIndex = -1;
  private multiline = false;
  private prefix = "❯ ";

  constructor(prefix = "❯ ", theme?: Partial<EditorTheme>, callbacks?: EditorCallbacks) {
    this.prefix = prefix;
    this.theme = {
      prompt: 39,  // blue
      text: 252,  // white
      cursor: 39,
      cursorText: 17,
      bg: 235,
      ...theme,
    };
    this.callbacks = callbacks ?? {};
  }

  // ─── Public API ─────────────────────────────────────────────────────────

  setValue(value: string): void {
    this.value = value;
    this.cursorPos = value.length;
    this.historyIndex = -1;
    this.callbacks.onChange?.(value);
  }

  getValue(): string {
    return this.value;
  }

  insertText(text: string): void {
    if (this.cursorPos === this.value.length) {
      this.value += text;
    } else if (this.inserting) {
      this.value = this.value.slice(0, this.cursorPos) + text + this.value.slice(this.cursorPos + text.length);
    } else {
      this.value = this.value.slice(0, this.cursorPos) + text + this.value.slice(this.cursorPos + text.length);
    }
    this.cursorPos += text.length;
    this.callbacks.onChange?.(this.value);
  }

  setPrefix(prefix: string): void {
    this.prefix = prefix;
  }

  handleInput(data: string): void {
    // parseKey already produces normalized names like "ctrl+a", "up", "a"
    this.handleKey(data);
  }

  invalidate(): void { /* no cache */ }

  // ─── Key Handling ──────────────────────────────────────────────────────

  handleKey(key: string): boolean {
    switch (key) {
      case "left":
        if (this.cursorPos > 0) this.cursorPos--;
        return true;
      case "right":
        if (this.cursorPos < this.value.length) this.cursorPos++;
        return true;
      case "home":
        this.cursorPos = 0;
        return true;
      case "end":
        this.cursorPos = this.value.length;
        return true;
      case "backspace":
        if (this.cursorPos > 0) {
          this.value = this.value.slice(0, this.cursorPos - 1) + this.value.slice(this.cursorPos);
          this.cursorPos--;
          this.callbacks.onChange?.(this.value);
        }
        return true;
      case "delete":
        if (this.cursorPos < this.value.length) {
          this.value = this.value.slice(0, this.cursorPos) + this.value.slice(this.cursorPos + 1);
          this.callbacks.onChange?.(this.value);
        }
        return true;
      case "enter":
        if (this.value.trim()) {
          this.history.unshift(this.value);
          this.historyIndex = -1;
          this.callbacks.onSubmit?.(this.value);
          this.value = "";
          this.cursorPos = 0;
        }
        return true;
      case "up":
        if (this.historyIndex < this.history.length - 1) {
          this.historyIndex++;
          this.value = this.history[this.historyIndex];
          this.cursorPos = this.value.length;
        }
        return true;
      case "down":
        if (this.historyIndex > 0) {
          this.historyIndex--;
          this.value = this.history[this.historyIndex];
          this.cursorPos = this.value.length;
        } else if (this.historyIndex === 0) {
          this.historyIndex = -1;
          this.value = "";
          this.cursorPos = 0;
        }
        return true;
      case "tab":
        // Autocomplete handled by App
        return true;
      case "ctrl+a":
        this.cursorPos = 0;
        return true;
      case "ctrl+e":
        this.cursorPos = this.value.length;
        return true;
      case "ctrl+u":
        this.value = this.value.slice(this.cursorPos);
        this.cursorPos = 0;
        this.callbacks.onChange?.(this.value);
        return true;
      case "ctrl+w":
        if (this.cursorPos > 0) {
          const before = this.value.slice(0, this.cursorPos);
          const after = this.value.slice(this.cursorPos);
          const lastSpace = before.trimEnd().lastIndexOf(' ');
          const deleteTo = lastSpace < 0 ? 0 : lastSpace + 1;
          this.value = before.slice(0, deleteTo) + after;
          this.cursorPos = deleteTo;
          this.callbacks.onChange?.(this.value);
        }
        return true;
      default:
        if (key.length === 1 && key.charCodeAt(0) >= 32) {
          // Insert character
          if (this.cursorPos === this.value.length) {
            this.value += key;
          } else {
            this.value = this.value.slice(0, this.cursorPos) + key + this.value.slice(this.cursorPos);
          }
          this.cursorPos++;
          this.callbacks.onChange?.(this.value);
          return true;
        }
        return false;
    }
  }

  // ─── Rendering ────────────────────────────────────────────────────────

  /**
   * Render the editor for the given width and screen height.
   * Returns an array of lines to render at the bottom of the screen.
   */
  render(screenWidth: number): string[] {
    const prefixStr = `${CSI}38;5;${this.theme.prompt}m${this.prefix}${RESET}`;
    const totalWidth = screenWidth;

    // Render input as: prefix + text + cursor
    let display = prefixStr;
    const textPart = this.value;
    const cursorInPart = this.cursorPos;

    if (this.focused && this.cursorPos === this.value.length) {
      // Cursor at end: prefix + text + cursor
      display += `${CSI}38;5;${this.theme.text}m${textPart}${CSI}0m`;
      display += CURSOR_MARKER + `${CSI}38;5;${this.theme.bg}m${CSI}48;5;${this.theme.cursor}m ${CSI}0m`;
    } else if (this.focused) {
      // Cursor in middle
      const before = textPart.slice(0, cursorInPart);
      const char = textPart[cursorInPart] ?? " ";
      const after = textPart.slice(cursorInPart + 1);
      display += `${CSI}38;5;${this.theme.text}m${before}${CSI}0m`;
      display += CURSOR_MARKER + `${CSI}38;5;${this.theme.cursorText}m${CSI}48;5;${this.theme.cursor}m${char}${CSI}0m`;
      display += `${CSI}38;5;${this.theme.text}m${after}${CSI}0m`;
    } else {
      display += `${CSI}38;5;${this.dim}m${textPart}${CSI}0m`;
    }

    return [display];
  }
}
