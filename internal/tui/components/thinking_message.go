package components

import (
	"strings"
)

// ThinkingMessageComponent renders "thinking" type chat entries.
// Pi-style: simple italicized dim text without emoji/border, matching the
// inline thinking appearance in pi's TUI.
type ThinkingMessageComponent struct {
	base *MessageComponentBase
}

// NewThinkingMessageComponent creates a new thinking message component.
func NewThinkingMessageComponent(base *MessageComponentBase) *ThinkingMessageComponent {
	return &ThinkingMessageComponent{base: base}
}

// Render renders a thinking block as dim italic text (pi-style).
// If HideAllThinking is enabled, shows a dim label.
// Otherwise renders as dim italicized inline text, skipping glamour markdown
// to preserve the italic styling.
func (c *ThinkingMessageComponent) Render(entry ChatEntry, width int) string {
	hideThinking := c.base.HideAllThinking != nil && *c.base.HideAllThinking
	label := "Thinking..."
	if c.base.HiddenThinkingLabel != nil && *c.base.HiddenThinkingLabel != "" {
		label = *c.base.HiddenThinkingLabel
	}

	if hideThinking {
		return c.base.ThinkingDim.Render("💭 " + label)
	}

	if entry.Expanded {
		// Pi-style: simple dim italic text, skip glamour to preserve italic
		// Leading blank line to separate from previous entry
		// Indented with one space to align with user message
		content := wordWrap(entry.Content, width-5)
		lines := strings.Split(content, "\n")
		var sb strings.Builder
		for i, line := range lines {
			if i > 0 {
				sb.WriteByte('\n')
			}
			sb.WriteString(" " + line)
		}
		return "\n" + c.base.ThinkingStyle.Render(sb.String())
	}

	return c.base.ThinkingDim.Render("💭 " + label)
}

// Ensure ThinkingMessageComponent implements MessageComponent.
var _ MessageComponent = (*ThinkingMessageComponent)(nil)
