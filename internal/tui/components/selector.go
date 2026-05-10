package components

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"
)

// ListSelector is a generic list-based selector for the overlay.
type ListSelector struct {
	Title    string
	Items    []SelectorItem
	Selected int
	Filter   string
	Width    int
	Height   int
	HelpText string // if non-empty, replaces the default "↑↓ navigate  Enter select  Esc cancel  / filter"

	// TwoColumn enables a two-column layout where items show Label and Description
	// side-by-side with aligned values (TS pi-mono: SettingsList).
	TwoColumn bool

	// ForkMode enables 2-line per item display with metadata line and blank separators
	// (TS pi-mono: UserMessageSelectorComponent). Items show bold cursor-selected text,
	// description on a second line, and blank line between messages.
	ForkMode bool
	// ForkDescription is the subtitle shown below the title in fork mode.
	ForkDescription string

	// NoMatchText overrides the default "No matching commands" message when filter
	// returns no results (TS pi-mono: "No matching models" in model selector).
	NoMatchText string

	// OnSelectionChange is called when the selected item changes (TS pi-mono: onSelectionChange).
	OnSelectionChange func(item SelectorItem)

	// SelectionInfoFunc returns an info line to display below the items (e.g. "Model Name: GPT-4o").
	// Called during View() with the currently selected index and item. Nil means no info line.
	SelectionInfoFunc func(index int, item SelectorItem) string

	// BorderColor for DynamicBorder lines at top/bottom (TS pi-mono: DynamicBorder).
	BorderColor string
	// AccentColor for the title (TS pi-mono: theme.accent).
	AccentColor string
}

// SelectorItem represents a selectable item.
type SelectorItem struct {
	Label       string
	Description string
	Value       string
	Active      bool // for multi-select support
}

// NewListSelector creates a new list selector.
func NewListSelector(title string, items []SelectorItem) ListSelector {
	return ListSelector{
		Title:       title,
		Items:       items,
		Width:       60,
		Height:      20,
		BorderColor: "#5c6370",
		AccentColor: "#89b4fa",
	}
}

// MoveDown moves selection down, wrapping to top at bottom (TS pi-mono).
func (s *ListSelector) MoveDown() {
	if len(s.Items) == 0 {
		return
	}
	if s.Selected < len(s.Items)-1 {
		s.Selected++
	} else {
		s.Selected = 0 // wrap to top
	}
	s.notifySelectionChange()
}

// MoveUp moves selection up, wrapping to bottom at top (TS pi-mono).
func (s *ListSelector) MoveUp() {
	if len(s.Items) == 0 {
		return
	}
	if s.Selected > 0 {
		s.Selected--
	} else {
		s.Selected = len(s.Items) - 1 // wrap to bottom
	}
	s.notifySelectionChange()
}

// PageDown moves selection down by a half-page (TS pi-mono: select.pageDown).
func (s *ListSelector) PageDown(pageSize int) {
	if len(s.Items) == 0 {
		return
	}
	if pageSize < 1 {
		pageSize = 5
	}
	s.Selected += pageSize
	if s.Selected >= len(s.Items) {
		s.Selected = len(s.Items) - 1
	}
	s.notifySelectionChange()
}

// PageUp moves selection up by a half-page (TS pi-mono: select.pageUp).
func (s *ListSelector) PageUp(pageSize int) {
	if len(s.Items) == 0 {
		return
	}
	if pageSize < 1 {
		pageSize = 5
	}
	s.Selected -= pageSize
	if s.Selected < 0 {
		s.Selected = 0
	}
	s.notifySelectionChange()
}

// notifySelectionChange calls OnSelectionChange if set (TS pi-mono).
func (s *ListSelector) notifySelectionChange() {
	if s.OnSelectionChange != nil {
		if item := s.SelectedItem(); item != nil {
			s.OnSelectionChange(*item)
		}
	}
}

// SetHelpText sets a custom help line, replacing the default.
// Use this to show keybinding-aware hints (TS pi-mono: keyHint/keyText).
func (s *ListSelector) SetHelpText(text string) {
	s.HelpText = text
}

