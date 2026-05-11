package components

import (
	"strings"
	"unicode/utf8"

	tea "github.com/charmbracelet/bubbletea"
)

// editorSnapshot captures the editor state for undo.

func (e *Editor) Focus() tea.Cmd {
	return e.area.Focus()
}

// Update handles Bubble Tea messages.
// Enter submits. Ctrl+J inserts newline (textarea handles natively).
// Tab completes slash commands.
func (e Editor) Update(msg tea.Msg) (Editor, tea.Cmd) {
	var cmd tea.Cmd
	var ks string

	if keyMsg, ok := msg.(tea.KeyMsg); ok {
		ks = keyMsg.String()

		// Tab: slash command autocomplete — cycles through candidates
		if e.matches(ks, BindTab, "tab") && e.slashMode {
			if len(e.slashCandidates) > 0 {
				e.exitHistoryBrowse()
				e.pushUndo("delete")
				e.slashMatchIndex = (e.slashMatchIndex + 1) % len(e.slashCandidates)
				e.area.SetValue(e.slashCandidates[e.slashMatchIndex])
				return e, nil
			}
			return e, nil
		}

		// Tab: file path autocomplete (not in slash/bash mode)
		if e.matches(ks, BindTab, "tab") && !e.slashMode && !e.bashMode {
			e.exitHistoryBrowse()
			e.pushUndo("delete")
			e.tryFilePathComplete()
			e.updateSlashMode()
			return e, nil
		}

		// Kill ring: yank (Ctrl+Y)
		if e.matches(ks, BindYank, "ctrl+y") {
			e.exitHistoryBrowse()
			e.pushUndo("delete")
			e.yank()
			e.updateSlashMode()
			return e, nil
		}

		// Kill ring: yank-pop (Alt+Y) — cycle through older kills
		if e.matches(ks, BindYankPop, "alt+y") && e.lastAction == "yank" {
			e.exitHistoryBrowse()
			e.pushUndo("delete")
			e.yankPop()
			e.updateSlashMode()
			return e, nil
		}

		// Kill ring: capture killed text for accumulation
		isKillKey := e.matches(ks, BindDeleteToLineEnd, "ctrl+k") ||
			e.matches(ks, BindDeleteToLineStart, "ctrl+u") ||
			e.matches(ks, BindDeleteWordBackward, "ctrl+w", "alt+backspace") ||
			e.matches(ks, BindDeleteWordForward, "alt+d", "alt+delete")
		if isKillKey {
			e.lastValue = e.area.Value()
		}

		// Undo (Ctrl+_ / Ctrl+/): restore previous editor state from undo stack
		if e.matches(ks, BindUndo, "ctrl+_", "ctrl+/", "ctrl+-") {
			e.undo()
			e.updateSlashMode()
			return e, nil
		}

		// Delete char forward (Ctrl+D) — TS pi-mono: deleteCharForward
		if ks == "ctrl+d" {
			e.exitHistoryBrowse()
			e.pushUndo("delete")
			e.deleteCharForward()
			e.updateSlashMode()
			return e, nil
		}

		// Clear sticky column on any non-vertical key (TS pi-mono: setCursorCol)
		if !IsVerticalKey(ks, e.matchKey) {
			e.preferredVisualCol = nil
		}

		// Exit history browsing on any content modification (TS pi-mono)
		if e.historyIndex > 0 && e.isModifyingKey(ks) {
			e.exitHistoryBrowse()
		}
		// Save undo state before text-modifying operations
		if e.isModifyingKey(ks) {
			typ := "delete"
			if r, size := utf8.DecodeRuneInString(ks); size > 0 && r >= 32 && r != 127 && r != utf8.RuneError {
				if r == ' ' || r == '\t' || r == '\n' {
					typ = "space" // word boundary, always new snapshot
				} else {
					typ = "word" // coalesce consecutive word chars
				}
			}
			e.pushUndo(typ)
		}

		// Jump-to-character mode (TS pi-mono: Ctrl+] to jump forward, Ctrl+Alt+] to jump backward)
		// After pressing, the next character key triggers the jump.
		if e.matches(ks, BindJumpForward, "ctrl+]") {
			e.jumpMode = true
			e.jumpForward = true
			e.area.Placeholder = "Jump to character..."
			return e, nil
		}
		if e.matches(ks, BindJumpBackward, "ctrl+alt+]") {
			e.jumpMode = true
			e.jumpForward = false
			e.area.Placeholder = "Jump backward to character..."
			return e, nil
		}

		// In jump mode, the next single character triggers the jump
		if e.jumpMode && IsPrintableKeyString(ks) && utf8.RuneCountInString(ks) == 1 {
			e.pushUndo("delete")
			r, _ := utf8.DecodeRuneInString(ks)
			e.charJump(byte(r), e.jumpForward)
			e.jumpMode = false
			e.updateSlashMode()
			return e, nil
		}
		// Cancel jump mode on any other key
		if e.jumpMode {
			e.jumpMode = false
			e.updateSlashMode()
			// Don't consume — let the key pass through normally
		}

		// Shift+Backspace: delete char backward (TS pi-mono alias)
		if ks == "shift+backspace" {
			e.exitHistoryBrowse()
			e.lastAction = ""
			e.pushUndo("delete")
			// Simulate backspace for textarea
			e.area, cmd = e.area.Update(tea.KeyMsg{Type: tea.KeyBackspace})
			e.updateSlashMode()
			return e, cmd
		}

		// Shift+Delete: delete char forward (TS pi-mono alias)
		if ks == "shift+delete" {
			e.exitHistoryBrowse()
			e.lastAction = ""
			e.pushUndo("delete")
			e.deleteCharForward()
			e.updateSlashMode()
			return e, nil
		}

		// Word jump (TS pi-mono: Alt+Left/Right, Ctrl+Left/Right)
		if e.matches(ks, BindCursorWordLeft, "alt+left", "ctrl+left", "alt+b") {
			e.moveWordBackward()
			e.updateSlashMode()
			return e, nil
		}
		if e.matches(ks, BindCursorWordRight, "alt+right", "ctrl+right", "alt+f") {
			e.moveWordForward()
			e.updateSlashMode()
			return e, nil
		}

		// Page Up/Down: scroll by one editor viewport height (TS pi-mono)
		if e.matches(ks, BindPageUp, "pgup") {
			e.pageScroll(-1)
			e.updateSlashMode()
			return e, nil
		}
		if e.matches(ks, BindPageDown, "pgdown") {
			e.pageScroll(1)
			e.updateSlashMode()
			return e, nil
		}

		// Cursor up with preferred visual column tracking + history navigation (TS pi-mono)
		if e.matches(ks, BindCursorUp, "up") {
			if len(e.history) > 0 {
				// Enter/continue history browsing (TS pi-mono: isOnFirstVisualLine gate)
				if e.Empty() || (e.historyIndex > 0 && e.isOnFirstVisualLine()) {
					if e.historyIndex == 0 {
						e.historyDraft = e.area.Value()
					}
					if e.historyIndex < len(e.history) {
						e.historyIndex++
						idx := len(e.history) - e.historyIndex
						e.area.SetValue(e.history[idx])
						e.updateSlashMode()
						return e, nil
					}
					return e, nil
				}
				// At first visual line but not browsing — jump to line start (TS pi-mono)
				if e.isOnFirstVisualLine() {
					e.area.SetCursor(0)
					e.preferredVisualCol = nil
					e.updateSlashMode()
					return e, nil
				}
			}
			// Track preferred visual column for sticky cursor
			if e.preferredVisualCol == nil {
				col := e.area.LineInfo().CharOffset
				e.preferredVisualCol = &col
			}
			e.area.CursorUp()
			// Restore preferred column on new line
			if e.preferredVisualCol != nil {
				col := *e.preferredVisualCol
				if col > e.area.LineInfo().CharOffset {
					e.area.SetCursor(col)
				}
			}
			e.updateSlashMode()
			return e, nil
		}
		// Cursor down with preferred visual column tracking + history navigation (TS pi-mono)
		if e.matches(ks, BindCursorDown, "down") {
			if len(e.history) > 0 {
				// Continue history browsing forward (TS pi-mono: isOnLastVisualLine gate)
				if e.historyIndex > 0 && e.isOnLastVisualLine() {
					e.historyIndex--
					if e.historyIndex == 0 {
						e.area.SetValue(e.historyDraft)
						e.historyDraft = ""
					} else {
						idx := len(e.history) - e.historyIndex
						e.area.SetValue(e.history[idx])
					}
					e.updateSlashMode()
					return e, nil
				}
				// At last visual line but not browsing — jump to line end (TS pi-mono)
				if e.isOnLastVisualLine() {
					lineLen := len(e.currentLine())
					e.area.SetCursor(lineLen)
					e.preferredVisualCol = nil
					e.updateSlashMode()
					return e, nil
				}
			}
			if e.preferredVisualCol == nil {
				col := e.area.LineInfo().CharOffset
				e.preferredVisualCol = &col
			}
			e.area.CursorDown()
			if e.preferredVisualCol != nil {
				col := *e.preferredVisualCol
				if col > e.area.LineInfo().CharOffset {
					e.area.SetCursor(col)
				}
			}
			e.updateSlashMode()
			return e, nil
		}

		// Shift+Space: insert a regular space character (TS pi-mono)
		if ks == "shift+space" {
			e.pushUndo("space")
			e.area.InsertString(" ")
			e.updateSlashMode()
			return e, nil
		}

		// Enter: submit (with backslash workaround for newline)
		if e.matches(ks, BindSubmit, "enter") {
			if e.DisableSubmit {
				return e, nil
			}
			if !e.Empty() {
				// Backslash+Enter workaround: if text ends with '\', strip it and insert newline
				val := e.area.Value()
				if strings.HasSuffix(val, "\\") {
					e.pushUndo("delete")
					e.area.SetValue(val[:len(val)-1] + "\n")
					e.updateSlashMode()
					return e, nil
				}
				text := strings.TrimSpace(e.area.Value())
				text = e.ExpandPastes(text)
				e.Reset()
				return e, func() tea.Msg { return SubmitMsg(text) }
			}
			return e, nil
		}

		// Alt+Enter: followUp (queue message, don't interrupt current stream)
		if ks == "alt+enter" {
			if !e.Empty() {
				text := strings.TrimSpace(e.area.Value())
				text = e.ExpandPastes(text)
				e.Reset()
				return e, func() tea.Msg { return FollowUpMsg(text) }
			}
			return e, nil
		}

		// Ctrl+J: textarea inserts newline natively (via keymap config)
		// Falls through to textarea.Update below
	}

	// Grapheme-aware backspace: delete the last grapheme cluster before cursor
	if ks == "backspace" {
		e.exitHistoryBrowse()
		e.lastAction = ""
		e.pushUndo("delete")
		e.graphemeBackspace()
		e.updateSlashMode()
		return e, nil
	}

	e.area, cmd = e.area.Update(msg)
	// Kill ring: detect what was deleted by the kill operation
	if e.lastValue != "" {
		newVal := e.area.Value()
		if newVal != e.lastValue {
			killed := extractKilledText(e.lastValue, newVal)
			if killed != "" {
				prepend := ks == "ctrl+u" || ks == "ctrl+w" || ks == "alt+backspace"
				e.killRing.Push(killed, prepend, e.lastAction == "kill")
				e.lastAction = "kill"
			}
		}
		e.lastValue = ""
	} else if e.lastAction != "yank" {
		// Reset lastAction on non-kill, non-yank actions
		e.lastAction = ""
	}
	e.updateSlashMode()
	return e, cmd
}

