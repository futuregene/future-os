package components

import (
	"strings"
)

// ThinkingMessageComponent renders "thinking" type chat entries.
// Extracted from ChatViewport.View() matching TS pi-mono's ThinkingBlock pattern.
type ThinkingMessageComponent struct {
	base *MessageComponentBase
}

// NewThinkingMessageComponent creates a new thinking message component.
func NewThinkingMessageComponent(base *MessageComponentBase) *ThinkingMessageComponent {
	return &ThinkingMessageComponent{base: base}
}

// Render renders a thinking block entry.
// If HideAllThinking is enabled, shows a dim label.
// If expanded, renders via glamour markdown (or wordWrap fallback).
// Otherwise, shows a dim label with the hidden thinking label text.
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
		rendered, err := c.base.MdRenderer.Render(entry.Content)
		if err != nil {
			return c.base.ThinkingStyle.Render("💭 " + wordWrap(entry.Content, width-10))
		}
		rendered = strings.TrimSuffix(rendered, "\n")
		return c.base.ThinkingStyle.Render("💭 " + wrapURLsOSC8(rendered))
	}

	return c.base.ThinkingDim.Render("💭 " + label)
}

// Ensure ThinkingMessageComponent implements MessageComponent.
var _ MessageComponent = (*ThinkingMessageComponent)(nil)
