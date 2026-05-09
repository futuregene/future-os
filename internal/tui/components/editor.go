package components

import (
	"bytes"
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"sort"
	"strings"
	"unicode/utf8"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/bubbles/textarea"
	"github.com/charmbracelet/lipgloss"
)

// editorSnapshot captures the editor state for undo.
type editorSnapshot struct {
	value  string
	offset int // byte offset of cursor
}

// Editor wraps the bubbles textarea for user input.
// Enter submits. Ctrl+J inserts a newline.
type Editor struct {
	area            textarea.Model
	style           lipgloss.Style
	slashMode       bool
	bashMode        bool
	fileMode        bool
	symbolMode      bool
	slashCandidates []string

	// Kill ring for Emacs-style kill/yank operations
	killRing   KillRing
	lastAction string // "kill", "yank", or ""
	lastValue  string // previous value, used to diff kill operations

	// Prompt history (TS pi-mono: Up/Down to cycle submission history)
	history      []string
	historyIndex int
	historyDraft string // saved draft while navigating history

	// Character jump mode (TS pi-mono: Ctrl+] / Ctrl+Alt+])
	jumpMode    bool
	jumpForward bool

	// Paste markers (TS pi-mono: compact markers for large pastes)
	pasteStore []string
	pasteID    int

	// Sticky preferred visual column for vertical cursor movement (TS pi-mono).
	// When set, vertical movement tries to maintain this visual column.
	preferredVisualCol *int

	// Undo stack (TS pi-mono: UndoStack<EditorState>)
	undoStack    []editorSnapshot
	lastUndoType string // "insert", "delete", "" for coalescing

	// File path completion cycling (TS pi-mono: fuzzy file matching with fd)
	fileMatches      []string
	fileMatchIndex   int
	fileMatchPrefix  string
	fileMatchStart   int // byte offset of replacement start in value
	fileMatchAtSign  bool

	// Slash command cycling (TS pi-mono: Tab cycles through slash candidates)
	slashMatchIndex   int
	slashMatchCurrent []string // snapshot of current candidates for change detection

	// Border colors for different modes (TS pi-mono: mode-based border coloring)
	defaultBorderColor string // thinking-based, set by app
	bashBorderColor    string // green, set by app
	slashBorderColor   string // default, set by app
	fileBorderColor    string // yellow/amber for @ file mode
	symbolBorderColor  string // magenta/purple for # symbol mode
	matchKey          KeyMatcher
}

// SetKeyMatcher sets the keybinding matcher for user-configurable keybindings.
// When nil, the editor falls back to hardcoded defaults.
func (e *Editor) SetKeyMatcher(m KeyMatcher) {
	e.matchKey = m
}

// matches checks if a key string matches a binding, falling back to hardcoded keys.
func (e *Editor) matches(ks, binding string, hardcoded ...string) bool {
	if e.matchKey != nil {
		return e.matchKey(ks, binding)
	}
	for _, h := range hardcoded {
		if ks == h {
			return true
		}
	}
	return false
}

// NewEditor creates a new editor component.
func NewEditor(style lipgloss.Style) Editor {
	ta := textarea.New()
	ta.Placeholder = "Type a message… (Enter=submit, Shift+Enter=newline)"
	ta.ShowLineNumbers = false
	ta.CharLimit = 0 // unlimited
	ta.SetHeight(3)

	// Remove "enter" from the textarea's newline insertion keymap.
	// We handle Enter ourselves for submit; Ctrl+J and Shift+Enter insert newlines.
	km := ta.KeyMap
	km.InsertNewline.SetKeys("ctrl+j", "shift+enter")
	ta.KeyMap = km

	if style.GetWidth() == 0 {
		style = lipgloss.NewStyle().
			Border(lipgloss.NormalBorder(), true).
			BorderForeground(lipgloss.Color("#61afef")).
			Padding(0, 1)
	}

	return Editor{area: ta, style: style}
}

// SetWidth updates the editor width.
func (e *Editor) SetWidth(w int) {
	e.area.SetWidth(w)
}

// SetHeight updates the editor height (number of visible text rows).
// Follows TS pi-mono: max(5, floor(terminalRows * 0.3)).
func (e *Editor) SetHeight(terminalRows int) {
	h := terminalRows * 30 / 100
	if h < 5 {
		h = 5
	}
	e.area.SetHeight(h)
}

