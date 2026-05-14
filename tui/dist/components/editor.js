/**
 * Editor component — multi-line text input with Emacs-style keybindings.
 * Ported from pi's editor.ts. Implements Component + Focusable.
 */
import { CSI, RESET, CURSOR_MARKER } from "../tui.js";
import { visibleWidth, sliceWithWidth } from "../utils.js";
// ─── Undo Stack ────────────────────────────────────────────────────────────
class UndoStack {
    stack = [];
    index = -1;
    maxSize;
    constructor(maxSize = 100) {
        this.maxSize = maxSize;
    }
    push(state) {
        this.stack = this.stack.slice(0, this.index + 1);
        this.stack.push(state);
        if (this.stack.length > this.maxSize)
            this.stack.shift();
        this.index = this.stack.length - 1;
    }
    undo() {
        if (this.index <= 0)
            return null;
        this.index--;
        return this.stack[this.index] ?? null;
    }
    redo() {
        if (this.index >= this.stack.length - 1)
            return null;
        this.index++;
        return this.stack[this.index] ?? null;
    }
    clear() {
        this.stack = [];
        this.index = -1;
    }
}
// ─── Kill Ring ─────────────────────────────────────────────────────────────
class KillRing {
    entries = [];
    index = -1;
    accumulating = false;
    maxEntries;
    constructor(maxEntries = 60) {
        this.maxEntries = maxEntries;
    }
    kill(text, accumulate) {
        if (text.length === 0)
            return;
        if (accumulate && this.accumulating && this.entries.length > 0) {
            const last = this.entries[0];
            if (last) {
                this.entries[0] = last + text;
            }
        }
        else {
            this.entries.unshift(text);
            if (this.entries.length > this.maxEntries)
                this.entries.pop();
        }
        this.accumulating = accumulate;
        this.index = -1;
    }
    yank() {
        this.index = 0;
        return this.entries[0] ?? "";
    }
    yankPop() {
        if (this.entries.length === 0)
            return "";
        this.index = (this.index + 1) % this.entries.length;
        return this.entries[this.index] ?? "";
    }
    resetYank() {
        this.index = -1;
    }
    breakAccumulate() {
        this.accumulating = false;
    }
}
// ─── Word Break Helpers ────────────────────────────────────────────────────
function isWordChar(c) {
    const code = c.codePointAt(0);
    if (code === undefined)
        return false;
    return (code >= 48 && code <= 57) || // digits
        (code >= 65 && code <= 90) || // A-Z
        (code >= 97 && code <= 122) || // a-z
        code === 95 || code === 45; // _ -
}
function findWordLeft(value, pos) {
    if (pos <= 0)
        return 0;
    // Skip non-word chars
    let i = pos - 1;
    while (i > 0 && !isWordChar(value[i] ?? ""))
        i--;
    // Skip word chars
    while (i > 0 && isWordChar(value[i - 1] ?? ""))
        i--;
    return i;
}
function findWordRight(value, pos) {
    const len = value.length;
    if (pos >= len)
        return len;
    let i = pos;
    // Skip current word chars
    while (i < len && isWordChar(value[i] ?? ""))
        i++;
    // Skip non-word chars
    while (i < len && !isWordChar(value[i] ?? ""))
        i++;
    return i;
}
// ─── Paste Markers ─────────────────────────────────────────────────────────
const PASTE_MARKER_START = "\x1b_pi:ps\x07";
const PASTE_MARKER_END = "\x1b_pi:pe\x07";
function escapeRegex(s) {
    return s.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}
