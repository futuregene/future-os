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
	hints    string

	// Dynamic expanded-mode key hints (set by app from keybindings)
	expNav   string
	expEdit  string
	expActs  string
	expMsgs  string
	expMore  string
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

// SetHints sets the compact-mode key hint string displayed below the logo.
// The string should be formatted externally using actual keybinding values, e.g.:
// "Esc interrupt  Ctrl+C clear  / commands  ! bash  Ctrl+H help  Ctrl+O tools"
func (h *Header) SetHints(hints string) {
	h.hints = hints
}

// SetExpandedHints sets the keybinding-aware hint lines for the expanded header.
// Each parameter is a pre-formatted string with actual keybinding values.
func (h *Header) SetExpandedHints(nav, edit, acts, msgs, more string) {
	h.expNav = nav
	h.expEdit = edit
	h.expActs = acts
	h.expMsgs = msgs
	h.expMore = more
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
	hintText := h.hints
	if hintText == "" {
		hintText = "Esc interrupt  Ctrl+C clear  / commands  ! bash  Ctrl+H help  Ctrl+O tools"
	}
	hints := dim.Render(hintText)
	return lipgloss.JoinVertical(lipgloss.Top, logo+"  "+hints, divider)
}

func (h Header) renderExpanded(accent, dim lipgloss.Style, divider string) string {
	nav := h.expNav
	if nav == "" {
		nav = "arrows cursor  PgUp/PgDn scroll  Home/End  gg top  G bottom"
	}
	edit := h.expEdit
	if edit == "" {
		edit = "Enter submit  Shift+Enter newline  Tab complete  Ctrl+Y yank  Alt+Y yank-pop  Ctrl+_ undo"
	}
	acts := h.expActs
	if acts == "" {
		acts = "Esc interrupt  Ctrl+C clear  Ctrl+D exit  Ctrl+Z suspend  Ctrl+L model  Ctrl+G edit  Ctrl+O tools  Ctrl+T thinking"
	}
	msgs := h.expMsgs
	if msgs == "" {
		msgs = "Alt+Enter queue follow-up  Alt+Up dequeue"
	}
	more := h.expMore
	if more == "" {
		more = "/fork  /tree  /settings  /theme  /session info  Ctrl+H collapse"
	}

	var sb strings.Builder
	sb.WriteString(accent.Render("xihu") + dim.Render(" — AI coding assistant"))
	sb.WriteByte('\n')
	sb.WriteString(dim.Render("Nav:   ") + nav)
	sb.WriteByte('\n')
	sb.WriteString(dim.Render("Edit:  ") + edit)
	sb.WriteByte('\n')
	sb.WriteString(dim.Render("Acts:  ") + acts)
	sb.WriteByte('\n')
	sb.WriteString(dim.Render("Msgs:  ") + msgs)
	sb.WriteByte('\n')
	sb.WriteString(dim.Render("More:  ") + more)
	sb.WriteByte('\n')
	sb.WriteString(divider)
	return sb.String()
}