// SelectIdx sets the selection to a specific index (clamped).
func (s *ListSelector) SelectIdx(idx int) {
	if idx < 0 {
		idx = 0
	}
	if idx >= len(s.Items) {
		idx = len(s.Items) - 1
	}
	if len(s.Items) > 0 {
		s.Selected = idx
		s.notifySelectionChange()
	}
}

// SelectedItem returns the currently selected item.
func (s *ListSelector) SelectedItem() *SelectorItem {
	if s.Selected >= 0 && s.Selected < len(s.Items) {
		return &s.Items[s.Selected]
	}
	return nil
}

// SetItems replaces all items and resets selection.
func (s *ListSelector) SetItems(items []SelectorItem) {
	s.Items = items
	if s.Selected >= len(items) {
		s.Selected = max(0, len(items)-1)
	}
}

// SetTitle updates the selector title.
func (s *ListSelector) SetTitle(title string) {
	s.Title = title
}

// normalizeDesc collapses multi-line descriptions to a single line (TS pi-mono: normalizeToSingleLine).
func normalizeDesc(d string) string {
	d = strings.ReplaceAll(d, "\r\n", " ")
	d = strings.ReplaceAll(d, "\r", " ")
	d = strings.ReplaceAll(d, "\n", " ")
	return strings.TrimSpace(d)
}

// ApplyFilter filters items by label prefix.
func (s *ListSelector) ApplyFilter(filter string) {
	s.Filter = filter
}

// fuzzyScore computes a match score for pattern in s.
// Returns (match bool, score int). Lower score = better match.
// Rewards word-boundary matches, consecutive chars, exact match.
// Penalizes gaps between matches (TS pi-mono: fuzzyMatch).
func fuzzyScore(pattern, s string) (bool, int) {
	if pattern == "" {
		return true, 0
	}
	if len(pattern) > len(s) {
		return false, 0
	}

	score := 0
	queryIdx := 0
	lastMatchIdx := -1
	consecutive := 0

	for i := 0; i < len(s) && queryIdx < len(pattern); i++ {
		if s[i] == pattern[queryIdx] {
			// Word boundary check
			isWordBoundary := i == 0 || s[i-1] == ' ' || s[i-1] == '-' || s[i-1] == '_' || s[i-1] == '.' || s[i-1] == '/' || s[i-1] == ':'
			if isWordBoundary {
				score -= 10
			}

			// Consecutive match bonus
			if lastMatchIdx == i-1 {
				consecutive++
				score -= consecutive * 5
			} else {
				consecutive = 0
				if lastMatchIdx >= 0 {
					score += (i - lastMatchIdx - 1) * 2
				}
			}

			// Slight penalty for later matches
			score += i / 10

			lastMatchIdx = i
			queryIdx++
		}
	}

	if queryIdx < len(pattern) {
		return false, 0
	}

	// Exact match bonus
	if pattern == s {
		score -= 100
	}

	return true, score
}

// trySwappedAlphaNum tries alpha-numeric swap (e.g. "haiku3" → "3haiku").
func trySwappedAlphaNum(pattern string) string {
	// Split into letters then digits
	split := -1
	for i := 0; i < len(pattern)-1; i++ {
		if isLetter(pattern[i]) && isDigit(pattern[i+1]) {
			split = i + 1
			break
		}
		if isDigit(pattern[i]) && isLetter(pattern[i+1]) {
			split = i + 1
			break
		}
	}
	if split < 0 {
		return ""
	}
	return pattern[split:] + pattern[:split]
}

func isLetter(c byte) bool {
	return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z')
}

func isDigit(c byte) bool {
	return c >= '0' && c <= '9'
}

// fuzzyMatchItem tries to match a pattern against an item's label and description.
// Returns (match bool, score int). Lower score = better.
func fuzzyMatchItem(pattern, label, description string) (bool, int) {
	lower := strings.ToLower(pattern)

	// Try without tokenization first for simple queries
	if ok, score := fuzzyScore(lower, strings.ToLower(label)); ok {
		return true, score
	}
	if description != "" {
		if ok, score := fuzzyScore(lower, strings.ToLower(description)); ok {
			return true, score + 20 // slight penalty for description matches
		}
	}

	// Try alpha-num swap (e.g., "haiku3" matches "claude-haiku-3.5")
	if swapped := trySwappedAlphaNum(lower); swapped != "" {
		if ok, score := fuzzyScore(swapped, strings.ToLower(label)); ok {
			return true, score + 5
		}
	}

	return false, 0
}

