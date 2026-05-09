package components

import (
	"fmt"
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
	maxVisible   int               // max items to show (0 = unlimited)
	scrollOffset int               // scroll offset when items exceed maxVisible
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

// SetMaxVisible sets the maximum number of visible items in the dropdown.
func (a *Autocomplete) SetMaxVisible(n int) {
	a.maxVisible = n
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
	lower := strings.ToLower(a.prefix)

	// First pass: prefix matches (e.g., "/mod" matches "/model")
	prefixMatches := make([]string, 0)
	fuzzyMatches := make([]string, 0)
	for _, c := range a.candidates {
		cl := strings.ToLower(c)
		if strings.HasPrefix(cl, lower) {
			prefixMatches = append(prefixMatches, c)
		} else if subsequenceMatch(cl, lower) {
			fuzzyMatches = append(fuzzyMatches, c)
		}
	}

	// Prefix matches first, then fuzzy matches
	a.candidates = append(prefixMatches, fuzzyMatches...)
	if a.selected >= len(a.candidates) {
		a.selected = 0
	}
}

// subsequenceMatch checks if all characters in query appear in s in order (fuzzy match).
func subsequenceMatch(s, query string) bool {
	if len(query) == 0 {
		return true
	}
	j := 0
	for i := 0; i < len(s) && j < len(query); i++ {
		if s[i] == query[j] {
			j++
		}
	}
	return j == len(query)
}

// View renders the autocomplete popover with descriptions when available.
// Selected line: "▶ /name  • Description"
// Normal line:  "  /name  • Description"
func (a *Autocomplete) View() string {
	if !a.active || len(a.candidates) == 0 {
		return ""
	}

	// Calculate visible range for scroll window
	total := len(a.candidates)
	maxV := a.maxVisible
	if maxV <= 0 || maxV > total {
		maxV = total
	}

	// Ensure selected is visible
	if a.selected < a.scrollOffset {
		a.scrollOffset = a.selected
	}
	if a.selected >= a.scrollOffset+maxV {
		a.scrollOffset = a.selected - maxV + 1
	}
	if a.scrollOffset < 0 {
		a.scrollOffset = 0
	}
	if a.scrollOffset+maxV > total {
		a.scrollOffset = total - maxV
		if a.scrollOffset < 0 {
			a.scrollOffset = 0
		}
	}

	start := a.scrollOffset
	end := start + maxV
	if end > total {
		end = total
	}

	var sb strings.Builder
	for i := start; i < end; i++ {
		c := a.candidates[i]
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

	// Scroll indicator
	if total > maxV {
		indicatorStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")).
			PaddingLeft(4)
		sb.WriteString(indicatorStyle.Render(
			strings.Repeat(" ", 8) + fmt.Sprintf("(%d/%d)", a.selected+1, total)))
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
		"/theme", "/thinking", "/sessions", "/debug",
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
		{Name: "/debug", Description: "Write debug log"},
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
		{Name: "/sessions", Description: "Browse saved sessions"},
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
