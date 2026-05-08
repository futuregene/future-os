package components

import (
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// SlashCommand represents a slash command with its name and description.
type SlashCommand struct {
	Name        string
	Description string
}

// Autocomplete provides tab-completion for commands, files, and models.
type Autocomplete struct {
	active       bool
	candidates   []string
	selected     int
	prefix       string
	descriptions map[string]string // candidate name → description
	style        lipgloss.Style
	selectStyle  lipgloss.Style
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

// Show activates autocomplete with candidates and optional descriptions.
// descriptions maps candidate names to human-readable descriptions.
func (a *Autocomplete) Show(candidates []string, descriptions map[string]string, prefix string) {
	a.active = true
	a.candidates = candidates
	a.descriptions = descriptions
	a.selected = 0
	a.prefix = prefix
}

// Hide deactivates autocomplete.
func (a *Autocomplete) Hide() {
	a.active = false
	a.candidates = nil
	a.descriptions = nil
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

// Selected returns the currently selected candidate name (without description).
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

// View renders the autocomplete popover with descriptions when available.
// Selected line: "▶ /name  • Description"
// Normal line:  "  /name  • Description"
func (a Autocomplete) View() string {
	if !a.active || len(a.candidates) == 0 {
		return ""
	}
	var sb strings.Builder
	for i, c := range a.candidates {
		line := "  " + c
		if desc, ok := a.descriptions[c]; ok && desc != "" {
			line += "  • " + desc
		}
		if i == a.selected {
			sb.WriteString(a.selectStyle.Render("▶" + line[1:]))
		} else {
			sb.WriteString(a.style.Render(line))
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

// SlashCommandsWithDesc returns all built-in slash commands with descriptions.
func SlashCommandsWithDesc() []SlashCommand {
	return []SlashCommand{
		{Name: "/model", Description: "Switch AI model"},
		{Name: "/baseurl", Description: "Set API base URL"},
		{Name: "/memory", Description: "List recent sessions"},
		{Name: "/clear", Description: "Delete session"},
		{Name: "/settings", Description: "Open settings"},
		{Name: "/scoped-models", Description: "Set project-specific models"},
		{Name: "/export", Description: "Export session"},
		{Name: "/import", Description: "Import session"},
		{Name: "/share", Description: "Share conversation"},
		{Name: "/copy", Description: "Copy last message"},
		{Name: "/name", Description: "Name current session"},
		{Name: "/session", Description: "Session info"},
		{Name: "/changelog", Description: "What's new"},
		{Name: "/hotkeys", Description: "Keyboard shortcuts"},
		{Name: "/fork", Description: "Fork conversation"},
		{Name: "/clone", Description: "Clone conversation"},
		{Name: "/tree", Description: "View message tree"},
		{Name: "/login", Description: "Authenticate"},
		{Name: "/logout", Description: "Sign out"},
		{Name: "/new", Description: "New session"},
		{Name: "/compact", Description: "Compress context"},
		{Name: "/resume", Description: "Resume session"},
		{Name: "/reload", Description: "Reload settings"},
		{Name: "/quit", Description: "Exit"},
		{Name: "/theme", Description: "Switch theme"},
		{Name: "/thinking", Description: "Set thinking level"},
		{Name: "/help", Description: "Show help"},
	}
}

// SlashCommandNames returns the names from a slice of SlashCommand.
func SlashCommandNames(cmds []SlashCommand) []string {
	names := make([]string, len(cmds))
	for i, c := range cmds {
		names[i] = c.Name
	}
	return names
}

// SlashCommandDescriptions returns a name→description map from a slice of SlashCommand.
func SlashCommandDescriptions(cmds []SlashCommand) map[string]string {
	m := make(map[string]string, len(cmds))
	for _, c := range cmds {
		m[c.Name] = c.Description
	}
	return m
}