// SetPaddingX updates the horizontal padding of the editor.
// paddingX should be 0-3 (TS pi-mono: editor_padding setting).
func (e *Editor) SetPaddingX(paddingX int) {
	if paddingX < 0 {
		paddingX = 0
	}
	if paddingX > 3 {
		paddingX = 3
	}
	e.style = e.style.Copy().Padding(0, paddingX)
}

// Height returns the current editor height in visible rows.
func (e *Editor) Height() int {
	return e.area.Height()
}

// SetBorderColor updates the editor border color (TS pi-mono: thinking level indicator).
func (e *Editor) SetBorderColor(color string) {
	e.style = e.style.Copy().BorderForeground(lipgloss.Color(color))
	e.defaultBorderColor = color
}

// SetBashBorderColor sets the border color used when in bash mode (! prefix).
func (e *Editor) SetBashBorderColor(color string) {
	e.bashBorderColor = color
}

// SetSlashBorderColor sets the border color used when in slash mode (/ prefix).
func (e *Editor) SetSlashBorderColor(color string) {
	e.slashBorderColor = color
}

// SetFileBorderColor sets the border color used when in file mode (@ prefix).
func (e *Editor) SetFileBorderColor(color string) {
	e.fileBorderColor = color
}

// SetSymbolBorderColor sets the border color used when in symbol mode (# prefix).
func (e *Editor) SetSymbolBorderColor(color string) {
	e.symbolBorderColor = color
}

// defaultBorderColor is the thinking-based color; bashBorderColor is for ! mode; slashBorderColor for / mode.
// These are set externally by the TUI app.

// Value returns the current text.
func (e *Editor) Value() string {
	return e.area.Value()
}

// Empty returns true if the editor has no text.
func (e *Editor) Empty() bool {
	return strings.TrimSpace(e.area.Value()) == ""
}

// Reset clears the editor. Pushes an undo snapshot for recoverability.
func (e *Editor) Reset() {
	e.pushUndo("delete")
	e.area.Reset()
	e.slashMode = false
	e.bashMode = false
	e.area.Placeholder = "Type a message… (Enter=submit, Shift+Enter=newline)"
}

// SetValue replaces the editor content.
func (e *Editor) SetValue(s string) {
	e.pushUndo("delete")
	e.area.SetValue(normalizeText(s))
	e.updateSlashMode()
}

// normalizeText normalizes line endings and expands tabs to spaces (TS pi-mono).
func normalizeText(text string) string {
	text = strings.ReplaceAll(text, "\r\n", "\n")
	text = strings.ReplaceAll(text, "\r", "\n")
	text = strings.ReplaceAll(text, "\t", "    ")
	return text
}

// Paste inserts text at the current cursor position with large-paste collapsing.
// Large text is stored and replaced with a [paste #N] marker that expands on submit.
func (e *Editor) Paste(text string) {
	text = normalizeText(text)
	marker := e.StorePaste(text)
	if marker != "" {
		e.area.InsertString(marker)
	} else {
		e.area.InsertString(text)
	}
}

// IsSlashMode returns true when the first character of the input is "/".
func (e Editor) IsSlashMode() bool {
	return e.slashMode
}

// IsBashMode returns true when the first character of the input is "!".
func (e Editor) IsBashMode() bool {
	return e.bashMode
}

// IsFileMode returns true when the first character of the input is "@" (file attachment).
func (e Editor) IsFileMode() bool {
	return e.fileMode
}

// IsSymbolMode returns true when the first character of the input is "#" (symbol/tag).
func (e Editor) IsSymbolMode() bool {
	return e.symbolMode
}

// GetSymbolPrefix returns the text after the leading "#".
func (e Editor) GetSymbolPrefix() string {
	if !e.symbolMode {
		return ""
	}
	val := e.area.Value()
	if len(val) <= 1 {
		return ""
	}
	return val[1:]
}

// GetSlashPrefix returns the text after the leading "/".
func (e Editor) GetSlashPrefix() string {
	if !e.slashMode {
		return ""
	}
	val := e.area.Value()
	if len(val) <= 1 {
		return ""
	}
	return val[1:]
}