type scoredItem struct {
	item  SelectorItem
	score int
}

// filteredItems returns items matching the filter, sorted by match quality (TS pi-mono: fuzzyFilter).
func (s ListSelector) filteredItems() []SelectorItem {
	if s.Filter == "" {
		return s.Items
	}
	lower := strings.ToLower(strings.TrimSpace(s.Filter))
	if lower == "" {
		return s.Items
	}

	// Split into space-separated tokens
	tokens := strings.Fields(lower)
	if len(tokens) == 0 {
		return s.Items
	}

	var results []scoredItem
	for _, item := range s.Items {
		totalScore := 0
		allMatch := true
		for _, token := range tokens {
			ok, score := fuzzyMatchItem(token, item.Label, item.Description)
			if !ok {
				allMatch = false
				break
			}
			totalScore += score
		}
		if allMatch {
			results = append(results, scoredItem{item, totalScore})
		}
	}

	// Sort by score (lower is better)
	sortResults(results)

	var items []SelectorItem
	for _, r := range results {
		items = append(items, r.item)
	}
	return items
}

func sortResults(results []scoredItem) {
	for i := 0; i < len(results); i++ {
		for j := i + 1; j < len(results); j++ {
			if results[i].score > results[j].score {
				results[i], results[j] = results[j], results[i]
			}
		}
	}
}

// viewForkMode renders the selector in fork mode with 2-line per item display (TS pi-mono: UserMessageSelector).
func (s ListSelector) viewForkMode() string {
	titleStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color(s.AccentColor)).
		Bold(true).
		PaddingBottom(0)

	descStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#6c7086"))

	helpStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370")).
		PaddingTop(1)

	noMatchStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370")).
		PaddingLeft(2)

	var sb strings.Builder

	sb.WriteString(titleStyle.Render(s.Title))
	if s.ForkDescription != "" {
		sb.WriteString(descStyle.Render(" " + s.ForkDescription))
	}
	sb.WriteByte('\n')
	sb.WriteString(titleStyle.PaddingBottom(1).Render(""))
	sb.WriteByte('\n')

	// DynamicBorder top
	borderLine := lipgloss.NewStyle().
		Foreground(lipgloss.Color(s.BorderColor)).
		Render(strings.Repeat("─", max(1, s.Width)))
	sb.WriteString(borderLine)
	sb.WriteByte('\n')

	items := s.Items
	if len(items) == 0 {
		sb.WriteString(noMatchStyle.Render("No user messages found"))
		helpText := "↑↓ navigate  Enter select  Esc cancel"
		if s.HelpText != "" {
			helpText = s.HelpText
		}
		sb.WriteString(helpStyle.Render(helpText))
		return sb.String()
	}

	// Calculate visible range
	visibleCount := s.Height - 5 // header + border + help
	if visibleCount < 3 {
		visibleCount = 3
	}
	// Each item takes 3 lines (text + metadata + blank), so adjust
	visibleItems := visibleCount / 3
	if visibleItems < 1 {
		visibleItems = 1
	}

	start := s.Selected - visibleItems/2
	if start < 0 {
		start = 0
	}
	end := start + visibleItems
	if end > len(items) {
		end = len(items)
		start = end - visibleItems
		if start < 0 {
			start = 0
		}
	}

	normalStyle := lipgloss.NewStyle()
	boldStyle := lipgloss.NewStyle().Bold(true)
	mutedStyle := lipgloss.NewStyle().Foreground(lipgloss.Color("#6c7086"))

	for i := start; i < end; i++ {
		item := items[i]
		isSelected := i == s.Selected

		// Normalize description to single line
		desc := normalizeDesc(item.Description)
		label := normalizeDesc(item.Label)

		// Line 1: cursor + message text
		cursor := "  "
		if isSelected {
			cursor = "› "
		}
		maxTextWidth := s.Width - 4
		text := label
		if lipgloss.Width(text) > maxTextWidth {
			text = TruncateByWidth(text, maxTextWidth-1) + "..."
		}
		if isSelected {
			sb.WriteString(boldStyle.Render(cursor + text))
		} else {
			sb.WriteString(normalStyle.Render(cursor + text))
		}
		sb.WriteByte('\n')

		// Line 2: metadata (position in session)
		sb.WriteString(mutedStyle.Render("  " + desc))
		sb.WriteByte('\n')

		// Blank line between messages
		sb.WriteByte('\n')
	}

	// Scroll position indicator
	if total := len(items); total > visibleItems {
		scrollStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")).
			PaddingLeft(2)
		sb.WriteString(scrollStyle.Render(fmt.Sprintf("  (%d/%d)", s.Selected+1, total)))
		sb.WriteByte('\n')
	}

	// DynamicBorder bottom
	sb.WriteString(borderLine)
	sb.WriteByte('\n')

	// Help text
	helpText := "↑↓ navigate  Enter select  Esc cancel"
	if s.HelpText != "" {
		helpText = s.HelpText
	}
	sb.WriteString(helpStyle.Render(helpText))

	return sb.String()
}

