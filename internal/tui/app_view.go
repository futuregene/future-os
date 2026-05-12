// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"

	"github.com/huichen/xihu/internal/tui/components"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m AppModel) View() string {
	if m.quitting {
		return "Goodbye.\n"
	}

	chatView := m.chat.View()
	headerView := m.renderHeader()
	inputView := m.renderEditor()
	footerView := m.renderFooter()

	// Show pending messages indicator (TS pi-mono: pending messages section)
	pendingView := m.pendingView()

	// Transient status container (TS pi-mono: statusContainer)
	statusView := m.statusLine()

	// Filter empty widgets to avoid blank lines in layout
	widgetAboveView := m.widgetsView(m.widgetsAbove)
	widgetBelowView := m.widgetsView(m.widgetsBelow)

	// Build main layout, skipping empty widget sections
	var mainParts []string
	mainParts = append(mainParts, headerView, chatView, pendingView, statusView)
	if widgetAboveView != "" {
		mainParts = append(mainParts, widgetAboveView)
	}
	mainParts = append(mainParts, inputView)
	if widgetBelowView != "" {
		mainParts = append(mainParts, widgetBelowView)
	}
	mainParts = append(mainParts, footerView)

	main := lipgloss.JoinVertical(lipgloss.Top, mainParts...)

	var result string

	// Show autocomplete popover above the input
	if m.autocomplete.Active() {
		acView := m.autocomplete.View()
		var acParts []string
		acParts = append(acParts, headerView, chatView, pendingView, statusView)
		if widgetAboveView != "" {
			acParts = append(acParts, widgetAboveView)
		}
		acParts = append(acParts, acView, inputView, footerView)
		result = lipgloss.JoinVertical(lipgloss.Top, acParts...)
	} else if m.overlay.Active() {
		// Show overlay (modal or non-capturing)
		overlayView := m.overlay.View()
		if m.overlay.NonCapturing() {
			result = lipgloss.JoinVertical(lipgloss.Top, overlayView, main)
		} else {
			anchor := m.overlay.Anchor()
			hPos, vPos := components.AnchorToLipgloss(anchor)
			result = lipgloss.Place(
				m.width, m.height,
				hPos, vPos,
				overlayView,
				lipgloss.WithWhitespaceChars(" "),
				lipgloss.WithWhitespaceForeground(lipgloss.Color("#000000")),
			)
		}
	} else {
		result = main
	}

	// TUI write log for debugging (TS pi-mono: PI_TUI_WRITE_LOG)
	if m.writeLogFile != nil {
		m.writeLogFile.WriteString(result)
		m.writeLogFile.WriteString("\n---\n")
	}

	return result
}

// renderHeader returns the rendered header, using custom component if set.
func (m *AppModel) renderHeader() string {
	if m.customHeader != nil {
		return m.customHeader.View()
	}
	return m.header.View()
}

// renderFooter returns the rendered footer, using custom component if set.
func (m *AppModel) renderFooter() string {
	if m.customFooter != nil {
		return m.customFooter.View()
	}
	return m.footer.View()
}

// renderEditor returns the rendered editor, using custom component if set.
func (m *AppModel) renderEditor() string {
	if m.customEditor != nil {
		return m.customEditor.View()
	}
	return m.input.View()
}

// editorHeight returns the current editor height in rows.
func (m *AppModel) editorHeight() int {
	if m.customEditor != nil {
		return m.customEditor.Height()
	}
	return m.input.Height()
}

// editorEmpty returns true when the editor has no content.
func (m *AppModel) editorEmpty() bool {
	if m.customEditor != nil {
		return m.customEditor.Empty()
	}
	return m.input.Empty()
}

// footerHeight returns the current footer height in rows.
// Default footer is 3 lines (2 info lines + optional extension line).
// Custom footer components report their own height.
func (m *AppModel) footerHeight() int {
	if m.customFooter != nil {
		// Custom footer View() height — we count newlines + 1
		view := m.customFooter.View()
		lines := 0
		for _, r := range view {
			if r == '\n' {
				lines++
			}
		}
		if view != "" {
			lines++
		}
		return lines
	}
	if len(m.extensionStatuses) > 0 {
		return 3
	}
	return 2
}

// ─── Pending Messages ──────────────────────────────────────────────────────


