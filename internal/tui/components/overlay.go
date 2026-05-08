package components

import (
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// Overlay manages modal selectors displayed on top of the main content.
type Overlay struct {
	active  bool
	content string
	width   int
	height  int
	style   lipgloss.Style
}

// NewOverlay creates a new overlay manager.
func NewOverlay() Overlay {
	return Overlay{
		style: lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#61afef")).
			Padding(1, 2).
			Background(lipgloss.Color("#282c34")),
	}
}

// Active returns whether an overlay is currently displayed.
func (o *Overlay) Active() bool {
	return o.active
}

// Show displays a new overlay with the given content.
func (o *Overlay) Show(content string, w, h int) {
	o.active = true
	o.content = content
	o.width = w
	o.height = h
}

// Hide dismisses the current overlay.
func (o *Overlay) Hide() {
	o.active = false
	o.content = ""
}

// Update handles Bubble Tea messages for the overlay in-place.
func (o *Overlay) Update(msg tea.Msg) tea.Cmd {
	if keyMsg, ok := msg.(tea.KeyMsg); ok {
		switch keyMsg.String() {
		case "esc":
			o.active = false
			return nil
		case "enter":
			o.active = false
			return func() tea.Msg { return OverlayConfirmMsg{} }
		}
	}
	return nil
}

// View renders the overlay.
func (o Overlay) View() string {
	if !o.active {
		return ""
	}
	content := o.style.Copy().Width(o.width).Height(o.height).Render(o.content)
	return lipgloss.Place(
		o.width, o.height,
		lipgloss.Center, lipgloss.Center,
		content,
	)
}

// OverlayConfirmMsg is sent when the user confirms the overlay selection.
type OverlayConfirmMsg struct{}
