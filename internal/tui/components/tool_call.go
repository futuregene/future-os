package components

import (
	"fmt"
	"strings"
)

// ToolCallComponent renders "tool_call" type chat entries.
// Extracted from ChatViewport.View() matching TS pi-mono's tool call display pattern.
// Handles compact read (system files), pending spinner, and expanded args.
type ToolCallComponent struct {
	base *MessageComponentBase
}

// NewToolCallComponent creates a new tool call component.
func NewToolCallComponent(base *MessageComponentBase) *ToolCallComponent {
	return &ToolCallComponent{base: base}
}

// toggleKey returns the tool toggle key string with fallback.
func (c *ToolCallComponent) toggleKey() string {
	if c.base.ToolToggleKey != nil && *c.base.ToolToggleKey != "" {
		return *c.base.ToolToggleKey
	}
	return "Ctrl+O"
}

// spinnerChar returns the current spinner animation frame character.
func (c *ToolCallComponent) spinnerChar() string {
	frame := 0
	if c.base.SpinnerFrame != nil {
		frame = *c.base.SpinnerFrame
	}
	return spinnerChars[frame%len(spinnerChars)]
}

// Render renders a tool call entry.
// Handles compact read (classifyCompactRead for system files like SKILL.md, AGENTS.md).
// Shows compact one-liner when collapsed.
// Shows tool pending spinner when executing.
// Shows args when expanded.
func (c *ToolCallComponent) Render(entry ChatEntry, width int) string {
	e := entry // mutable copy

	// Compact read rendering (TS pi-mono: classify on the fly)
	if e.CompactReadKind == "" && e.ToolName == "read" {
		kind, label := classifyCompactRead(e.ToolArgs)
		if kind != "" {
			e.CompactReadKind = kind
			e.CompactReadLabel = label
			e.Expanded = false
		}
	}

	if e.CompactReadKind != "" {
		var compactLine string
		switch e.CompactReadKind {
		case "skill":
			compactLine = c.base.CustomLabelStyle.Render("[skill] ") + e.CompactReadLabel
		case "docs":
			compactLine = c.base.ToolStyle.Render("read docs ") + e.CompactReadLabel
		case "resource":
			compactLine = c.base.ToolStyle.Render("read resource ") + e.CompactReadLabel
		}
		if lr := formatLineRange(e.ToolArgs); lr != "" {
			compactLine += c.base.WarningStyle.Render(lr)
		}
		if !e.Expanded {
			compactLine += c.base.CustomDimStyle.Render(" (" + c.toggleKey() + " to expand)")
		}

		var sb strings.Builder
		sb.WriteString(applyLineBg(compactLine, width, c.base.ToolPendingBg))
		if e.Expanded && e.ToolArgs != "" {
			sb.WriteByte('\n')
			sb.WriteString(prefixedLineBg("  args: ", wordWrap(e.ToolArgs, width-14), width, c.base.ToolPendingBg))
		}
		return sb.String()
	}

	// Standard tool call display
	line := c.base.ToolStyle.Render(toolIcon(e.ToolName) + e.ToolName)
	argsPreview := toolArgsPreview(e.ToolName, e.ToolArgs)
	if argsPreview != "" {
		line += " " + argsPreview
	}
	if e.ToolPending {
		line += c.base.ToolStyle.Render("  " + c.spinnerChar() + " Running... (Esc to cancel)")
	}

	var sb strings.Builder
	sb.WriteString(applyLineBg(line, width, c.base.ToolPendingBg))
	if e.Expanded && e.ToolArgs != "" {
		sb.WriteByte('\n')
		sb.WriteString(prefixedLineBg("  args: ", wordWrap(e.ToolArgs, width-14), width, c.base.ToolPendingBg))
	}
	return sb.String()
}

// Ensure ToolCallComponent implements MessageComponent.
var _ MessageComponent = (*ToolCallComponent)(nil)

// Keep fmt import for future use / alignment with other components.
var _ = fmt.Sprintf