// GetFilePrefix returns the text after the leading "@" for file autocomplete.
// Handles quoted paths: @"path with spaces" → path with spaces
func (e Editor) GetFilePrefix() string {
	if !e.fileMode {
		return ""
	}
	val := e.area.Value()
	if len(val) <= 1 {
		return ""
	}
	raw := val[1:]
	// Handle quoted paths: strip leading quote, find matching close quote
	if strings.HasPrefix(raw, "\"") {
		inner := raw[1:]
		if idx := strings.Index(inner, "\""); idx >= 0 {
			return inner[:idx]
		}
		return inner
	}
	return raw
}

// GetFilePrefixRaw returns the raw prefix including quotes for display/completion.
func (e Editor) GetFilePrefixRaw() string {
	if !e.fileMode {
		return ""
	}
	val := e.area.Value()
	if len(val) <= 1 {
		return ""
	}
	return val[1:]
}

// ExitSlashMode exits slash/bash/file/symbol mode and restores the normal placeholder.
func (e *Editor) ExitSlashMode() {
	e.slashMode = false
	e.bashMode = false
	e.fileMode = false
	e.symbolMode = false
	e.area.Placeholder = "Type a message… (Enter=submit, Shift+Enter=newline)"
}

// SetSlashCandidates sets the autocomplete candidates for slash commands.
func (e *Editor) SetSlashCandidates(candidates []string) {
	// Reset cycling index when candidates change
	if !stringSlicesEqual(e.slashMatchCurrent, candidates) {
		e.slashMatchIndex = -1
		e.slashMatchCurrent = candidates
	}
	e.slashCandidates = candidates
}

// updateSlashMode checks the current value and updates the slash/bash/file/symbol mode flags.
func (e *Editor) updateSlashMode() {
	val := e.area.Value()
	e.slashMode = strings.HasPrefix(val, "/")
	e.bashMode = strings.HasPrefix(val, "!")
	// File mode: @ prefix for file attachment (TS pi-mono: @ triggers file autocomplete)
	e.fileMode = strings.HasPrefix(val, "@") && !e.slashMode && !e.bashMode && !e.symbolMode
	// Symbol mode: # prefix for symbol/tag autocomplete
	e.symbolMode = strings.HasPrefix(val, "#") && !e.slashMode && !e.bashMode && !e.fileMode

	if e.bashMode {
		e.area.Placeholder = "Run bash command… (Enter=execute, Esc=cancel)"
		if e.bashBorderColor != "" {
			e.style = e.style.Copy().BorderForeground(lipgloss.Color(e.bashBorderColor))
		}
	} else if e.slashMode {
		e.area.Placeholder = "Type a command… (Enter=run, Tab=complete)"
		if e.slashBorderColor != "" {
			e.style = e.style.Copy().BorderForeground(lipgloss.Color(e.slashBorderColor))
		}
	} else if e.fileMode {
		e.area.Placeholder = "Attach a file… (Tab=complete, Esc=cancel)"
		if e.fileBorderColor != "" {
			e.style = e.style.Copy().BorderForeground(lipgloss.Color(e.fileBorderColor))
		}
	} else if e.symbolMode {
		e.area.Placeholder = "Search symbols… (Tab=complete, Esc=cancel)"
		if e.symbolBorderColor != "" {
			e.style = e.style.Copy().BorderForeground(lipgloss.Color(e.symbolBorderColor))
		}
	} else {
		e.area.Placeholder = "Type a message… (Enter=submit, Shift+Enter=newline)"
		if e.defaultBorderColor != "" {
			e.style = e.style.Copy().BorderForeground(lipgloss.Color(e.defaultBorderColor))
		}
	}
}

