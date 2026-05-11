package components

import (
	"strings"
)

// AssistantMessageComponent renders "text" type chat entries (assistant responses).
// Extracted from ChatViewport.View() matching TS pi-mono's AssistantMessageComponent pattern.
type AssistantMessageComponent struct {
	base *MessageComponentBase
	// HasFollowingToolCalls indicates whether tool calls follow this message in the entry list.
	// When true, OSC 133 zones are skipped (TS pi-mono: only wrap when no tool calls in message block).
	HasFollowingToolCalls bool
}

// NewAssistantMessageComponent creates a new assistant message component.
func NewAssistantMessageComponent(base *MessageComponentBase) *AssistantMessageComponent {
	return &AssistantMessageComponent{base: base}
}

// Render renders an assistant text message entry.
// Wraps in OSC 133 semantic zones if no tool calls follow.
// Uses glamour markdown rendering with wordWrap fallback.
// Handles stop reasons: "aborted" → error style, "error" → error style, "length" → warning style.
func (c *AssistantMessageComponent) Render(entry ChatEntry, width int) string {
	var sb strings.Builder

	// OSC 133 zones only when no tool calls in this message (TS pi-mono)
	if !c.HasFollowingToolCalls {
		sb.WriteString(osc133ZoneStart)
	}

	rendered, err := c.base.MdRenderer.Render(entry.Content)
	if err != nil {
		sb.WriteString(c.base.AssistantStyle.Render(wordWrap(entry.Content, width-10)))
	} else {
		rendered = strings.TrimSuffix(rendered, "\n")
		sb.WriteString(wrapURLsOSC8(rendered))
	}

	if !c.HasFollowingToolCalls {
		sb.WriteString(osc133ZoneEnd)
		sb.WriteString(osc133ZoneFinal)
	}

	// Stop reason rendering (TS pi-mono: stopReason)
	switch entry.StopReason {
	case "aborted":
		sb.WriteByte('\n')
		abortMsg := "Operation aborted"
		if entry.ErrorMessage != "" && entry.ErrorMessage != "Request was aborted" {
			abortMsg = entry.ErrorMessage
		}
		sb.WriteString(c.base.ErrorStyle.Render(abortMsg))
	case "error":
		sb.WriteByte('\n')
		errMsg := "Unknown error"
		if entry.ErrorMessage != "" {
			errMsg = entry.ErrorMessage
		}
		sb.WriteString(c.base.ErrorStyle.Render("Error: " + errMsg))
	case "length":
		sb.WriteByte('\n')
		sb.WriteString(c.base.WarningStyle.Render("[Output truncated — model reached token limit]"))
	}

	return sb.String()
}

// Ensure AssistantMessageComponent implements MessageComponent.
var _ MessageComponent = (*AssistantMessageComponent)(nil)
