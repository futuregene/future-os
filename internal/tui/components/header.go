package components

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// Header renders the top spacer container above the chat.
// In TS pi-mono, the headerContainer holds (Spacer + builtInHeader + Spacer).
// Welcome text lives inside the chat viewport (ExpandableText). This component
// only draws the spacer lines.
type Header struct {
	width int
}

// NewHeader creates a new Header component.
func NewHeader(accentColor, version string) Header {
	return Header{}
}

// SetWidth updates the header width.
func (h *Header) SetWidth(w int) {
	h.width = w
}

// View renders the header spacer lines (TS pi-mono: headerContainer spacers).
func (h Header) View() string {
	if h.width <= 0 {
		return ""
	}
	dividerStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#3e4452"))
	divider := dividerStyle.Render(strings.Repeat("─", h.width))
	return lipgloss.JoinVertical(lipgloss.Top, divider, " ", divider)
}
