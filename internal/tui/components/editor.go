package components

import (
	"strings"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/bubbles/textarea"
	"github.com/charmbracelet/lipgloss"
)

// Editor wraps the bubbles textarea for user input.
// Enter submits. Ctrl+J inserts a newline.
type Editor struct {
	area            textarea.Model
	style           lipgloss.Style
	slashMode       bool
	slashCandidates []string
}

// NewEditor creates a new editor component.
func NewEditor(style lipgloss.Style) Editor {
	ta := textarea.New()
	ta.Placeholder = "Type a message… (Enter=submit, Ctrl+J=newline)"
	ta.ShowLineNumbers = false
	ta.CharLimit = 0 // unlimited
	ta.SetHeight(3)

	// Remove "enter" from the textarea's newline insertion keymap.
	// We handle Enter ourselves for submit; Ctrl+J stays for newline.
	km := ta.KeyMap
	km.InsertNewline.SetKeys("ctrl+j")
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
	e.slashMode = false
}

// SetValue replaces the editor content.
func (e *Editor) SetValue(s string) {
	e.area.SetValue(s)
	e.updateSlashMode()
}

// IsSlashMode returns true when the first character of the input is "/".
func (e Editor) IsSlashMode() bool {
	return e.slashMode
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

// SetSlashCandidates sets the autocomplete candidates for slash commands.
func (e *Editor) SetSlashCandidates(candidates []string) {
	e.slashCandidates = candidates
}

// updateSlashMode checks the current value and updates the slash mode flag.
func (e *Editor) updateSlashMode() {
	val := e.area.Value()
	e.slashMode = strings.HasPrefix(val, "/")
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

	if keyMsg, ok := msg.(tea.KeyMsg); ok {
		ks := keyMsg.String()

		// Tab: slash command autocomplete
		if ks == "tab" && e.slashMode {
			if len(e.slashCandidates) > 0 {
				e.area.SetValue(e.slashCandidates[0])
				e.slashMode = false
				return e, nil
			}
			return e, nil
		}

		// Enter: submit (with backslash workaround for newline)
		if ks == "enter" {
			if !e.Empty() {
				// Backslash+Enter workaround: if text ends with '\', strip it and insert newline
				val := e.area.Value()
				if strings.HasSuffix(val, "\\") {
					e.area.SetValue(val[:len(val)-1] + "\n")
					e.updateSlashMode()
					return e, nil
				}
				text := strings.TrimSpace(e.area.Value())
				e.area.Reset()
				e.slashMode = false
				return e, func() tea.Msg { return SubmitMsg(text) }
			}
			return e, nil
		}

		// Alt+Enter: followUp (queue message, don't interrupt current stream)
		if ks == "alt+enter" {
			if !e.Empty() {
				text := strings.TrimSpace(e.area.Value())
				e.area.Reset()
				e.slashMode = false
				return e, func() tea.Msg { return FollowUpMsg(text) }
			}
			return e, nil
		}

		// Ctrl+J: textarea inserts newline natively (via keymap config)
		// Falls through to textarea.Update below
	}

	e.area, cmd = e.area.Update(msg)
	e.updateSlashMode()
	return e, cmd
}

// View renders the editor.
func (e Editor) View() string {
	return e.style.Render(e.area.View())
}

// SubmitMsg is sent when the user presses Enter with text.
type SubmitMsg string

// FollowUpMsg is sent on Alt+Enter — queues message for after agent finishes.
type FollowUpMsg string
