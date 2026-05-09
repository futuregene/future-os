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
		Title:  title,
		Items:  items,
		Width:  60,
		Height: 20,
	}
}

// MoveDown moves selection down.
func (s *ListSelector) MoveDown() {
	if s.Selected < len(s.Items)-1 {
		s.Selected++
	}
}

// MoveUp moves selection up.
func (s *ListSelector) MoveUp() {
	if s.Selected > 0 {
		s.Selected--
	}
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

// ApplyFilter filters items by label prefix.
func (s *ListSelector) ApplyFilter(filter string) {
	s.Filter = filter
}

// filteredItems returns items matching the filter using subsequence fuzzy matching.
func (s ListSelector) filteredItems() []SelectorItem {
	if s.Filter == "" {
		return s.Items
	}
	var result []SelectorItem
	lower := strings.ToLower(s.Filter)
	for _, item := range s.Items {
		if fuzzyMatch(lower, strings.ToLower(item.Label)) ||
			fuzzyMatch(lower, strings.ToLower(item.Description)) {
			result = append(result, item)
		}
	}
	return result
}

// fuzzyMatch returns true if each character in pattern appears in order in s.
func fuzzyMatch(pattern, s string) bool {
	if pattern == "" {
		return true
	}
	j := 0
	for i := 0; i < len(s) && j < len(pattern); i++ {
		if s[i] == pattern[j] {
			j++
		}
	}
	return j == len(pattern)
}

// View renders the selector.
func (s ListSelector) View() string {
	titleStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#61afef")).
		Bold(true).
		PaddingBottom(1)

	itemStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#abb2bf")).
		PaddingLeft(2)

	selectedStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#1e1e2e")).
		Background(lipgloss.Color("#89b4fa")).
		PaddingLeft(2)

	filterStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#e5c07b")).
		PaddingBottom(1)

	var sb strings.Builder

	sb.WriteString(titleStyle.Render(s.Title))
	if s.Filter != "" {
		sb.WriteString(filterStyle.Render("Filter: " + s.Filter))
	}
	sb.WriteByte('\n')

	items := s.filteredItems()

	// No matches message (TS pi-mono: select-list.ts)
	if len(items) == 0 {
		noMatchStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")).
			PaddingLeft(2)
		sb.WriteString(noMatchStyle.Render("No matching"))
		// Help text
		helpStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")).
			PaddingTop(1)
		sb.WriteString(helpStyle.Render("↑↓ navigate  Enter select  Esc cancel  / filter"))
		return sb.String()
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
		line := item.Label
		if i == s.Selected {
			sb.WriteString(selectedStyle.Render("▶ " + line))
		} else {
			sb.WriteString(itemStyle.Render("  " + line))
		}
		sb.WriteByte('\n')
	}

	// Show selected item description below the list (TS pi-mono style)
	if sel := s.SelectedItem(); sel != nil && sel.Description != "" {
		descBoxStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#6c7086")).
			Italic(true).
			PaddingTop(1).
			PaddingLeft(2)
		// Wrap description to fit width
		descText := sel.Description
		if len(descText) > s.Width-4 && s.Width > 10 {
			descText = descText[:s.Width-7] + "..."
		}
		sb.WriteString(descBoxStyle.Render(descText))
		sb.WriteByte('\n')
	}

	// Scroll position indicator (TS pi-mono: "(3/15)")
	if total := len(items); total > visibleCount {
		scrollStyle := lipgloss.NewStyle().
			Foreground(lipgloss.Color("#5c6370")).
			PaddingLeft(2)
		sb.WriteString(scrollStyle.Render(fmt.Sprintf("(%d/%d)", s.Selected+1, total)))
		sb.WriteByte('\n')
	}

	// Help text
	helpStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370")).
		PaddingTop(1)
	sb.WriteString(helpStyle.Render("↑↓ navigate  Enter select  Esc cancel  / filter"))

	return sb.String()
}
