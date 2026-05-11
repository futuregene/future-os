package components

import (
	"fmt"
	"time"

	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
)

// ─── CountdownTimer ───────────────────────────────────────────────────────

// CountdownTimer is a tea.Model that shows a retry countdown with attempt tracking.
// It renders a line like "Retrying (1/3) in 5s...  [Esc to cancel]" and
// auto-decrements remainingSeconds on each tick.
type CountdownTimer struct {
	attempt          int
	maxAttempts      int
	remainingSeconds int
	message          string
	active           bool

	// styles
	accentStyle  lipgloss.Style
	mutedStyle   lipgloss.Style
	spinnerChars []string
	frame        int
}

// OnCountdownTickMsg is sent internally on each 1s tick to advance the countdown.
type OnCountdownTickMsg struct{}

// NewCountdownTimer creates a new CountdownTimer with default styles.
func NewCountdownTimer() CountdownTimer {
	return CountdownTimer{
		accentStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#89b4fa")),
		mutedStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")),
		spinnerChars: []string{"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"},
	}
}

// SetAccentColor sets the accent (spinner) color.
func (c *CountdownTimer) SetAccentColor(hex string) {
	c.accentStyle = c.accentStyle.Copy().Foreground(lipgloss.Color(hex))
}

// SetMutedColor sets the muted (hint) color.
func (c *CountdownTimer) SetMutedColor(hex string) {
	c.mutedStyle = c.mutedStyle.Copy().Foreground(lipgloss.Color(hex))
}

// Start activates the countdown with the given attempt/max and delay in milliseconds.
func (c *CountdownTimer) Start(attempt, maxAttempts, delayMs int) {
	c.attempt = attempt
	c.maxAttempts = maxAttempts
	c.remainingSeconds = delayMs / 1000
	if c.remainingSeconds < 1 {
		c.remainingSeconds = 1
	}
	c.message = fmt.Sprintf("Retrying (%d/%d)", attempt, maxAttempts)
	c.active = true
}

// Stop deactivates the countdown.
func (c *CountdownTimer) Stop() {
	c.active = false
	c.remainingSeconds = 0
}

// IsActive returns whether the countdown is currently active.
func (c CountdownTimer) IsActive() bool {
	return c.active
}

// Init returns the initial Tick command for the countdown (1s interval).
func (c CountdownTimer) Init() tea.Cmd {
	if !c.active {
		return nil
	}
	return c.tickCmd()
}

// tickCmd returns a command that fires OnCountdownTickMsg after 1 second.
func (c CountdownTimer) tickCmd() tea.Cmd {
	return tea.Tick(time.Second, func(t time.Time) tea.Msg {
		return OnCountdownTickMsg{}
	})
}

// Update handles tick messages and stops the countdown when it reaches zero.
func (c CountdownTimer) Update(msg tea.Msg) (CountdownTimer, tea.Cmd) {
	if !c.active {
		return c, nil
	}

	switch msg.(type) {
	case OnCountdownTickMsg:
		c.frame = (c.frame + 1) % len(c.spinnerChars)
		if c.remainingSeconds > 0 {
			c.remainingSeconds--
		}
		if c.remainingSeconds <= 0 {
			c.active = false
			return c, nil
		}
		return c, c.tickCmd()
	}

	return c, nil
}

// View renders the countdown display.
func (c CountdownTimer) View() string {
	if !c.active {
		return ""
	}

	spin := c.spinnerChars[c.frame]
	msg := fmt.Sprintf("%s Retrying (%d/%d) in %ds...",
		spin, c.attempt, c.maxAttempts, c.remainingSeconds)

	cancelHint := c.mutedStyle.Render("  [Esc to cancel]")
	return c.accentStyle.Render(msg) + cancelHint
}
