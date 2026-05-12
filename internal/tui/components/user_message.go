package components

import (
	"strings"
)

// UserMessageComponent renders "user_message" type chat entries.
// Extracted from ChatViewport.View() matching TS pi-mono's UserMessageComponent pattern.
// Wraps in OSC 133 semantic zones and uses glamour markdown rendering
// with a distinct background color via prefixedLineBg.
type UserMessageComponent struct {
	base *MessageComponentBase
}

// NewUserMessageComponent creates a new user message component.
func NewUserMessageComponent(base *MessageComponentBase) *UserMessageComponent {
	return &UserMessageComponent{base: base}
}

// Render renders a user message entry with distinct background.
// Skips glamour markdown rendering to avoid ANSI reset codes that
// strip the background color from the text cells.
// Adds a blank line above and below with the same background color.
func (c *UserMessageComponent) Render(entry ChatEntry, width int) string {
	var sb strings.Builder

	sb.WriteString(osc133ZoneStart)

	// Leading blank line with same background
	sb.WriteString(applyLineBg("", width-4, c.base.UserMessageBg))
	sb.WriteByte('\n')

	// Message content with background extended to full width
	// Indented with one space to not flush against the left edge
	wrapped := wordWrap(entry.Content, width-5)
	lines := strings.Split(wrapped, "\n")
	var indented strings.Builder
	for i, line := range lines {
		if i > 0 {
			indented.WriteByte('\n')
		}
		indented.WriteString(" " + line)
	}
	sb.WriteString(applyLineBg(indented.String(), width-4, c.base.UserMessageBg))

	// Trailing blank line with same background
	sb.WriteByte('\n')
	sb.WriteString(applyLineBg("", width-4, c.base.UserMessageBg))

	sb.WriteString(osc133ZoneEnd)
	sb.WriteString(osc133ZoneFinal)

	return sb.String()
}

// Ensure UserMessageComponent implements MessageComponent.
var _ MessageComponent = (*UserMessageComponent)(nil)
