package components

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// BashExecutionComponent renders "bash" type chat entries with bordered display.
// Extracted from ChatViewport.renderBashEntry() matching TS pi-mono's BashExecutionComponent pattern.
type BashExecutionComponent struct {
	base *MessageComponentBase
}

// NewBashExecutionComponent creates a new bash execution component.
func NewBashExecutionComponent(base *MessageComponentBase) *BashExecutionComponent {
	return &BashExecutionComponent{base: base}
}

// toggleKey returns the tool toggle key string with fallback.
func (c *BashExecutionComponent) toggleKey() string {
	if c.base.ToolToggleKey != nil && *c.base.ToolToggleKey != "" {
		return *c.base.ToolToggleKey
	}
	return "Ctrl+O"
}

// spinnerChar returns the current spinner animation frame character.
func (c *BashExecutionComponent) spinnerChar() string {
	frame := 0
	if c.base.SpinnerFrame != nil {
		frame = *c.base.SpinnerFrame
	}
	return spinnerChars[frame%len(spinnerChars)]
}

// Render renders a bash execution entry with bordered display.
// DynamicBorder top + bottom, command header with "$ command",
// output lines with width-aware truncation (20 preview lines when collapsed),
// status line with running spinner, exit code, and truncation warning.
// !! exclusion → dim border.
func (c *BashExecutionComponent) Render(entry ChatEntry, width int) string {
	contentWidth := width - 6
	if contentWidth < 20 {
		contentWidth = 20
	}

	// TS pi-mono: border dim for !! (excluded)
	borderStyle := c.base.BashBorder
	headerStyle := c.base.BashHeader
	outputStyle := c.base.BashOutput
	if entry.BashExcluded {
		borderStyle = c.base.BashOutput.Copy()
	}

	var sb strings.Builder

	// DynamicBorder top (TS pi-mono: DynamicBorder at full width)
	// The viewport's PaddingLeft(2) provides the indent.
	border := borderStyle.Render(strings.Repeat("─", width))
	sb.WriteString(border)
	sb.WriteByte('\n')

	// Command header (TS pi-mono: "$ command" with 1-space indent)
	cmdDisplay := entry.BashCommand
	if lipgloss.Width(cmdDisplay) > width-4 {
		cmdDisplay = TruncateByWidth(cmdDisplay, width-7) + "..."
	}
	sb.WriteString(" " + headerStyle.Render("$ "+cmdDisplay))
	sb.WriteByte('\n')

	// Build visual (word-wrapped) lines for width-aware truncation (TS pi-mono: visual-truncate)
	previewLines := 20
	lineContentWidth := width - 4
	if lineContentWidth < 10 {
		lineContentWidth = 10
	}
	var visualLines []string
	for _, line := range entry.BashLines {
		if lipgloss.Width(line) <= lineContentWidth {
			visualLines = append(visualLines, line)
		} else {
			visualLines = append(visualLines, strings.Split(wordWrap(line, lineContentWidth), "\n")...)
		}
	}

	hiddenCount := 0
	visibleLines := visualLines
	if !entry.Expanded && len(visualLines) > previewLines {
		hiddenCount = len(visualLines) - previewLines
		visibleLines = visualLines[len(visualLines)-previewLines:]
	}

	// Empty line before output (TS pi-mono: \n before styled output)
	if len(visibleLines) > 0 {
		sb.WriteByte('\n')
	}

	// Output lines (TS pi-mono: 1-space indent, muted color)
	for _, line := range visibleLines {
		displayLine := line
		if lipgloss.Width(displayLine) > lineContentWidth {
			displayLine = TruncateByWidth(displayLine, lineContentWidth-3) + "..."
		}
		sb.WriteString(" " + outputStyle.Render(displayLine))
		sb.WriteByte('\n')
	}

	// Status line (TS pi-mono: starts with \n before status, or adds empty after output)
	hadOutput := len(visibleLines) > 0
	if hadOutput {
		sb.WriteByte('\n')
	}

	// Status line
	if entry.BashRunning {
		runningText := " " + c.spinnerChar() + " Running... (Esc to cancel)"
		sb.WriteString(outputStyle.Render(runningText))
	} else {
		cancelled := entry.BashExitCode == -1

		var statusParts []string

		// Hidden lines info
		if hiddenCount > 0 {
			if entry.Expanded {
				statusParts = append(statusParts, c.base.BashOutput.Render("("+c.toggleKey()+" to collapse)"))
			} else {
				statusParts = append(statusParts, c.base.BashOutput.Render(fmt.Sprintf("... %d more lines ("+c.toggleKey()+" to expand)", hiddenCount)))
			}
		}

		// Exit status: suppressed for exit 0 (TS pi-mono: no status for success)
		if cancelled {
			statusParts = append(statusParts, c.base.WarningStyle.Render("(cancelled)"))
		} else if entry.BashExitCode != 0 {
			statusParts = append(statusParts, c.base.BashErrorStatus.Render(fmt.Sprintf("(exit %d)", entry.BashExitCode)))
		}

		// Truncation warning inline in border (TS pi-mono)
		if entry.BashTruncated && entry.BashFullOutputPath != "" {
			statusParts = append(statusParts, c.base.WarningStyle.Render("Output truncated. Full output: "+entry.BashFullOutputPath))
		}

		if len(statusParts) > 0 {
			sb.WriteString(" " + strings.Join(statusParts, "\n "))
		}
	}
	// Always add newline before bottom border for spacing
	sb.WriteByte('\n')

	// DynamicBorder bottom (TS pi-mono: DynamicBorder at full width)
	sb.WriteString(borderStyle.Render(strings.Repeat("─", width)))
	sb.WriteByte('\n') // trailing newline for inter-entry spacing

	return sb.String()
}

// Ensure BashExecutionComponent implements MessageComponent.
var _ MessageComponent = (*BashExecutionComponent)(nil)
