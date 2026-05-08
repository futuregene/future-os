package components

import (
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

// SelectedItem returns the currently selected item.
func (s *ListSelector) SelectedItem() *SelectorItem {
	if s.Selected >= 0 && s.Selected < len(s.Items) {
		return &s.Items[s.Selected]
	}
	return nil
}

// ApplyFilter filters items by label prefix.
func (s *ListSelector) ApplyFilter(filter string) {
	s.Filter = filter
}

// filteredItems returns items matching the filter.
func (s ListSelector) filteredItems() []SelectorItem {
	if s.Filter == "" {
		return s.Items
	}
	var result []SelectorItem
	lower := strings.ToLower(s.Filter)
	for _, item := range s.Items {
		if strings.Contains(strings.ToLower(item.Label), lower) ||
			strings.Contains(strings.ToLower(item.Description), lower) {
			result = append(result, item)
		}
	}
	return result
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

	items := s.filteredItems()
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
		if item.Description != "" {
			line += "  " + descStyle.Render(item.Description)
		}
		if i == s.Selected {
			sb.WriteString(selectedStyle.Render("▶ " + line))
		} else {
			sb.WriteString(itemStyle.Render("  " + line))
		}
		sb.WriteByte('\n')
	}

	// Help text
	helpStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#5c6370")).
		PaddingTop(1)
	sb.WriteString(helpStyle.Render("↑↓ navigate  Enter select  Esc cancel  / filter"))

	return sb.String()
}
