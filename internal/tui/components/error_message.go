package components

import (
	"strings"
)

// ErrorMessageComponent renders "error" and "warning" type chat entries.
// Extracted from ChatViewport.View() matching TS pi-mono's error/warning patterns.
// Uses ErrorStyle for errors, WarningStyle for warnings.
type ErrorMessageComponent struct {
	base *MessageComponentBase
}

// NewErrorMessageComponent creates a new error message component.
func NewErrorMessageComponent(base *MessageComponentBase) *ErrorMessageComponent {
	return &ErrorMessageComponent{base: base}
}

// Render renders an error or warning entry.
// "error" type → ErrorStyle with "Error: " prefix.
// "warning" type → WarningStyle with "Warning: " prefix.
// Both use wordWrap for text formatting.
func (c *ErrorMessageComponent) Render(entry ChatEntry, width int) string {
	switch entry.Type {
	case "error":
		return c.base.ErrorStyle.Render("Error: " + wordWrap(entry.Content, width-10))
	case "warning":
		return c.base.WarningStyle.Render("Warning: " + wordWrap(entry.Content, width-10))
	default:
		// Fallback for unknown types
		var sb strings.Builder
		sb.WriteString(c.base.WarningStyle.Render(entry.Type + ": "))
		sb.WriteString(wordWrap(entry.Content, width-10))
		return sb.String()
	}
}

// Ensure ErrorMessageComponent implements MessageComponent.
var _ MessageComponent = (*ErrorMessageComponent)(nil)
