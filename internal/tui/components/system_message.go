package components

// SystemMessageComponent renders "system" type chat entries.
// Extracted from ChatViewport.View() matching TS pi-mono's system message pattern.
// Uses SystemStyle (dim italic) with word-wrap.
type SystemMessageComponent struct {
	base *MessageComponentBase
}

// NewSystemMessageComponent creates a new system message component.
func NewSystemMessageComponent(base *MessageComponentBase) *SystemMessageComponent {
	return &SystemMessageComponent{base: base}
}

// Render renders a system message entry.
// Uses SystemStyle with a 2-space indent and wordWrap.
func (c *SystemMessageComponent) Render(entry ChatEntry, width int) string {
	return c.base.SystemStyle.Render("  " + wordWrap(entry.Content, width-10))
}

// Ensure SystemMessageComponent implements MessageComponent.
var _ MessageComponent = (*SystemMessageComponent)(nil)