// View renders the editor with scroll indicators when content exceeds visible area.
// TS pi-mono style: shows "─── ↑ N more" when scrolled, "─── ↓ N more" when more below.

// SubmitMsg is sent when the user presses Enter with text.
type SubmitMsg string

// yank inserts the most recent kill ring entry at the cursor position.
func (e *Editor) yank() {
	text := e.killRing.Peek()
	if text == "" {
		return
	}
	e.area.InsertString(text)
	e.lastAction = "yank"
}

// yankPop replaces the last yanked text with the previous kill ring entry.
func (e *Editor) yankPop() {
	if e.killRing.Len() < 2 {
		return
	}
	// Undo the last yank by removing the peek() text from the value
	lastYank := e.killRing.Peek()
	val := e.area.Value()
	if strings.HasSuffix(val, lastYank) {
		e.area.SetValue(val[:len(val)-len(lastYank)])
	} else if strings.Contains(val, lastYank) {
		// Try to find and remove the last occurrence
		idx := strings.LastIndex(val, lastYank)
		if idx >= 0 {
			e.area.SetValue(val[:idx] + val[idx+len(lastYank):])
		}
	}
	e.killRing.Rotate()
	e.yank() // yank the previous entry
}

// extractInsertedText finds what was inserted between oldStr and newStr.
// This is the inverse of extractKilledText — finds text that was added.

