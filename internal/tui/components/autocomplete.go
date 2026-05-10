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
	descStyle    lipgloss.Style
}

// NewAutocomplete creates a new autocomplete component.
func NewAutocomplete() Autocomplete {
	return Autocomplete{
		style: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#abb2bf")).
			PaddingLeft(2),
		selectStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#89b4fa")).
			PaddingLeft(2),
		descStyle: lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")),
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

// scoredCandidate holds a candidate with its fuzzy match score.
type scoredCandidate struct {
	name  string
	score float64
}

// Filter filters candidates by prefix, then scores fuzzy matches (TS pi-mono: fuzzyFilter).
func (a *Autocomplete) Filter() {
	if a.prefix == "" {
		return
	}
	lower := strings.ToLower(a.prefix)

	// Split into space-separated tokens
	tokens := strings.Fields(lower)
	if len(tokens) == 0 {
		return
	}

	var prefixMatches []scoredCandidate
	var fuzzyMatches []scoredCandidate

	for _, c := range a.candidates {
		cl := strings.ToLower(c)
		if strings.HasPrefix(cl, lower) {
			// Prefix match: score 0 (best), break ties by length
			prefixMatches = append(prefixMatches, scoredCandidate{c, float64(len(c))})
		} else {
			// Try multi-token fuzzy match
			totalScore := 0.0
			allMatch := true
			for _, token := range tokens {
				if ok, score := fuzzyScoreToken(token, cl); ok {
					totalScore += score
				} else {
					allMatch = false
					break
				}
			}
			if allMatch {
				fuzzyMatches = append(fuzzyMatches, scoredCandidate{c, totalScore})
			}
		}
	}

	// Sort by score (lower = better)
	sortCandidates(prefixMatches)
	sortCandidates(fuzzyMatches)

	a.candidates = make([]string, 0, len(prefixMatches)+len(fuzzyMatches))
	for _, sc := range prefixMatches {
		a.candidates = append(a.candidates, sc.name)
	}
	for _, sc := range fuzzyMatches {
		a.candidates = append(a.candidates, sc.name)
	}
	if a.selected >= len(a.candidates) {
		a.selected = 0
	}
}

// fuzzyScoreToken checks if all runes of query appear in s in order.
// Returns (match bool, score float64). Lower score = better match.
// Rewards word-boundary matches and consecutive chars; penalizes gaps.
func fuzzyScoreToken(query, s string) (bool, float64) {
	if len(query) == 0 {
		return true, 0
	}

	qRunes := []rune(query)
	sRunes := []rune(s)
	if len(qRunes) > len(sRunes) {
		return false, 0
	}

	score := 0.0
	queryIdx := 0
	lastMatchIdx := -1
	consecutive := 0

	for i := 0; i < len(sRunes) && queryIdx < len(qRunes); i++ {
		if sRunes[i] == qRunes[queryIdx] {
			// Word boundary bonus
			isWordBoundary := i == 0 || sRunes[i-1] == ' ' || sRunes[i-1] == '-' || sRunes[i-1] == '_' || sRunes[i-1] == '.' || sRunes[i-1] == '/' || sRunes[i-1] == ':'
			if isWordBoundary {
				score -= 10
			}

			// Consecutive match bonus
			if lastMatchIdx == i-1 {
				consecutive++
				score -= float64(consecutive) * 5
			} else {
				consecutive = 0
				if lastMatchIdx >= 0 {
					score += float64(i-lastMatchIdx-1) * 2
				}
			}

			// Slight penalty for later matches
			score += float64(i) * 0.1

			lastMatchIdx = i
			queryIdx++
		}
	}

	if queryIdx < len(qRunes) {
		return false, 0
	}

	// Exact match bonus
	if query == s {
		score -= 100
	}

	return true, score
}

func sortCandidates(items []scoredCandidate) {
	for i := 0; i < len(items); i++ {
		for j := i + 1; j < len(items); j++ {
			if items[i].score > items[j].score {
				items[i], items[j] = items[j], items[i]
			}
		}
	}
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
		desc, hasDesc := a.descriptions[c]
		if i == a.selected {
			prefix := "→ "
			if hasDesc && desc != "" {
				sb.WriteString(a.selectStyle.Render(prefix + c) + a.descStyle.Render("  " + desc))
			} else {
				sb.WriteString(a.selectStyle.Render(prefix + c))
			}
		} else {
			prefix := "  "
			if hasDesc && desc != "" {
				sb.WriteString(a.style.Render(prefix + c) + a.descStyle.Render("  " + desc))
			} else {
				sb.WriteString(a.style.Render(prefix + c))
			}
		}
		sb.WriteByte('\n')
	}

	// Scroll indicator
	if total > maxV {
		indicatorStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")).
			PaddingLeft(4)
		sb.WriteString(indicatorStyle.Render(
			strings.Repeat(" ", 2) + fmt.Sprintf("(%d/%d)", a.selected+1, total)))
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