// View renders the selector with optional multi-column description layout (TS pi-mono).
func (s ListSelector) View() string {
	titleStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color(s.AccentColor)).
		Bold(true).
		PaddingBottom(1)

	itemStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#abb2bf")).
		PaddingLeft(2)

	selectedStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#89b4fa")).
		PaddingLeft(2)

	descStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#6c7086"))

	filterStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#e5c07b")).
		PaddingBottom(1)

	var sb strings.Builder

	sb.WriteString(titleStyle.Render(s.Title))
	if s.Filter != "" {
		sb.WriteString(filterStyle.Render("Filter: " + s.Filter))
	}
	sb.WriteByte('\n')

	// DynamicBorder top (TS pi-mono: DynamicBorder)
	borderLine := lipgloss.NewStyle().
		Foreground(lipgloss.Color(s.BorderColor)).
		Render(strings.Repeat("─", max(1, s.Width)))
	sb.WriteString(borderLine)
	sb.WriteByte('\n')

	// Fork mode: 2-line per item with metadata, blank separators (TS pi-mono: UserMessageSelector)
	if s.ForkMode {
		return s.viewForkMode()
	}

	items := s.filteredItems()

	// No matches message (TS pi-mono: select-list.ts)
	if len(items) == 0 {
		noMatchStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")).
			PaddingLeft(2)
		noMatchMsg := s.NoMatchText
		if noMatchMsg == "" {
			noMatchMsg = "No matching commands"
		}
		sb.WriteString(noMatchStyle.Render(noMatchMsg))
		// Help text
		helpStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")).
			PaddingTop(1)
		helpText := "↑↓ navigate  Enter select  Esc cancel  / filter"
		if s.HelpText != "" {
			helpText = s.HelpText
		}
		sb.WriteString(helpStyle.Render(helpText))
		return sb.String()
	}

	// Check if any items have descriptions for multi-column mode (TS pi-mono)
	hasDesc := false
	for _, it := range items {
		if it.Description != "" {
			hasDesc = true
			break
		}
	}

	// Calculate primary column width (longest label + gap)
	// TS pi-mono: bounded by DEFAULT_PRIMARY_COLUMN_WIDTH (32)
	primaryColWidth := 0
	if hasDesc && s.Width > 40 {
		for _, it := range items {
			if l := lipgloss.Width(it.Label); l > primaryColWidth {
				primaryColWidth = l
			}
		}
		if primaryColWidth < 12 {
			primaryColWidth = 12
		}
		if primaryColWidth > 32 {
			primaryColWidth = 32
		}
		if primaryColWidth > s.Width/2 {
			primaryColWidth = s.Width / 2
		}
	}

	visibleCount := s.Height - 3
	start := s.Selected - visibleCount/2
	if start < 0 {
		start = 0
	}
	end := start + visibleCount
	if end > len(items) {
		end = len(items)
		start = end - visibleCount
		if start < 0 {
			start = 0
		}
	}

	for i := start; i < end; i++ {
		item := items[i]
		prefix := "→ "
		if i != s.Selected {
			prefix = "  "
		}

		// Two-column settings layout (TS pi-mono: SettingsList)
		if s.TwoColumn {
			maxLabelWidth := 0
			for _, it := range items {
				if lw := lipgloss.Width(it.Label); lw > maxLabelWidth {
					maxLabelWidth = lw
				}
			}
			if maxLabelWidth > 30 {
				maxLabelWidth = 30
			}
			labelPadded := item.Label + strings.Repeat(" ", max(0, maxLabelWidth-lipgloss.Width(item.Label)))
			value := item.Description
			separator := "  "
			prefixW := lipgloss.Width(prefix)
			valueMaxW := s.Width - prefixW - maxLabelWidth - lipgloss.Width(separator) - 2
			if valueMaxW < 8 {
				valueMaxW = 8
			}
			if lipgloss.Width(value) > valueMaxW {
				value = TruncateByWidth(value, valueMaxW-1) + "..."
			}
			valueStyled := lipgloss.NewStyle().Foreground(lipgloss.Color("#6c7086")).Render(value)
			line := prefix + itemStyle.Render(labelPadded) + separator + valueStyled
			if i == s.Selected {
				line = selectedStyle.Render(prefix + labelPadded + separator + value)
			}
			sb.WriteString(line)
			sb.WriteByte('\n')
			continue
		}

		if hasDesc && s.Width > 40 && item.Description != "" {
			// Multi-column: label + spacing + description (TS pi-mono)
			label := item.Label
			gap := primaryColWidth - lipgloss.Width(label)
			if gap < 1 {
				gap = 1
			}
			spacing := strings.Repeat(" ", gap)
			// Truncate description to fit remaining width
			prefixWidth := lipgloss.Width(prefix) + 2 // +2 for safety margin
			remainingWidth := s.Width - prefixWidth - primaryColWidth - gap
			if remainingWidth < 8 {
				remainingWidth = 8
			}
			descText := normalizeDesc(item.Description)
			if lipgloss.Width(descText) > remainingWidth {
				descText = TruncateByWidth(descText, remainingWidth-1) + "..."
			}
			if i == s.Selected {
				sb.WriteString(selectedStyle.Render(prefix + label + spacing + descStyle.Render(descText)))
			} else {
				sb.WriteString(itemStyle.Render(prefix + label + spacing + descStyle.Render(descText)))
			}
		} else {
			line := item.Label
			if i == s.Selected {
				sb.WriteString(selectedStyle.Render(prefix + line))
			} else {
				sb.WriteString(itemStyle.Render(prefix + line))
			}
		}
		sb.WriteByte('\n')
	}

	// Show selected item description below the list (TS pi-mono: always visible when selected item has description)
	if sel := s.SelectedItem(); sel != nil && sel.Description != "" {
		descBoxStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#6c7086")).
			Italic(true).
			PaddingTop(1).
			PaddingLeft(2)
		descText := normalizeDesc(sel.Description)
		descWrapped := wordWrap(descText, s.Width-6)
		sb.WriteString(descBoxStyle.Render(descWrapped))
		sb.WriteByte('\n')
	}

	// Selection info line (TS pi-mono: "Model Name: GPT-4o" in model selector)
	if s.SelectionInfoFunc != nil && len(items) > 0 {
		selectedItem := items[s.Selected]
		if info := s.SelectionInfoFunc(s.Selected, selectedItem); info != "" {
			infoStyle := lipgloss.NewStyle().
				Foreground(lipgloss.Color("#6c7086")).
				PaddingLeft(2)
			sb.WriteString(infoStyle.Render(info))
			sb.WriteByte('\n')
		}
	}

	// Scroll position indicator (TS pi-mono: "(3/15)")
	if total := len(items); total > visibleCount {
		scrollStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")).
			PaddingLeft(2)
		sb.WriteString(scrollStyle.Render(fmt.Sprintf("(%d/%d)", s.Selected+1, total)))
		sb.WriteByte('\n')
	}

	// DynamicBorder bottom (TS pi-mono: DynamicBorder)
	sb.WriteString(borderLine)
	sb.WriteByte('\n')

	// Help text
	helpStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370")).
		PaddingTop(1)
	helpText := "↑↓ navigate  Enter select  Esc cancel  / filter"
	if s.HelpText != "" {
		helpText = s.HelpText
	}
	sb.WriteString(helpStyle.Render(helpText))

	return sb.String()
}
