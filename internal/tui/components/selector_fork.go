package components

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// viewForkMode renders the selector in fork mode with 2-line per item display (TS pi-mono: UserMessageSelector).
func (s ListSelector) viewForkMode() string {
	titleStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color(s.AccentColor)).
		Bold(true).
		PaddingBottom(0)

	descStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#6c7086"))

	helpStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370")).
		PaddingTop(1)

	noMatchStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370")).
		PaddingLeft(2)

	var sb strings.Builder

	sb.WriteString(titleStyle.Render(s.Title))
	if s.ForkDescription != "" {
		sb.WriteString(descStyle.Render(" " + s.ForkDescription))
	}
	sb.WriteByte('\n')
	sb.WriteString(titleStyle.PaddingBottom(1).Render(""))
	sb.WriteByte('\n')

	// DynamicBorder top
	borderLine := lipgloss.NewStyle().
		Foreground(lipgloss.Color(s.BorderColor)).
		Render(strings.Repeat("─", max(1, s.Width)))
	sb.WriteString(borderLine)
	sb.WriteByte('\n')

	items := s.Items
	if len(items) == 0 {
		sb.WriteString(noMatchStyle.Render("No user messages found"))
		helpText := "↑↓ navigate  Enter select  Esc cancel"
		if s.HelpText != "" {
			helpText = s.HelpText
		}
		sb.WriteString(helpStyle.Render(helpText))
		return sb.String()
	}

	// Calculate visible range
	visibleCount := s.Height - 5 // header + border + help
	if visibleCount < 3 {
		visibleCount = 3
	}
	// Each item takes 3 lines (text + metadata + blank), so adjust
	visibleItems := visibleCount / 3
	if visibleItems < 1 {
		visibleItems = 1
	}

	start := s.Selected - visibleItems/2
	if start < 0 {
		start = 0
	}
	end := start + visibleItems
	if end > len(items) {
		end = len(items)
		start = end - visibleItems
		if start < 0 {
			start = 0
		}
	}

	normalStyle := lipgloss.NewStyle()
	boldStyle := lipgloss.NewStyle().Bold(true)
	mutedStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#6c7086"))

	for i := start; i < end; i++ {
		item := items[i]
		isSelected := i == s.Selected

		// Normalize description to single line
		desc := normalizeDesc(item.Description)
		label := normalizeDesc(item.Label)

		// Line 1: cursor + message text
		cursor := "  "
		if isSelected {
			cursor = "› "
		}
		maxTextWidth := s.Width - 4
		text := label
		if lipgloss.Width(text) > maxTextWidth {
			text = TruncateByWidth(text, maxTextWidth-1) + "..."
		}
		if isSelected {
			sb.WriteString(boldStyle.Render(cursor + text))
		} else {
			sb.WriteString(normalStyle.Render(cursor + text))
		}
		sb.WriteByte('\n')

		// Line 2: metadata (position in session)
		sb.WriteString(mutedStyle.Render("  " + desc))
		sb.WriteByte('\n')

		// Blank line between messages
		sb.WriteByte('\n')
	}

	// Scroll position indicator
	if total := len(items); total > visibleItems {
		scrollStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")).
			PaddingLeft(2)
		sb.WriteString(scrollStyle.Render(fmt.Sprintf("  (%d/%d)", s.Selected+1, total)))
		sb.WriteByte('\n')
	}

	// DynamicBorder bottom
	sb.WriteString(borderLine)
	sb.WriteByte('\n')

	// Help text
	helpText := "↑↓ navigate  Enter select  Esc cancel"
	if s.HelpText != "" {
		helpText = s.HelpText
	}
	sb.WriteString(helpStyle.Render(helpText))

	return sb.String()
}
