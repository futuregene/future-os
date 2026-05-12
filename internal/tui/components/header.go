package components

// Header renders the top spacer above the chat.
// Currently a no-op — welcome text lives in the chat viewport.
type Header struct{}

// NewHeader creates a new Header component.
func NewHeader(accentColor, version string) Header {
	return Header{}
}

// SetWidth updates the header width (no-op).
func (h *Header) SetWidth(w int) {}

// View renders the header spacer lines.
func (h Header) View() string {
	return ""
}