// truncateToVisualWidth truncates a string to maxWidth visual columns,
// adding "..." if truncated. Uses rune-level truncation.
func truncateToVisualWidth(s string, maxWidth int) string {
	runes := []rune(s)
	width := 0
	for i, r := range runes {
		w := 1
		// East Asian wide chars and emoji
		if r >= 0x1100 && (r <= 0x115f || r == 0x2329 || r == 0x232a ||
			(r >= 0x2e80 && r <= 0xa4cf) || (r >= 0xac00 && r <= 0xd7a3) ||
			(r >= 0xf900 && r <= 0xfaff) || (r >= 0xfe10 && r <= 0xfe19) ||
			(r >= 0xfe30 && r <= 0xfe6f) || (r >= 0xff01 && r <= 0xff60) ||
			(r >= 0xffe0 && r <= 0xffe6) || (r >= 0x1f300 && r <= 0x1f9ff)) {
			w = 2
		}
		if width+w > maxWidth {
			return string(runes[:i]) + "..."
		}
		width += w
	}
	return s
}
// pendingView renders individual queued messages (TS pi-mono updatePendingMessagesDisplay).
// Shows Steering:/Follow-up: prefixes, width-aware truncation, and dequeue hint.
func (m *AppModel) pendingView() string {
	steeringMsgs := m.pendingSteeringMsgs
	followUpMsgs := m.pendingFollowUpMsgs

	// Include compaction queued messages as steering (pi-mono style)
	allSteering := make([]string, 0, len(steeringMsgs)+len(m.compactionQueue))
	allSteering = append(allSteering, steeringMsgs...)
	allSteering = append(allSteering, m.compactionQueue...)

	if len(allSteering) == 0 && len(followUpMsgs) == 0 {
		return ""
	}

	dimStyle := lipgloss.NewStyle().Faint(true)

	var sb strings.Builder
	for _, msg := range allSteering {
		sb.WriteString(dimStyle.Render("Steering: " + truncateToVisualWidth(msg, 80)))
		sb.WriteString("\n")
	}
	for _, msg := range followUpMsgs {
		sb.WriteString(dimStyle.Render("Follow-up: " + truncateToVisualWidth(msg, 80)))
		sb.WriteString("\n")
	}
	// Dequeue hint: ↳ Alt+Up to edit all queued messages
	sb.WriteString(dimStyle.Render("↳ Alt+Up to edit all queued messages"))

	return sb.String()
}

// statusLine renders the transient status container between chat and editor
// (TS pi-mono: statusContainer — shows Working.../compaction/retry progress).
func (m *AppModel) statusLine() string {
	mutedStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Muted))

	if m.compacting {
		spinnerStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Accent))
		frames := []string{"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"}
		frame := frames[m.spinnerFrame%len(frames)]
		return spinnerStyle.Render("  "+frame+" ") + mutedStyle.Render("Compacting context... (Esc to cancel)")
	}
	if m.retryTicking {
		spinnerStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Accent))
		frames := []string{"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"}
		frame := frames[m.spinnerFrame%len(frames)]
		msg := fmt.Sprintf("Retrying (%d/%d) in %ds... (Esc to cancel)",
			m.retryAttempt, m.retryMaxAttempts, m.retryDelaySec)
		return spinnerStyle.Render("  "+frame+" ") + mutedStyle.Render(msg)
	}
	if m.streaming && m.workingVisible {
		msg := m.workingMessage
		if msg == "" {
			msg = "Working..."
		}
		// Pi-style: accent spinner + muted message
		frames := []string{"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"}
		if len(m.workingFrames) > 0 {
			frames = m.workingFrames
		}
		frame := frames[m.spinnerFrame%len(frames)]
		spinnerStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Accent))
		return spinnerStyle.Render("  "+frame+" ") + mutedStyle.Render(msg+" (Esc to interrupt)")
	}
	return ""
}

// widgetsView renders extension widgets as text lines.
func (m *AppModel) widgetsView(widgets map[string]string) string {
	if len(widgets) == 0 {
		return ""
	}
	widgetStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#c678dd")).
		PaddingLeft(2)
	var sb strings.Builder
	for _, text := range widgets {
		sb.WriteString(widgetStyle.Render("│ " + text))
		sb.WriteByte('\n')
	}
	return sb.String()
}

// ─── Startup Banner ────────────────────────────────────────────────────────

// formatKeyStr returns the first key for a binding, or empty string.
func formatKeyStr(kb *KeybindingsManager, binding KeybindingID) string {
	if kb == nil {
		return ""
	}
	keys := kb.GetKeys(binding)
	if len(keys) > 0 {
		return keys[0]
	}
	return ""
}

