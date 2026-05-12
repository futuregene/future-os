package components

import (
	"regexp"
	"strings"
	"unicode"

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

	// DisableSubmit prevents Enter from submitting (TS pi-mono: disableSubmit).
	// Useful when an extension or overlay is handling input.
	DisableSubmit bool

	// Border colors for different modes (TS pi-mono: mode-based border coloring)
	defaultBorderColor string // thinking-based, set by app
	bashBorderColor    string // green, set by app
	slashBorderColor   string // default, set by app
	fileBorderColor    string // yellow/amber for @ file mode
	symbolBorderColor  string // magenta/purple for # symbol mode
	matchKey          KeyMatcher
}

// SetKeyMatcher sets the keybinding matcher for user-configurable keybindings.
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
	e.area.Placeholder = "Message..."
}

// SetValue replaces the editor content.
func (e *Editor) SetValue(s string) {
	e.pushUndo("delete")
	e.area.SetValue(normalizeText(s))
	e.updateSlashMode()
}

// normalizeText normalizes line endings and expands tabs to spaces (TS pi-mono).
// csiUDecodeRe matches CSI-u encoded control characters (e.g. \x1b[106;5u → j).
// Tmux popups with extended-keys-format=csi-u re-encode control bytes inside
// bracketed paste as these sequences. We decode them back so the per-char filter
// can preserve newlines instead of leaking the printable tail into the editor.
var csiUDecodeRe = regexp.MustCompile(`\x1b\[(\d+);5u`)

func normalizeText(text string) string {
	text = strings.ReplaceAll(text, "\r\n", "\n")
	text = strings.ReplaceAll(text, "\r", "\n")
	text = strings.ReplaceAll(text, "\t", "    ")
	return text
}

// cleanPasteText prepares pasted text: CSI-u decode, normalize, strip non-printables.
// TS pi-mono: handlePaste decoding & filtering pipeline.
func cleanPasteText(text string) string {
	// Decode CSI-u control characters (TS pi-mono: tmux csi-u workaround)
	text = csiUDecodeRe.ReplaceAllStringFunc(text, func(m string) string {
		matches := csiUDecodeRe.FindStringSubmatch(m)
		if len(matches) < 2 {
			return m
		}
		cp := 0
		for _, c := range matches[1] {
			cp = cp*10 + int(c-'0')
		}
		if cp >= 97 && cp <= 122 {
			return string(rune(cp - 96))
		}
		if cp >= 65 && cp <= 90 {
			return string(rune(cp - 64))
		}
		return m
	})

	text = normalizeText(text)

	// Filter non-printable characters except newlines (TS pi-mono)
	var b strings.Builder
	for _, r := range text {
		if r == '\n' || r >= 32 {
			b.WriteRune(r)
		}
	}
	return b.String()
}

// Paste inserts text at the current cursor position with large-paste collapsing.
// Large text is stored and replaced with a [paste #N] marker that expands on submit.
func (e *Editor) Paste(text string) {
	// Exit history browsing, push undo, reset state (TS pi-mono: handlePaste)
	e.exitHistoryBrowse()
	e.pushUndo("paste")
	e.lastAction = ""

	text = cleanPasteText(text)

	// Auto-prepend space when pasting a file path after a word char (TS pi-mono)
	if len(text) > 0 && (text[0] == '/' || text[0] == '~' || text[0] == '.') {
		val := e.area.Value()
		lines := strings.Split(val, "\n")
		curLine := e.area.Line()
		if curLine < len(lines) {
			col := e.area.LineInfo().CharOffset
			if col > 0 && col <= len(lines[curLine]) {
				charBefore := rune(lines[curLine][col-1])
				if unicode.IsLetter(charBefore) || unicode.IsDigit(charBefore) || charBefore == '_' {
					text = " " + text
				}
			}
		}
	}

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
	e.area.Placeholder = "Message..."
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
		e.area.Placeholder = "Run bash command... (Enter=execute, Esc=cancel)"
		if e.bashBorderColor != "" {
			e.style = e.style.Copy().BorderForeground(lipgloss.Color(e.bashBorderColor))
		}
	} else if e.slashMode {
		e.area.Placeholder = "Type a command... (Enter=run, Tab=complete)"
		if e.slashBorderColor != "" {
			e.style = e.style.Copy().BorderForeground(lipgloss.Color(e.slashBorderColor))
		}
	} else if e.fileMode {
		e.area.Placeholder = "Attach a file... (Tab=complete, Esc=cancel)"
		if e.fileBorderColor != "" {
			e.style = e.style.Copy().BorderForeground(lipgloss.Color(e.fileBorderColor))
		}
	} else if e.symbolMode {
		e.area.Placeholder = "Search symbols... (Tab=complete, Esc=cancel)"
		if e.symbolBorderColor != "" {
			e.style = e.style.Copy().BorderForeground(lipgloss.Color(e.symbolBorderColor))
		}
	} else {
		e.area.Placeholder = "Message..."
		if e.defaultBorderColor != "" {
			e.style = e.style.Copy().BorderForeground(lipgloss.Color(e.defaultBorderColor))
		}
	}
}

// Focus focuses the editor.

func (e Editor) View() string {
	view := e.area.View()
	// Only show first line (TS pi-mono: single-line inline prompt)
	if idx := strings.Index(view, "\n"); idx >= 0 {
		view = view[:idx]
	}
	// Pi-style: horizontal rule lines above and below editor, full width
	w := e.area.Width()
	if w <= 0 {
		w = 80
	}
	rule := lipgloss.NewStyle().Foreground(lipgloss.Color("#585b70")).Render(strings.Repeat("─", w))
	return rule + "\n" + view + "\n" + rule
}
