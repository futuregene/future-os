package components

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// Autocomplete provides tab-completion for commands, files, and models.
type Autocomplete struct {
	active      bool
	candidates  []string
	selected    int
	prefix      string
	style       lipgloss.Style
	selectStyle lipgloss.Style
}

// NewAutocomplete creates a new autocomplete component.
func NewAutocomplete() Autocomplete {
	return Autocomplete{
		style: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#abb2bf")).
			PaddingLeft(4),
		selectStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#1e1e2e")).
			Background(lipgloss.Color("#89b4fa")).
			PaddingLeft(4),
	}
}

// Show activates autocomplete with candidates.
func (a *Autocomplete) Show(candidates []string, prefix string) {
	a.active = true
	a.candidates = candidates
	a.selected = 0
	a.prefix = prefix
}

// Hide deactivates autocomplete.
func (a *Autocomplete) Hide() {
	a.active = false
	a.candidates = nil
	a.selected = 0
}

// Active returns whether autocomplete is visible.
func (a *Autocomplete) Active() bool {
	return a.active
}

// SelectNext moves selection down.
func (a *Autocomplete) SelectNext() {
	if a.selected < len(a.candidates)-1 {
		a.selected++
	}
}

// SelectPrev moves selection up.
func (a *Autocomplete) SelectPrev() {
	if a.selected > 0 {
		a.selected--
	}
}

// Selected returns the currently selected candidate.
func (a *Autocomplete) Selected() string {
	if !a.active || len(a.candidates) == 0 {
		return ""
	}
	return a.candidates[a.selected]
}

// Filter filters candidates by prefix.
func (a *Autocomplete) Filter() {
	if a.prefix == "" {
		return
	}
	filtered := make([]string, 0)
	lower := strings.ToLower(a.prefix)
	for _, c := range a.candidates {
		if strings.HasPrefix(strings.ToLower(c), lower) {
			filtered = append(filtered, c)
		}
	}
	a.candidates = filtered
	if a.selected >= len(a.candidates) {
		a.selected = 0
	}
}

// View renders the autocomplete popover.
func (a Autocomplete) View() string {
	if !a.active || len(a.candidates) == 0 {
		return ""
	}
	var sb strings.Builder
	for i, c := range a.candidates {
		if i == a.selected {
			sb.WriteString(a.selectStyle.Render("▶ " + c))
		} else {
			sb.WriteString(a.style.Render("  " + c))
		}
		sb.WriteByte('\n')
	}
	return sb.String()
}

// ─── Slash Command Candidates ─────────────────────────────────────────────

// SlashCommands returns all built-in slash command names.
func SlashCommands() []string {
	return []string{
		"/model", "/baseurl", "/memory", "/clear", "/settings",
		"/scoped-models", "/export", "/import", "/share", "/copy",
		"/name", "/session", "/changelog", "/hotkeys",
		"/fork", "/clone", "/tree", "/login", "/logout",
		"/new", "/compact", "/resume", "/reload", "/quit",
		"/theme", "/thinking",
	}
}
