package components

import (
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// ─── BorderedLoader ───────────────────────────────────────────────────────

// BorderedLoaderMsg is sent to the BorderedLoader to control its state.
type BorderedLoaderMsg struct {
	// Action: "start", "stop", "set_message"
	Action  string
	Message string
}

// BorderedLoader is a tea.Model that renders a rounded-border loading box
// with a spinner animation and a message inside.
type BorderedLoader struct {
	active       bool
	message      string
	spinnerFrame int
	spinnerChars []string

	// styles
	borderColor lipgloss.Color
	textColor   lipgloss.Color
	mutedColor  lipgloss.Color
}

// NewBorderedLoader creates a new BorderedLoader with default Catppuccin-style colors.
func NewBorderedLoader() BorderedLoader {
	return BorderedLoader{
		spinnerChars: []string{"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"},
		borderColor:  "#89b4fa",
		textColor:    "#cdd6f4",
		mutedColor:   "#5c6370",
	}
}

// SetBorderColor sets the border color (hex string).
func (b *BorderedLoader) SetBorderColor(hex string) {
	b.borderColor = lipgloss.Color(hex)
}

// SetTextColor sets the text color (hex string).
func (b *BorderedLoader) SetTextColor(hex string) {
	b.textColor = lipgloss.Color(hex)
}

// SetMutedColor sets the muted/hint color (hex string).
func (b *BorderedLoader) SetMutedColor(hex string) {
	b.mutedColor = lipgloss.Color(hex)
}

// Start activates the loader with the given message.
func (b *BorderedLoader) Start(msg string) {
	b.active = true
	b.message = msg
}

// Stop deactivates the loader.
func (b *BorderedLoader) Stop() {
	b.active = false
}

// SetMessage updates the loader message without changing active state.
func (b *BorderedLoader) SetMessage(msg string) {
	b.message = msg
}

// Init returns the initial spinner tick command.
func (b BorderedLoader) Init() tea.Cmd {
	if !b.active {
		return nil
	}
	return b.tickCmd()
}

func (b BorderedLoader) tickCmd() tea.Cmd {
	return tea.Tick(100*time.Millisecond, func(t time.Time) tea.Msg {
		return spinnerTickMsg{}
	})
}

// spinnerTickMsg advances the spinner animation frame.
type spinnerTickMsg struct{}

// Update handles spinner ticks and control messages.
func (b BorderedLoader) Update(msg tea.Msg) (BorderedLoader, tea.Cmd) {
	switch m := msg.(type) {
	case spinnerTickMsg:
		if !b.active {
			return b, nil
		}
		b.spinnerFrame = (b.spinnerFrame + 1) % len(b.spinnerChars)
		return b, b.tickCmd()

	case BorderedLoaderMsg:
		switch m.Action {
		case "start":
			b.Start(m.Message)
			if b.active {
				return b, b.tickCmd()
			}
		case "stop":
			b.Stop()
		case "set_message":
			b.SetMessage(m.Message)
		}
	}

	return b, nil
}

// View renders the bordered loading box.
func (b BorderedLoader) View() string {
	if !b.active {
		return ""
	}

	spin := b.spinnerChars[b.spinnerFrame]

	contentStyle := lipgloss.NewStyle().
		Foreground(b.textColor)

	content := contentStyle.Render(spin + " " + b.message)

	boxStyle := lipgloss.NewStyle().
		Border(lipgloss.RoundedBorder()).
		BorderForeground(b.borderColor).
		Padding(1, 2)

	return boxStyle.Render(content)
}
