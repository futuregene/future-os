package components

import (
	"strings"
	"time"

	"github.com/charmbracelet/lipgloss"
)

// LoaderType represents different loader animations.
type LoaderType int

const (
	LoaderSpinner LoaderType = iota
	LoaderCountdown
	LoaderBordered
)

// LoaderState holds the current state of a loader.
type LoaderState struct {
	Active    bool
	Type      LoaderType
	Message   string
	Progress  int    // for countdown: current attempt
	MaxRetry  int    // for countdown: max attempts
	Remaining int    // seconds remaining
}

// Loader renders loading indicators.
type Loader struct {
	state    LoaderState
	frame    int
	spinner  []string
	style    lipgloss.Style
}

// NewLoader creates a new loader.
func NewLoader() *Loader {
	return &Loader{
		spinner: []string{"⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"},
		style: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#89b4fa")),
	}
}

// StartSpinner starts a spinner loader.
func (l *Loader) StartSpinner(msg string) {
	l.state = LoaderState{Active: true, Type: LoaderSpinner, Message: msg}
}

// StartCountdown starts a countdown timer.
func (l *Loader) StartCountdown(msg string, attempt, maxRetry, remaining int) {
	l.state = LoaderState{
		Active:    true,
		Type:      LoaderCountdown,
		Message:   msg,
		Progress:  attempt,
		MaxRetry:  maxRetry,
		Remaining: remaining,
	}
}

// StartBordered starts a bordered loader.
func (l *Loader) StartBordered(msg string) {
	l.state = LoaderState{Active: true, Type: LoaderBordered, Message: msg}
}

// Stop deactivates the loader.
func (l *Loader) Stop() {
	l.state.Active = false
}

// Tick advances the animation frame.
func (l *Loader) Tick() {
	l.frame = (l.frame + 1) % len(l.spinner)
	if l.state.Type == LoaderCountdown && l.state.Remaining > 0 {
		l.state.Remaining--
	}
}

// View renders the loader.
func (l *Loader) View() string {
	if !l.state.Active {
		return ""
	}

	switch l.state.Type {
	case LoaderSpinner:
		spin := l.spinner[l.frame]
		return l.style.Render(spin + " " + l.state.Message)
	case LoaderCountdown:
		msg := l.spinner[l.frame] + " Retrying (" +
			itoa(l.state.Progress) + "/" + itoa(l.state.MaxRetry) +
			") in " + itoa(l.state.Remaining) + "s…"
		return l.style.Render(msg)
	case LoaderBordered:
		content := l.spinner[l.frame] + " " + l.state.Message
		return lipgloss.NewStyle().
			Border(lipgloss.RoundedBorder()).
			BorderForeground(lipgloss.Color("#89b4fa")).
			Padding(1, 2).
			Render(content)
	default:
		return ""
	}
}

var _ = strings.Repeat
var _ = time.Now
