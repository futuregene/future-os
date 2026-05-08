package components

import (
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/bubbles/textarea"
	"github.com/charmbracelet/lipgloss"
)

// Editor wraps the bubbles textarea for user input.
type Editor struct {
	area  textarea.Model
	style lipgloss.Style
}

// NewEditor creates a new editor component.
func NewEditor(style lipgloss.Style) Editor {
	ta := textarea.New()
	ta.Placeholder = "Type a message or /command…"
	ta.ShowLineNumbers = false
	ta.CharLimit = 0 // unlimited
	ta.SetHeight(3)

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

// Value returns the current text.
func (e *Editor) Value() string {
	return e.area.Value()
}

// Empty returns true if the editor has no text.
func (e *Editor) Empty() bool {
	return strings.TrimSpace(e.area.Value()) == ""
}

// Reset clears the editor.
func (e *Editor) Reset() {
	e.area.Reset()
}

// SetValue replaces the editor content.
func (e *Editor) SetValue(s string) {
	e.area.SetValue(s)
}

// Focus focuses the editor.
func (e *Editor) Focus() tea.Cmd {
	return e.area.Focus()
}

// Update handles Bubble Tea messages.
func (e Editor) Update(msg tea.Msg) (Editor, tea.Cmd) {
	var cmd tea.Cmd
	// Handle Enter to submit
	if keyMsg, ok := msg.(tea.KeyMsg); ok {
		if keyMsg.String() == "enter" {
			if !e.Empty() {
				text := strings.TrimSpace(e.area.Value())
				e.area.Reset()
				return e, func() tea.Msg { return SubmitMsg(text) }
			}
			return e, nil
		}
		// Shift+Enter for newline
		if keyMsg.String() == "shift+enter" || keyMsg.String() == "alt+enter" {
			// Let textarea handle it
		}
	}
	e.area, cmd = e.area.Update(msg)
	return e, cmd
}

// View renders the editor.
func (e Editor) View() string {
	return e.style.Render(e.area.View())
}

// SubmitMsg is sent when the user presses Enter with text.
type SubmitMsg string
