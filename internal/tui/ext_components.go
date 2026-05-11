// Package tui provides Bubble Tea component interfaces for extension UI customization.
// Extensions can replace the header, footer, and editor with custom implementations.
package tui

import tea "github.com/charmbracelet/bubbletea"

// ─── HeaderComponent ──────────────────────────────────────────────────────────

// HeaderFactory creates a new HeaderComponent instance.
// Called each time the component needs to be (re)instantiated in the TUI.
type HeaderFactory func() HeaderComponent

// HeaderComponent allows an extension to replace the top-of-screen header.
type HeaderComponent interface {
	// tea.Model methods: Init, Update(msg) returns (nextModel, cmd), View returns rendered output.
	tea.Model

	// SetWidth informs the component of the current terminal width in columns.
	SetWidth(w int)

	// Height returns the number of visual rows this header occupies.
	Height() int
}

// ─── FooterComponent ──────────────────────────────────────────────────────────

// FooterFactory creates a new FooterComponent instance.
type FooterFactory func() FooterComponent

// FooterComponent allows an extension to replace the bottom-of-screen footer.
type FooterComponent interface {
	tea.Model

	// SetWidth informs the component of the current terminal width in columns.
	SetWidth(w int)
}

// ─── EditorComponent ──────────────────────────────────────────────────────────

// EditorFactory creates a new EditorComponent instance.
type EditorFactory func() EditorComponent

// EditorComponent allows an extension to replace the main input editor.
// The editor handles all keyboard input when active.
type EditorComponent interface {
	tea.Model

	// Value returns the current text content.
	Value() string

	// SetValue replaces the entire editor text.
	SetValue(v string)

	// Reset clears the editor content.
	Reset()

	// Focus sets focus on the editor. Returns a command to trigger the focus.
	Focus() tea.Cmd

	// Blur removes focus from the editor.
	Blur()

	// SetWidth sets the editor width in columns.
	SetWidth(w int)

	// SetHeight calculates and sets the editor height based on terminal rows.
	SetHeight(terminalRows int)

	// Height returns the current editor height in visible rows.
	Height() int

	// Empty returns true if the editor has no text content.
	Empty() bool
}

// ─── Extension Component Message Types ────────────────────────────────────────

// extSetFooterMsg is sent by the extension bridge to replace the footer component.
type extSetFooterMsg struct {
	factory FooterFactory // nil to restore default footer
}

// extSetHeaderMsg is sent by the extension bridge to replace the header component.
type extSetHeaderMsg struct {
	factory HeaderFactory // nil to restore default header
}

// extSetEditorMsg is sent by the extension bridge to replace the editor component.
type extSetEditorMsg struct {
	factory EditorFactory // nil to restore default editor
}

// extGetEditorMsg is sent to query the current editor factory.
type extGetEditorMsg struct {
	respCh chan EditorFactory
}