// Focus focuses the editor.
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
				e.pushUndo("delete")
				e.slashMatchIndex = (e.slashMatchIndex + 1) % len(e.slashCandidates)
				e.area.SetValue(e.slashCandidates[e.slashMatchIndex])
				return e, nil
			}
			return e, nil
		}

		// Tab: file path autocomplete (not in slash/bash mode)
		if e.matches(ks, BindTab, "tab") && !e.slashMode && !e.bashMode {
			e.pushUndo("delete")
			e.tryFilePathComplete()
			e.updateSlashMode()
			return e, nil
		}

		// Kill ring: yank (Ctrl+Y)
		if e.matches(ks, BindYank, "ctrl+y") {
			e.pushUndo("delete")
			e.yank()
			e.updateSlashMode()
			return e, nil
		}

		// Kill ring: yank-pop (Alt+Y) — cycle through older kills
		if e.matches(ks, BindYankPop, "alt+y") && e.lastAction == "yank" {
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
		if e.matches(ks, BindUndo, "ctrl+_", "ctrl+/") {
			e.undo()
			e.updateSlashMode()
			return e, nil
		}

		// Delete char forward (Ctrl+D) — TS pi-mono: deleteCharForward
		if ks == "ctrl+d" {
			e.pushUndo("delete")
			e.deleteCharForward()
			e.updateSlashMode()
			return e, nil
		}

		// Clear sticky column on any non-vertical key (TS pi-mono: setCursorCol)
		if !IsVerticalKey(ks, e.matchKey) {
			e.preferredVisualCol = nil
		}

		// Save undo state before text-modifying operations
		if e.isModifyingKey(ks) {
			typ := "delete"
			if len(ks) == 1 && ks[0] >= 32 && ks[0] < 127 {
				r := rune(ks[0])
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
			e.area.Placeholder = "Jump to character…"
			return e, nil
		}
		if e.matches(ks, BindJumpBackward, "ctrl+alt+]") {
			e.jumpMode = true
			e.jumpForward = false
			e.area.Placeholder = "Jump backward to character…"
			return e, nil
		}

		// In jump mode, the next single character triggers the jump
		if e.jumpMode && len(ks) == 1 && ks[0] >= 32 && ks[0] < 127 {
			e.pushUndo("delete")
			e.charJump(ks[0], e.jumpForward)
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

		// Cursor up with preferred visual column tracking (TS pi-mono: sticky column)
		if e.matches(ks, BindCursorUp, "up") {
			if e.Empty() && len(e.history) > 0 {
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
		// Cursor down with preferred visual column tracking
		if e.matches(ks, BindCursorDown, "down") {
			if e.historyIndex > 0 {
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
				e.Reset()
				return e, func() tea.Msg { return SubmitMsg(text) }
			}
			return e, nil
		}

		// Alt+Enter: followUp (queue message, don't interrupt current stream)
		if ks == "alt+enter" {
			if !e.Empty() {
				text := strings.TrimSpace(e.area.Value())
				e.Reset()
				return e, func() tea.Msg { return FollowUpMsg(text) }
			}
			return e, nil
		}

		// Ctrl+J: textarea inserts newline natively (via keymap config)
		// Falls through to textarea.Update below
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
func (e Editor) View() string {
	view := e.style.Render(e.area.View())

	total := e.area.LineCount()
	visible := e.area.Height()
	if total <= visible {
		return view
	}

	// Approximate scroll position from cursor line (textarea auto-scrolls to cursor).
	cursorLine := e.area.Line()
	hiddenAbove := max(0, cursorLine-visible/2)
	hiddenBelow := max(0, total-cursorLine-visible/2)

	if hiddenAbove > 0 {
		indicator := fmt.Sprintf("─── ↑ %d more", hiddenAbove)
		if hiddenAbove > 1 {
			indicator += " lines"
		} else {
			indicator += " line"
		}
		view = indicator + "\n" + view
	}
	if hiddenBelow > 0 {
		indicator := fmt.Sprintf("─── ↓ %d more", hiddenBelow)
		if hiddenBelow > 1 {
			indicator += " lines"
		} else {
			indicator += " line"
		}
		view += "\n" + indicator
	}
	return view
}

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
func extractInsertedText(oldStr, newStr string) string {
	if oldStr == newStr || len(newStr) <= len(oldStr) {
		return ""
	}
	// Find common prefix
	prefixLen := 0
	minLen := len(oldStr)
	if len(newStr) < minLen {
		minLen = len(newStr)
	}
	for i := 0; i < minLen; i++ {
		if oldStr[i] == newStr[i] {
			prefixLen++
		} else {
			break
		}
	}
	// Find common suffix
	oldRemain := oldStr[prefixLen:]
	newRemain := newStr[prefixLen:]
	suffixLen := 0
	for i := 0; i < len(oldRemain) && i < len(newRemain); i++ {
		if oldRemain[len(oldRemain)-1-i] == newRemain[len(newRemain)-1-i] {
			suffixLen++
		} else {
			break
		}
	}
	inserted := newRemain
	if suffixLen > 0 {
		inserted = newRemain[:len(newRemain)-suffixLen]
	}
	return inserted
}

// extractKilledText finds the text removed between oldStr and newStr.
// Uses a simple common-prefix/suffix approach.
func extractKilledText(oldStr, newStr string) string {
	if oldStr == newStr {
		return ""
	}
	// Find common prefix
	minLen := len(oldStr)
	if len(newStr) < minLen {
		minLen = len(newStr)
	}
	prefixLen := 0
	for i := 0; i < minLen; i++ {
		if oldStr[i] == newStr[i] {
			prefixLen++
		} else {
			break
		}
	}
	// Find common suffix
	oldRemain := oldStr[prefixLen:]
	newRemain := newStr[prefixLen:]
	suffixLen := 0
	for i := 0; i < len(oldRemain) && i < len(newRemain); i++ {
		if oldRemain[len(oldRemain)-1-i] == newRemain[len(newRemain)-1-i] {
			suffixLen++
		} else {
			break
		}
	}
	killed := oldRemain
	if suffixLen > 0 {
		killed = oldRemain[:len(oldRemain)-suffixLen]
	}
	return killed
}

// tryFilePathComplete performs file path completion on the last word.
// Uses fd for fuzzy recursive search (TS pi-mono style), falls back to filepath.Glob.
// Multiple matches can be cycled with repeated Tab presses.
func (e *Editor) tryFilePathComplete() {
	val := e.area.Value()
	// Find the last word
	lastSpace := strings.LastIndexAny(val, " \t\n")
	prefix := val
	replaceStart := 0
	if lastSpace >= 0 {
		prefix = val[lastSpace+1:]
		replaceStart = lastSpace + 1
	}
	// Handle @ file attachment syntax (TS pi-mono)
	atPrefix := false
	if strings.HasPrefix(prefix, "@") {
		atPrefix = true
		prefix = prefix[1:]
	}

	// Check if we're cycling through existing matches
	samePrefix := e.fileMatchPrefix == prefix
	if samePrefix && len(e.fileMatches) > 0 {
		e.fileMatchIndex = (e.fileMatchIndex + 1) % len(e.fileMatches)
		match := e.fileMatches[e.fileMatchIndex]
		result := val[:e.fileMatchStart]
		if e.fileMatchAtSign {
			result += "@"
		}
		result += match
		e.area.SetValue(result)
		return
	}

	// Reset match state
	e.fileMatches = nil
	e.fileMatchIndex = -1
	e.fileMatchPrefix = prefix
	e.fileMatchStart = replaceStart
	e.fileMatchAtSign = atPrefix

	// Only complete if it looks like a path, OR if @-prefixed (TS pi-mono: @ triggers file completion)
	if prefix == "" {
		if !atPrefix {
			return
		}
		// @ alone: list files in CWD
		matches := FindFiles(".")
		if len(matches) > 0 {
			e.fileMatches = matches
			e.fileMatchIndex = 0
			match := e.fileMatches[0]
			result := val[:replaceStart] + "@" + match
			e.area.SetValue(result)
		}
		return
	}
	if !atPrefix && !strings.ContainsAny(prefix, "./~") && !strings.HasPrefix(prefix, "/") {
		return
	}

	matches := FindFiles(prefix)
	if len(matches) == 0 {
		return
	}

	// Sort for stable results
	e.fileMatches = matches
	e.fileMatchIndex = 0
	match := e.fileMatches[0]

	result := val[:replaceStart]
	if atPrefix {
		result += "@"
	}
	result += match
	e.area.SetValue(result)
}

// findFiles searches for files matching the given prefix using fd for fuzzy
// recursive search (TS pi-mono style). Falls back to filepath.Glob if fd is
// unavailable. Results are paths relative to CWD (or absolute if prefix is absolute).
func FindFiles(prefix string) []string {
	cwd, err := os.Getwd()
	if err != nil {
		cwd = "."
	}

	// Expand ~
	expanded := prefix
	if strings.HasPrefix(prefix, "~") {
		home, err := os.UserHomeDir()
		if err == nil {
			expanded = filepath.Join(home, prefix[1:])
		}
	}

	isAbs := strings.HasPrefix(expanded, "/")

	// Determine search dir and pattern
	var searchDir, pattern string
	if isAbs {
		searchDir = filepath.Dir(expanded)
		pattern = filepath.Base(expanded)
	} else {
		searchDir = filepath.Join(cwd, filepath.Dir(expanded))
		pattern = filepath.Base(expanded)
	}

	// Try fd first (TS pi-mono: uses fd/fdfind for fuzzy, gitignore-aware search)
	var matches []string
	if matches = tryFd(searchDir, pattern); len(matches) > 0 {
		return formatMatches(matches, searchDir, expanded, prefix, isAbs, cwd)
	}

	// Fallback: filepath.Glob (TS pi-mono: uses readdir/glob as fallback)
	globPattern := expanded + "*"
	globMatches, err := filepath.Glob(globPattern)
	if err != nil || len(globMatches) == 0 {
		return nil
	}

	return formatMatches(globMatches, searchDir, expanded, prefix, isAbs, cwd)
}

// tryFd runs fd/fdfind to find files matching a pattern in a directory.
func tryFd(dir, pattern string) []string {
	bin := "fd"
	if _, err := exec.LookPath("fdfind"); err == nil {
		bin = "fdfind"
	} else if _, err := exec.LookPath("fd"); err != nil {
		return nil
	}

	// Strip leading dot from pattern for fuzzy matching if it's a filename
	// fd does substring matching by default when using -g (glob)
	searchPattern := pattern
	if !strings.HasPrefix(pattern, ".") {
		searchPattern = pattern
	}

	// --glob for glob-like matching, --max-results for limiting,
	// --strip-cwd-prefix for relative paths, --hidden includes dotfiles
	cmd := exec.Command(bin,
		"--glob", searchPattern,
		"--max-results", "20",
		"--strip-cwd-prefix",
		"--color", "never",
		dir,
	)
	var out bytes.Buffer
	cmd.Stdout = &out
	cmd.Stderr = nil

	if err := cmd.Run(); err != nil {
		return nil
	}

	lines := strings.Split(strings.TrimSpace(out.String()), "\n")
	var results []string
	for _, line := range lines {
		line = strings.TrimSpace(line)
		if line != "" {
			results = append(results, line)
		}
	}
	return results
}

// formatMatches converts raw file matches to display-ready strings.
func formatMatches(matches []string, searchDir, expanded, prefix string, isAbs bool, cwd string) []string {
	dedup := make(map[string]bool)
	var result []string
	for _, m := range matches {
		m = strings.TrimSuffix(m, "/")
		// Convert to a path relative to what the user typed
		display := formatMatchPath(m, searchDir, expanded, prefix, isAbs, cwd)
		if display != "" && !dedup[display] {
			dedup[display] = true
			// If directory, add trailing slash
			fullPath := m
			if !isAbs {
				fullPath = filepath.Join(cwd, m)
			}
			if info, err := os.Stat(fullPath); err == nil && info.IsDir() {
				display += "/"
			}
			result = append(result, display)
		}
	}
	sort.Strings(result)
	// Limit to 10 results
	if len(result) > 10 {
		result = result[:10]
	}
	return result
}

// formatMatchPath converts a matched file path to a display string.
func formatMatchPath(match, searchDir, expanded, prefix string, isAbs bool, cwd string) string {
	if isAbs {
		return match
	}
	// If the matched path starts with cwd, make it relative
	if strings.HasPrefix(match, cwd+"/") {
		return match[len(cwd)+1:]
	}
	if strings.HasPrefix(match, cwd) {
		return match[len(cwd):]
	}
	return match
}

// charJump moves the cursor to the next/previous occurrence of target character.
func (e *Editor) charJump(target byte, forward bool) {
	val := e.area.Value()
	if val == "" {
		return
	}
	curLine := e.area.Line()
	curCol := e.area.LineInfo().CharOffset

	// Find byte offset of cursor in val
	lines := strings.Split(val, "\n")
	offset := 0
	for i := 0; i < curLine && i < len(lines); i++ {
		offset += len(lines[i]) + 1 // +1 for newline
	}
	offset += curCol
	if offset > len(val) {
		offset = len(val)
	}

	if forward {
		for i := offset + 1; i < len(val); i++ {
			if val[i] == target {
				e.moveCursorToByte(i)
				return
			}
		}
	} else {
		for i := offset - 1; i >= 0; i-- {
			if val[i] == target {
				e.moveCursorToByte(i)
				return
			}
		}
	}
}

// moveCursorToByte positions the textarea cursor at the given byte offset.
func (e *Editor) moveCursorToByte(pos int) {
	val := e.area.Value()
	targetLine := 0
	targetCol := 0
	for i := 0; i < pos && i < len(val); i++ {
		if val[i] == '\n' {
			targetLine++
			targetCol = 0
		} else {
			targetCol++
		}
	}
	curLine := e.area.Line()
	for curLine < targetLine {
		e.area.CursorDown()
		curLine++
	}
	for curLine > targetLine {
		e.area.CursorUp()
		curLine--
	}
	e.area.SetCursor(targetCol)
}

// pageScroll moves the cursor by one page height (visible editor area).
// direction: -1 for up, 1 for down (TS pi-mono).
func (e *Editor) pageScroll(direction int) {
	height := e.area.Height()
	if height <= 0 {
		height = 5
	}
	targetLine := e.area.Line() + direction*height
	totalLines := max(1, e.area.LineCount())
	if targetLine < 0 {
		targetLine = 0
	}
	if targetLine >= totalLines {
		targetLine = totalLines - 1
	}
	// Use preferred visual column if set, otherwise current column
	targetCol := e.area.LineInfo().CharOffset
	if e.preferredVisualCol != nil {
		targetCol = *e.preferredVisualCol
	}
	// Navigate to target line
	for e.area.Line() < targetLine {
		e.area.CursorDown()
	}
	for e.area.Line() > targetLine {
		e.area.CursorUp()
	}
	e.area.SetCursor(targetCol)
	e.preferredVisualCol = &targetCol
}

// moveWordBackward moves the cursor to the start of the current/previous word.
// Word boundaries are defined by whitespace characters.
func (e *Editor) moveWordBackward() {
	val := e.area.Value()
	if val == "" {
		return
	}
	curLine := e.area.Line()
	curCol := e.area.LineInfo().CharOffset

	// Compute byte offset from line/col
	offset := bytePos(val, curLine, curCol)

	// Skip whitespace backward
	for offset > 0 && isWhitespace(val[offset-1]) {
		offset--
	}
	// Skip word characters backward
	for offset > 0 && !isWhitespace(val[offset-1]) {
		offset--
	}
	e.moveCursorToByte(offset)
}

// moveWordForward moves the cursor to the start of the next word.
func (e *Editor) moveWordForward() {
	val := e.area.Value()
	if val == "" {
		return
	}
	curLine := e.area.Line()
	curCol := e.area.LineInfo().CharOffset

	offset := bytePos(val, curLine, curCol)

	// Skip current word forward
	for offset < len(val) && !isWhitespace(val[offset]) {
		offset++
	}
	// Skip whitespace forward
	for offset < len(val) && isWhitespace(val[offset]) {
		offset++
	}
	if offset > len(val) {
		offset = len(val)
	}
	e.moveCursorToByte(offset)
}

// deleteCharForward deletes the character at the cursor position (Ctrl+D).
func (e *Editor) deleteCharForward() {
	val := e.area.Value()
	if val == "" {
		return
	}
	offset := bytePos(val, e.area.Line(), e.area.LineInfo().CharOffset)
	if offset >= len(val) {
		return
	}
	_, size := utf8.DecodeRuneInString(val[offset:])
	e.area.SetValue(val[:offset] + val[offset+size:])
	e.moveCursorToByte(offset)
}

// pushUndo saves a snapshot of the current editor state before a modification.
// typ is "word" (coalesced with previous word insertions), "space" (word boundary),
// or "delete" (always new snapshot).
func (e *Editor) pushUndo(typ string) {
	snap := editorSnapshot{
		value:  e.area.Value(),
		offset: bytePos(e.area.Value(), e.area.Line(), e.area.LineInfo().CharOffset),
	}
	// Coalesce consecutive word-character insertions (fish-style undo)
	if typ == "word" && len(e.undoStack) > 0 && e.lastUndoType == "word" {
		e.undoStack[len(e.undoStack)-1] = snap
		return
	}
	e.undoStack = append(e.undoStack, snap)
	e.lastUndoType = typ
	// Cap at 100 snapshots
	if len(e.undoStack) > 100 {
		e.undoStack = e.undoStack[1:]
	}
}

// undo pops the most recent snapshot and restores the editor state.
func (e *Editor) undo() {
	if len(e.undoStack) == 0 {
		return
	}
	snap := e.undoStack[len(e.undoStack)-1]
	e.undoStack = e.undoStack[:len(e.undoStack)-1]
	e.lastUndoType = "" // reset coalescing
	e.area.SetValue(snap.value)
	if snap.offset <= len(snap.value) {
		e.moveCursorToByte(snap.offset)
	}
}

// bytePos converts a (line, column) position to a byte offset into the string.
func bytePos(val string, line, col int) int {
	curLine := 0
	curCol := 0
	for i := 0; i < len(val); i++ {
		if curLine == line && curCol >= col {
			return i
		}
		if val[i] == '\n' {
			curLine++
			curCol = 0
		} else {
			curCol++
		}
	}
	return len(val)
}

// isWhitespace returns true if b is an ASCII whitespace character.
func isWhitespace(b byte) bool {
	return b == ' ' || b == '\t' || b == '\n' || b == '\r'
}

// StorePaste stores pasted text and returns a marker like "[paste #1 +123 lines]".
// Returns empty string if the text isn't large enough to warrant a marker.
// TS pi-mono: markers created for pastes >10 lines or >1000 chars.
func (e *Editor) StorePaste(text string) string {
	lineCount := strings.Count(text, "\n") + 1
	charCount := len(text)
	if lineCount <= 10 && charCount <= 1000 {
		return "" // small enough to insert directly
	}
	e.pasteID++
	marker := fmt.Sprintf("[paste #%d +%d lines]", e.pasteID, lineCount)
	if lineCount <= 1 {
		marker = fmt.Sprintf("[paste #%d %d chars]", e.pasteID, charCount)
	}
	e.pasteStore = append(e.pasteStore, text)
	return marker
}

// ExpandPastes replaces paste markers like [paste #1 +123 lines] with stored content.
func (e *Editor) ExpandPastes(text string) string {
	for i, stored := range e.pasteStore {
		marker := fmt.Sprintf("[paste #%d", i+1)
		if idx := strings.Index(text, marker); idx >= 0 {
			// Find end of marker
			end := strings.Index(text[idx:], "]")
			if end >= 0 {
				text = text[:idx] + stored + text[idx+end+1:]
			}
		}
	}
	return text
}

// isModifyingKey returns true for keys that modify the editor text content.
// Used for undo state tracking (TS pi-mono: UndoStack snapshot before mutation).
func (e *Editor) isModifyingKey(ks string) bool {
	if e.matches(ks, BindDeleteCharBackward, "backspace") ||
		e.matches(ks, BindDeleteCharForward, "delete", "ctrl+d") ||
		e.matches(ks, BindDeleteToLineEnd, "ctrl+k") ||
		e.matches(ks, BindDeleteToLineStart, "ctrl+u") ||
		e.matches(ks, BindDeleteWordBackward, "ctrl+w", "alt+backspace") ||
		e.matches(ks, BindDeleteWordForward, "alt+d") ||
		e.matches(ks, BindSubmit, "enter") ||
		e.matches(ks, BindNewLine, "alt+enter") ||
		e.matches(ks, BindTab, "tab") ||
		ks == " " || ks == "space" {
		return true
	}
	if len(ks) == 1 {
		return true
	}
	return false
}

// RecordSubmission saves text to prompt history (TS pi-mono: Up/Down navigation).
func (e *Editor) RecordSubmission(text string) {
	// Avoid consecutive duplicates
	if len(e.history) > 0 && e.history[len(e.history)-1] == text {
		return
	}
	e.history = append(e.history, text)
	e.historyIndex = 0
	e.historyDraft = ""
	// Cap history at 100 entries
	if len(e.history) > 100 {
		e.history = e.history[len(e.history)-100:]
	}
}

func (e *Editor) historyLen() int {
	return len(e.history)
}

// stringSlicesEqual compares two string slices for equality.
func stringSlicesEqual(a, b []string) bool {
	if len(a) != len(b) {
		return false
	}
	for i := range a {
		if a[i] != b[i] {
			return false
		}
	}
	return true
}

// FollowUpMsg is sent on Alt+Enter — queues message for after agent finishes.
type FollowUpMsg string
