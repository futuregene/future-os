package components

import (

	"github.com/charmbracelet/bubbles/textarea"
	"github.com/charmbracelet/lipgloss"
)

// editorSnapshot captures the editor state for undo.

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
	ta.Placeholder = "Type a message... (Enter=submit, Shift+Enter=newline)"
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

