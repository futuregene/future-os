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

// Render renders a user message entry.
// Wraps in OSC 133 zones (start/end/final).
// Uses glamour markdown rendering with wordWrap fallback.
// Applies UserMessageBg background via prefixedLineBg.
func (c *UserMessageComponent) Render(entry ChatEntry, width int) string {
	var sb strings.Builder

	sb.WriteString(osc133ZoneStart)

	rendered, err := c.base.MdRenderer.Render(entry.Content)
	if err != nil {
		sb.WriteString(prefixedLineBg(" ", wordWrap(entry.Content, width-14), width, c.base.UserMessageBg))
	} else {
		rendered = strings.TrimSuffix(rendered, "\n")
		sb.WriteString(prefixedLineBg(" ", wrapURLsOSC8(rendered), width, c.base.UserMessageBg))
	}

	sb.WriteString(osc133ZoneEnd)
	sb.WriteString(osc133ZoneFinal)

	return sb.String()
}

// Ensure UserMessageComponent implements MessageComponent.
var _ MessageComponent = (*UserMessageComponent)(nil)
