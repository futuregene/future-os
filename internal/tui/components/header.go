package components

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// Header shows a compact status bar above the chat.
type Header struct {
	width    int
	expanded bool
	accent   string
}

// NewHeader creates a new Header component.
func NewHeader(accentColor string) Header {
	return Header{accent: accentColor}
}

// SetWidth updates the header width.
func (h *Header) SetWidth(w int) {
	h.width = w
}

// Toggle toggles expanded mode.
func (h *Header) Toggle() {
	h.expanded = !h.expanded
}

// Expanded returns whether the header is in expanded mode.
func (h Header) Expanded() bool {
	return h.expanded
}

// View renders the header.
func (h Header) View() string {
	if h.width <= 0 {
		return ""
	}

	accentStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color(h.accent)).
		Bold(true)
	dimStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370")).
		Faint(true)
	dividerStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#3e4452"))

	divider := dividerStyle.Render(strings.Repeat("─", h.width))

	if h.expanded {
		return h.renderExpanded(accentStyle, dimStyle, divider)
	}
	return h.renderCompact(accentStyle, dimStyle, divider)
}

func (h Header) renderCompact(accent, dim lipgloss.Style, divider string) string {
	logo := accent.Render("xihu")
	hints := dim.Render("Esc interrupt  Ctrl+C clear  / commands  ! bash  Ctrl+H help  Ctrl+O tools")
	return lipgloss.JoinVertical(lipgloss.Top, logo+"  "+hints, divider)
}

func (h Header) renderExpanded(accent, dim lipgloss.Style, divider string) string {
	var sb strings.Builder
	sb.WriteString(accent.Render("xihu") + dim.Render(" — AI coding assistant"))
	sb.WriteByte('\n')
	sb.WriteString(dim.Render("Nav:   ") + "arrows cursor  PgUp/PgDn scroll  Home/End  gg top  G bottom")
	sb.WriteByte('\n')
	sb.WriteString(dim.Render("Edit:  ") + "Enter submit  Shift+Enter newline  Tab complete  Ctrl+Y yank  Alt+Y yank-pop  Ctrl+_ undo")
	sb.WriteByte('\n')
	sb.WriteString(dim.Render("Acts:  ") + "Esc interrupt  Ctrl+C clear  Ctrl+D exit  Ctrl+Z suspend  Ctrl+L model  Ctrl+G edit  Ctrl+O tools  Ctrl+T thinking")
	sb.WriteByte('\n')
	sb.WriteString(dim.Render("Msgs:  ") + "Alt+Enter queue follow-up  Alt+Up dequeue")
	sb.WriteByte('\n')
	sb.WriteString(dim.Render("More:  ") + "/fork  /tree  /settings  /theme  /session info  Ctrl+H collapse")
	sb.WriteByte('\n')
	sb.WriteString(divider)
	return sb.String()
}