/** Split text into segments, keeping paste markers as atomic units. */
function segmentWithMarkers(text) {
    const segments = [];
    const markerRegex = new RegExp(`${escapeRegex(PASTE_MARKER_START)}paste_\\d+${escapeRegex(PASTE_MARKER_END)}\\[Paste: [^\\]]+\\]`, "g");
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
// ─── Auto-Dedent ────────────────────────────────────────────────────────────
/** Strip common leading whitespace from all lines in a multi-line text. */
function autoDedent(text) {
    const lines = text.split("\n");
    // Don't dedent single-line text
    if (lines.length <= 1)
        return text;
    // Find minimum leading whitespace among non-empty lines
    let minIndent = Infinity;
    for (const line of lines) {
        if (line.trim().length === 0)
            continue;
        const match = line.match(/^[ \t]*/);
        if (match)
            minIndent = Math.min(minIndent, match[0].length);
    }
    if (minIndent === 0 || minIndent === Infinity)
        return text;
    // Strip the common prefix
    return lines.map(line => {
        if (line.length < minIndent)
            return line;
        return line.slice(minIndent);
    }).join("\n");
}
// ─── Editor ────────────────────────────────────────────────────────────────
export class Editor {
    value = "";
    cursorPos = 0;
    theme;
    callbacks;
    focused = true;
    history = [];
    historyIndex = -1;
    prefix = "❯ ";
    borderColor;
    paddingX = 1;
    // Multi-line state
    visualLines = [];
    viewportTop = 0;
    maxVisualLines = 8; // max visual lines before scroll
    // Undo / Kill Ring
    undoStack = new UndoStack(100);
    killRing = new KillRing();
    lastKillWasAccumulate = false;
    // Track last yank position to support yank-pop text replacement
    yankStartPos = -1;
    yankLength = 0;
    // Jump mode: f{char} forward, F{char} backward
    jumpMode = null;
    // Visual selection mode (Vim-like): ctrl+v toggles, movements extend selection
    visualMode = false;
    selectionAnchor = 0; // anchor of selection (start position when visual mode began)
    // Paste markers: map of paste-id → full content
    pasteMarkerId = 0;
    pasteStorage = new Map();
    pasteMarkersInValue = new Set(); // paste-ids present in current value
    constructor(prefix = "❯ ", theme, callbacks) {
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
    setValue(value) {
        this.saveUndo();
        this.value = value;
        this.cursorPos = value.length;
        this.historyIndex = -1;
        this.killRing.breakAccumulate();
        this.pasteStorage.clear();
        this.pasteMarkersInValue.clear();
        this.callbacks.onChange?.(value);
    }
    getValue() {
        return this.value;
    }
    insertText(text) {
        this.saveUndo();
        // Auto-dedent: strip common leading whitespace from multi-line pastes
        const dedented = autoDedent(text);
        // Fold large pastes into collapse markers to avoid rendering issues
        const lineCount = (dedented.match(/\n/g) || []).length + 1;
        if (lineCount > 10 || dedented.length > 1000) {
            const id = `paste_${++this.pasteMarkerId}`;
            this.pasteStorage.set(id, dedented);
            this.pasteMarkersInValue.add(id);
            const marker = `${PASTE_MARKER_START}${id}${PASTE_MARKER_END}[Paste: ${lineCount} lines, ${dedented.length} chars]`;
            this.insertAtCursor(marker);
        }
        else {
            this.insertAtCursor(dedented);
        }
        this.killRing.breakAccumulate();
        this.callbacks.onChange?.(this.value);
    }
    /** Get value with paste markers expanded to original content. */
    getValueExpanded() {
        let result = this.value;
        // Expand paste markers: \x1b_pi:ps\x07<id>\x1b_pi:pe\x07 → stored content
        const markerRegex = new RegExp(`${escapeRegex(PASTE_MARKER_START)}(paste_\\d+)${escapeRegex(PASTE_MARKER_END)}\\[Paste: [^\\]]+\\]`, "g");
        result = result.replace(markerRegex, (_match, id) => {
            return this.pasteStorage.get(id) ?? _match;
        });
        return result;
    }
    setPrefix(prefix) {
        this.prefix = prefix;
    }
    setBorderColor(color) {
        this.borderColor = color;
    }
    setPaddingX(px) {
        this.paddingX = Math.max(0, px);
    }
    handleInput(data) {
        this.handleKey(data);
    }
    invalidate() {
        this.visualLines = [];
    }
    // ─── Text Manipulation ────────────────────────────────────────────────────
    insertAtCursor(text) {
        this.value = this.value.slice(0, this.cursorPos) + text + this.value.slice(this.cursorPos);
        this.cursorPos += text.length;
    }
    deleteRange(start, end) {
        const deleted = this.value.slice(start, end);
        this.value = this.value.slice(0, start) + this.value.slice(end);
        return deleted;
    }
    saveUndo() {
        this.undoStack.push({ value: this.value, cursorPos: this.cursorPos });
    }
    // ─── Key Handling ─────────────────────────────────────────────────────────
    handleKey(key) {
        // Clear yank tracking on any non-yank-pop operation
        if (key !== "alt+y") {
            this.yankStartPos = -1;
            this.yankLength = 0;
        }
        // Jump mode: consume next character to jump to
        if (this.jumpMode) {
            if (key.length === 1 && key.charCodeAt(0) >= 32) {
                return this.doJump(key);
            }
            this.jumpMode = null;
            return false;
        }
        // Visual mode: ctrl+v toggles, Escape/ctrl+c exits
        if (key === "ctrl+v") {
            this.toggleVisualMode();
            return true;
        }
        if (this.visualMode) {
            return this.handleVisualKey(key);
        }
        switch (key) {
            // Movement
            case "left": return this.moveLeft();
            case "right": return this.moveRight();
            case "ctrl+b": return this.moveLeft(); // pi: cursorLeft
            case "ctrl+f": return this.moveRight(); // pi: cursorRight
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
            case "delete":
            case "ctrl+d": return this.deleteForward(); // pi: deleteCharForward
            case "ctrl+h": return this.deleteBackward();
            // Kill / Yank
            case "ctrl+k": return this.killToEndOfLine();
            case "alt+d":
            case "alt+delete": return this.killWordForward(); // pi: deleteWordForward
            case "alt+backspace":
            case "ctrl+w": return this.killWordBackward();
            case "ctrl+y": return this.yank();
            case "alt+y": return this.yankPop();
            // Undo
            case "ctrl+/":
            case "ctrl+z":
            case "ctrl+_":
            case "ctrl+-": return this.undo(); // pi: undo
            case "alt+/":
            case "ctrl+shift+z": return this.redo();
            // Navigation shortcuts
            case "ctrl+a": return this.moveHome();
            case "ctrl+e": return this.moveEnd();
            case "ctrl+u": return this.killToStartOfLine();
            // Jump mode: f{char} (pi: ctrl+] forward, ctrl+alt+] backward)
            case "ctrl+]":
                this.jumpMode = "forward";
                return true;
            case "ctrl+alt+]":
                this.jumpMode = "backward";
                return true;
            // Line operations (Vim-like: dd, yy)
            case "ctrl+shift+k": return this.deleteLine();
            case "ctrl+shift+y": return this.yankLine();
            // Submit / Newline
            case "enter": return this.submit();
            case "shift+enter": return this.insertNewline();
            // Space (parseKey converts " " to "space")
            case "space": return this.insertChar(" ");
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
    // ─── Visual Mode ────────────────────────────────────────────────────────
    toggleVisualMode() {
        this.visualMode = !this.visualMode;
        if (this.visualMode) {
            this.selectionAnchor = this.cursorPos;
        }
    }
    getSelectionRange() {
        const start = Math.min(this.selectionAnchor, this.cursorPos);
        const end = Math.max(this.selectionAnchor, this.cursorPos);
        return { start, end };
    }
    handleVisualKey(key) {
        switch (key) {
            case "escape":
            case "ctrl+c":
                this.visualMode = false;
                return true;
            // Movement keys extend selection
            case "left": return this.visualMove(() => this.moveLeft());
            case "right": return this.visualMove(() => this.moveRight());
            case "up": return this.visualMove(() => this.moveUp());
            case "down": return this.visualMove(() => this.moveDown());
            case "home": return this.visualMove(() => this.moveHome());
            case "end": return this.visualMove(() => this.moveEnd());
            case "ctrl+left":
            case "alt+b": return this.visualMove(() => this.moveWordLeft());
            case "ctrl+right":
            case "alt+f": return this.visualMove(() => this.moveWordRight());
            case "ctrl+a": return this.visualMove(() => this.moveHome());
            case "ctrl+e": return this.visualMove(() => this.moveEnd());
            case "ctrl+]":
                this.jumpMode = "forward";
                return true;
            case "ctrl+alt+]":
                this.jumpMode = "backward";
                return true;
            // Operators on selection
            case "d": return this.visualDelete();
            case "y": return this.visualYank();
            case "c": return this.visualChange();
            case "x": return this.visualDelete();
            default: return false;
        }
    }
    visualMove(fn) {
        const prevPos = this.cursorPos;
        if (!fn())
            return false;
        // If jump mode was activated, move is handled by it
        if (this.jumpMode)
            return true;
        return this.cursorPos !== prevPos;
    }
    visualDelete() {
        const { start, end } = this.getSelectionRange();
        if (start === end)
            return false;
        this.saveUndo();
        const deleted = this.value.slice(start, end);
        this.value = this.value.slice(0, start) + this.value.slice(end);
        this.cursorPos = start;
        this.selectionAnchor = start;
        this.visualMode = false;
        this.killRing.kill(deleted, false);
        this.callbacks.onChange?.(this.value);
        return true;
    }
    visualYank() {
        const { start, end } = this.getSelectionRange();
        if (start === end)
            return false;
        const text = this.value.slice(start, end);
        this.killRing.kill(text, false);
        this.visualMode = false;
        return true;
    }
    visualChange() {
        const { start, end } = this.getSelectionRange();
        if (start === end)
            return false;
        this.saveUndo();
        const deleted = this.value.slice(start, end);
        this.value = this.value.slice(0, start) + this.value.slice(end);
        this.cursorPos = start;
        this.selectionAnchor = start;
        this.visualMode = false;
        this.killRing.kill(deleted, false);
        this.callbacks.onChange?.(this.value);
        return true;
    }
    // ─── Line Operations (Vim-like) ────────────────────────────────────────
    deleteLine() {
        this.saveUndo();
        // Find current line boundaries
        const lineStart = this.value.lastIndexOf("\n", this.cursorPos - 1);
        const start = lineStart === -1 ? 0 : lineStart + 1;
        const lineEnd = this.value.indexOf("\n", this.cursorPos);
        const end = lineEnd === -1 ? this.value.length : lineEnd + 1;
        const deleted = this.value.slice(start, end);
        this.value = this.value.slice(0, start) + this.value.slice(end);
        this.cursorPos = Math.min(start, this.value.length);
        this.killRing.kill(deleted, true);
        this.callbacks.onChange?.(this.value);
        return true;
    }
    yankLine() {
        const lineStart = this.value.lastIndexOf("\n", this.cursorPos - 1);
        const start = lineStart === -1 ? 0 : lineStart + 1;
        const lineEnd = this.value.indexOf("\n", this.cursorPos);
        const end = lineEnd === -1 ? this.value.length : lineEnd;
        const text = this.value.slice(start, end);
        if (text.length > 0) {
            this.killRing.kill(text, false);
        }
        return true;
    }
    // ─── Movement ─────────────────────────────────────────────────────────────
    moveLeft() {
        if (this.cursorPos > 0) {
            this.cursorPos--;
            return true;
        }
        return false;
    }
    moveRight() {
        if (this.cursorPos < this.value.length) {
            this.cursorPos++;
            return true;
        }
        return false;
    }
    moveHome() {
        this.cursorPos = 0;
        return true;
    }
    moveEnd() {
        this.cursorPos = this.value.length;
        return true;
    }
    moveUp() {
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
        if (!prevLine)
            return false;
        const colInLine = this.cursorPos - (this.visualLines[visIdx]?.startOffset ?? 0);
        this.cursorPos = Math.min(prevLine.endOffset, prevLine.startOffset + colInLine);
        return true;
    }
    moveDown() {
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
        if (!nextLine)
            return false;
        const curLine = this.visualLines[visIdx];
        const colInLine = curLine ? this.cursorPos - curLine.startOffset : 0;
        this.cursorPos = Math.min(nextLine.endOffset, nextLine.startOffset + colInLine);
        return true;
    }
    movePageUp() {
        const visIdx = this.findVisualLine(this.cursorPos);
        const target = Math.max(0, visIdx - this.maxVisualLines);
        const line = this.visualLines[target];
        if (line)
            this.cursorPos = line.startOffset;
        else
            this.cursorPos = 0;
        return true;
    }
    movePageDown() {
        const visIdx = this.findVisualLine(this.cursorPos);
        const target = Math.min(this.visualLines.length - 1, visIdx + this.maxVisualLines);
        const line = this.visualLines[target];
        if (line)
            this.cursorPos = line.endOffset;
        else
            this.cursorPos = this.value.length;
        return true;
    }
    moveWordLeft() {
        this.cursorPos = findWordLeft(this.value, this.cursorPos);
        return true;
    }
    moveWordRight() {
        this.cursorPos = findWordRight(this.value, this.cursorPos);
        return true;
    }
    /** Jump to next/prev occurrence of char. */
    doJump(char) {
        const direction = this.jumpMode;
        this.jumpMode = null;
        if (char.length === 0)
            return false;
        if (direction === "forward") {
            const idx = this.value.indexOf(char, this.cursorPos + 1);
            if (idx !== -1) {
                this.cursorPos = idx;
                return true;
            }
        }
        else if (direction === "backward") {
            const idx = this.value.lastIndexOf(char, this.cursorPos - 1);
            if (idx !== -1) {
                this.cursorPos = idx;
                return true;
            }
        }
        return false;
    }
    // ─── Editing ──────────────────────────────────────────────────────────────
    insertChar(char) {
        this.saveUndo();
        this.insertAtCursor(char);
        this.callbacks.onChange?.(this.value);
        return true;
    }
    insertNewline() {
        this.saveUndo();
        this.insertAtCursor("\n");
        this.callbacks.onChange?.(this.value);
        return true;
    }
    deleteBackward() {
        if (this.cursorPos <= 0)
            return false;
        this.saveUndo();
        this.value = this.value.slice(0, this.cursorPos - 1) + this.value.slice(this.cursorPos);
        this.cursorPos--;
        this.callbacks.onChange?.(this.value);
        return true;
    }
    deleteForward() {
        if (this.cursorPos >= this.value.length)
            return false;
        this.saveUndo();
        this.value = this.value.slice(0, this.cursorPos) + this.value.slice(this.cursorPos + 1);
        this.callbacks.onChange?.(this.value);
        return true;
    }
    // ─── Kill / Yank ──────────────────────────────────────────────────────────
    killToEndOfLine() {
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
    killToStartOfLine() {
        this.saveUndo();
        const lineStart = this.value.lastIndexOf("\n", this.cursorPos - 1);
        const start = lineStart === -1 ? 0 : lineStart + 1;
        const deleted = this.deleteRange(start, this.cursorPos);
        this.cursorPos = start;
        this.killRing.kill(deleted, false);
        this.callbacks.onChange?.(this.value);
        return true;
    }
    killWordForward() {
        this.saveUndo();
        const end = findWordRight(this.value, this.cursorPos);
        const deleted = this.deleteRange(this.cursorPos, end);
        this.killRing.kill(deleted || " ", true);
        this.callbacks.onChange?.(this.value);
        return true;
    }
    killWordBackward() {
        this.saveUndo();
        const start = findWordLeft(this.value, this.cursorPos);
        const deleted = this.deleteRange(start, this.cursorPos);
        this.cursorPos = start;
        this.killRing.kill(deleted || " ", true);
        this.callbacks.onChange?.(this.value);
        return true;
    }
    yank() {
        const text = this.killRing.yank();
        if (!text)
            return false;
        this.saveUndo();
        this.yankStartPos = this.cursorPos;
        this.insertAtCursor(text);
        this.yankLength = text.length;
        this.callbacks.onChange?.(this.value);
        return true;
    }
    yankPop() {
        if (this.yankStartPos < 0 || this.yankStartPos > this.value.length)
            return false;
        // Remove previously yanked text
        this.value = this.value.slice(0, this.yankStartPos) + this.value.slice(this.yankStartPos + this.yankLength);
        this.cursorPos = this.yankStartPos;
        // Get next entry from kill ring (don't reset — allow multiple rotations)
        const text = this.killRing.yankPop();
        if (!text)
            return false;
        this.insertAtCursor(text);
        this.yankLength = text.length;
        this.callbacks.onChange?.(this.value);
        return true;
    }
    // ─── Undo / Redo ──────────────────────────────────────────────────────────
    undo() {
        const state = this.undoStack.undo();
        if (!state)
            return false;
        this.value = state.value;
        this.cursorPos = state.cursorPos;
        this.callbacks.onChange?.(this.value);
        return true;
    }
    redo() {
        const state = this.undoStack.redo();
        if (!state)
            return false;
        this.value = state.value;
        this.cursorPos = state.cursorPos;
        this.callbacks.onChange?.(this.value);
        return true;
    }
    // ─── Submit ───────────────────────────────────────────────────────────────
    submit() {
        const expandedValue = this.getValueExpanded();
        if (expandedValue.trim()) {
            this.history.unshift(expandedValue);
            if (this.history.length > 100)
                this.history.pop();
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
    historyUp() {
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
    historyDown() {
        if (this.historyIndex > 0) {
            this.historyIndex--;
            const h = this.history[this.historyIndex];
            if (h !== undefined) {
                this.value = h;
                this.cursorPos = h.length;
            }
            this.callbacks.onChange?.(this.value);
        }
        else if (this.historyIndex === 0) {
            this.historyIndex = -1;
            this.value = "";
            this.cursorPos = 0;
            this.callbacks.onChange?.(this.value);
        }
        return true;
    }
    // ─── Visual Line Mapping ──────────────────────────────────────────────────
    findVisualLine(offset) {
        for (let i = 0; i < this.visualLines.length; i++) {
            const vl = this.visualLines[i];
            if (!vl)
                continue;
            if (offset >= vl.startOffset && offset <= vl.endOffset)
                return i;
        }
        // If beyond last line, return last
        return Math.max(0, this.visualLines.length - 1);
    }
    rebuildVisualLines(availableWidth) {
        this.visualLines = [];
        if (availableWidth <= 0)
            return;
        const prefixWidth = visibleWidth(this.prefix);
        const borderPad = this.borderColor !== undefined ? 2 : 0;
        const firstLineWidth = availableWidth - prefixWidth - this.paddingX - borderPad;
        const continuationWidth = availableWidth - this.paddingX - borderPad;
        // Split by logical newlines first, then segment each line by paste markers
        const logicalLines = this.value.split("\n");
        const allSegments = [];
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
        let currentLineChunks = [];
        let currentLineStart = 0;
        let currentLineWidth = 0;
        const lineWidth = firstVisualLine ? firstLineWidth : continuationWidth;
        const self = this;
        function flushVisualLine(endOffset) {
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
        const flushVL = (eo) => flushVisualLine(eo);
        const contW = continuationWidth;
        for (const seg of allSegments) {
            const segWidth = seg.isMarker ? 20 : visibleWidth(seg.text); // approximate marker width
            const effectiveWidth = firstVisualLine ? firstLineWidth : contW;
            if (seg.text === "" && !seg.newlineAfter) {
                // Empty segment within a line
                offset += seg.text.length;
                if (seg.newlineAfter)
                    offset++;
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
            }
            else if (seg.text.includes("\n") || seg.newlineAfter) {
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
            }
            else {
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
    render(screenWidth) {
        const lines = [];
        const borderChar = this.borderColor !== undefined
            ? `${CSI}38;5;${this.borderColor}m│${RESET}`
            : "";
        const padStr = " ".repeat(this.paddingX);
        const borderPad = this.borderColor !== undefined ? 2 : 0;
        const prefixStr = `${CSI}38;5;${this.theme.prompt}m${this.prefix}${RESET}`;
        const prefixWidth = visibleWidth(this.prefix);
        const firstLineTextWidth = screenWidth - prefixWidth - this.paddingX - borderPad;
        const contTextWidth = screenWidth - this.paddingX - borderPad;
        if (firstLineTextWidth <= 0)
            return [""];
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
        }
        else if (cursorVL >= this.viewportTop + maxVisible) {
            this.viewportTop = cursorVL - maxVisible + 1;
        }
        this.viewportTop = Math.max(0, Math.min(this.viewportTop, Math.max(0, totalVL - maxVisible)));
        const visibleEnd = Math.min(totalVL, this.viewportTop + maxVisible);
        // Top border rule (matches pi's horizontal separator above editor)
        const hrColor = this.theme.border ?? 240;
        lines.push(`${CSI}38;5;${hrColor}m${"─".repeat(screenWidth)}${RESET}`);
        // Scroll indicators
        if (this.viewportTop > 0) {
            const indicator = `${CSI}38;5;${this.theme.border ?? 240}m↑ ${this.viewportTop} more${RESET}`;
            lines.push(borderChar + indicator);
        }
        for (let vi = this.viewportTop; vi < visibleEnd; vi++) {
            const vl = this.visualLines[vi];
            if (!vl)
                continue;
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
            // Visual selection highlighting
            const selRange = this.visualMode ? this.getSelectionRange() : null;
            const selInLine = selRange && selRange.start < vl.endOffset && selRange.end > vl.startOffset;
            // Pad to fill width
            const pad = Math.max(0, textWidth - contentWidth);
            let display = linePrefix + padStr;
            if (selInLine && selRange) {
                // Build display with selection highlighted
                const selStartInLine = Math.max(0, selRange.start - vl.startOffset);
                const selEndInLine = Math.min(content.length, selRange.end - vl.startOffset);
                const beforeSel = content.slice(0, selStartInLine);
                const selText = content.slice(selStartInLine, selEndInLine);
                const afterSel = content.slice(selEndInLine);
                const fgCode = `${CSI}38;5;${this.theme.text}m`;
                const selCode = `${CSI}48;5;${this.theme.cursor}m${CSI}38;5;${this.theme.cursorText}m`;
                display += fgCode + beforeSel + RESET;
                display += selCode + selText + RESET;
                display += fgCode + afterSel + RESET;
                display += " ".repeat(pad);
            }
            else {
                display += `${CSI}38;5;${this.theme.text}m${content}${RESET}`;
                display += " ".repeat(pad);
            }
            // Render cursor in this visual line (skip in visual mode — selection shows the range)
            if (!this.visualMode && this.focused && this.cursorPos >= vl.startOffset && this.cursorPos <= vl.endOffset) {
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
            }
            else if (!this.focused && !this.visualMode) {
                display = linePrefix + padStr;
                display += `${CSI}38;5;245m${content}${RESET}`;
                display += " ".repeat(pad);
            }
            else if (!this.focused && this.visualMode) {
                // Visual mode always shows full color even when unfocused
                display = linePrefix + padStr;
                display += `${CSI}38;5;${this.theme.text}m${content}${RESET}`;
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
        // Bottom border rule (matches pi's horizontal separator below editor)
        lines.push(`${CSI}38;5;${hrColor}m${"─".repeat(screenWidth)}${RESET}`);
        return lines;
    }
}
