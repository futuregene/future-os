package tui

import (
	"fmt"
	"strings"

	"github.com/charmbracelet/bubbles/textinput"
	tea "github.com/charmbracelet/bubbletea"
	"github.com/charmbracelet/lipgloss"
	"github.com/huichen/xihu/internal/config"
)

// ConfigSelectorModel is the tea.Model for the xihu config TUI.
type ConfigSelectorModel struct {
	groups     []config.ResourceGroup
	allItems   []config.ResourceItem
	filtered   []FlatEntry
	selected   int
	search     textinput.Model
	width      int
	height     int
	quitting   bool
	maxVisible int
}

// FlatEntry is a flattened view entry for rendering.
type FlatEntry struct {
	Type     string
	Group    *config.ResourceGroup
	Subgroup *config.ResourceSubgroup
	Item     *config.ResourceItem
}

// NewConfigSelectorModel creates a new config selector model.
func NewConfigSelectorModel(groups []config.ResourceGroup, allItems []config.ResourceItem) *ConfigSelectorModel {
	ti := textinput.New()
	ti.Placeholder = "Type to filter..."
	ti.Prompt = ""
	ti.CharLimit = 50
	ti.Width = 40

	m := &ConfigSelectorModel{
		groups:     groups,
		allItems:   allItems,
		search:     ti,
		maxVisible: 15,
	}
	m.filtered = m.flatList()
	// Select first item
	for i, e := range m.filtered {
		if e.Type == "item" {
			m.selected = i
			break
		}
	}
	return m
}

func (m *ConfigSelectorModel) flatList() []FlatEntry {
	var entries []FlatEntry
	for i := range m.groups {
		g := &m.groups[i]
		entries = append(entries, FlatEntry{Type: "group", Group: g})
		for j := range g.Subgroups {
			sg := &g.Subgroups[j]
			entries = append(entries, FlatEntry{Type: "subgroup", Group: g, Subgroup: sg})
			for k := range sg.Items {
				item := &sg.Items[k]
				entries = append(entries, FlatEntry{Type: "item", Item: item})
			}
		}
	}
	return entries
}

func (m *ConfigSelectorModel) filterItems(query string) {
	if strings.TrimSpace(query) == "" {
		m.filtered = m.flatList()
	} else {
		lower := strings.ToLower(query)
		var filtered []FlatEntry

		matchingItems := make(map[*config.ResourceItem]bool)
		matchingSubgroups := make(map[*config.ResourceSubgroup]bool)
		matchingGroups := make(map[*config.ResourceGroup]bool)

		flat := m.flatList()
		for _, e := range flat {
			if e.Type == "item" && e.Item != nil {
				if strings.Contains(strings.ToLower(e.Item.Name), lower) ||
					strings.Contains(strings.ToLower(string(e.Item.Type)), lower) ||
					strings.Contains(strings.ToLower(e.Item.Path), lower) {
					matchingItems[e.Item] = true
				}
			}
		}

		for _, e := range flat {
			if e.Type == "subgroup" && e.Subgroup != nil {
				for i := range e.Subgroup.Items {
					item := &e.Subgroup.Items[i]
					if matchingItems[item] {
						matchingSubgroups[e.Subgroup] = true
						matchingGroups[e.Group] = true
					}
				}
			}
		}

		for _, e := range flat {
			if e.Type == "group" && e.Group != nil && matchingGroups[e.Group] {
				filtered = append(filtered, e)
			} else if e.Type == "subgroup" && e.Subgroup != nil && matchingSubgroups[e.Subgroup] {
				filtered = append(filtered, e)
			} else if e.Type == "item" && e.Item != nil && matchingItems[e.Item] {
				filtered = append(filtered, e)
			}
		}
		m.filtered = filtered
	}

	// Select first item
	for i, e := range m.filtered {
		if e.Type == "item" {
			m.selected = i
			return
		}
	}
	m.selected = 0
}

func (m *ConfigSelectorModel) findNextItem(from int, dir int) int {
	idx := from + dir
	for idx >= 0 && idx < len(m.filtered) {
		if m.filtered[idx].Type == "item" {
			return idx
		}
		idx += dir
	}
	return from
}

func (m *ConfigSelectorModel) toggleSelectedItem() tea.Cmd {
	if m.selected < 0 || m.selected >= len(m.filtered) {
		return nil
	}
	e := m.filtered[m.selected]
	if e.Type != "item" || e.Item == nil {
		return nil
	}

	item := e.Item
	item.Enabled = !item.Enabled

	// Update in groups
	for gi := range m.groups {
		for si := range m.groups[gi].Subgroups {
			for ii := range m.groups[gi].Subgroups[si].Items {
				it := &m.groups[gi].Subgroups[si].Items[ii]
				if it.Path == item.Path && it.Type == item.Type {
					it.Enabled = item.Enabled
				}
			}
		}
	}

	// Persist to settings
	scope := item.Scope
	if scope == "" {
		scope = "user"
	}
	if err := config.ToggleResource(item.Path, item.Type, scope, item.Enabled); err != nil {
		// Log error but don't stop
	}

	return nil
}

// Init implements tea.Model.
func (m *ConfigSelectorModel) Init() tea.Cmd {
	return textinput.Blink
}

// Update implements tea.Model.
func (m *ConfigSelectorModel) Update(msg tea.Msg) (tea.Model, tea.Cmd) {
	switch msg := msg.(type) {
	case tea.KeyMsg:
		switch msg.String() {
		case "ctrl+c", "q", "esc":
			m.quitting = true
			return m, tea.Quit
		case "up", "k":
			m.selected = m.findNextItem(m.selected, -1)
			return m, nil
		case "down", "j":
			m.selected = m.findNextItem(m.selected, 1)
			return m, nil
		case " ":
			cmd := m.toggleSelectedItem()
			return m, cmd
		case "enter":
			cmd := m.toggleSelectedItem()
			if cmd != nil {
				return m, cmd
			}
			m.selected = m.findNextItem(m.selected, 1)
			return m, nil
		}

		var cmd tea.Cmd
		m.search, cmd = m.search.Update(msg)
		m.filterItems(m.search.Value())
		return m, cmd

	case tea.WindowSizeMsg:
		m.width = msg.Width
		m.height = msg.Height
		m.search.Width = msg.Width - 4
		return m, nil
	}

	return m, nil
}

// View implements tea.Model.
func (m *ConfigSelectorModel) View() string {
	if m.quitting {
		return "Configuration saved.\n"
	}

	var b strings.Builder

	// Title
	titleStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#89b4fa")).
		Bold(true)
	b.WriteString(titleStyle.Render("Resource Configuration"))
	b.WriteString("\n")

	hintStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#7f849c"))
	b.WriteString(hintStyle.Render("space=toggle · esc=quit · type to filter"))
	b.WriteString("\n\n")

	// Search input
	searchStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#cdd6f4")).
		PaddingLeft(2)
	if m.search.Value() == "" {
		b.WriteString(searchStyle.Render(hintStyle.Render("🔍 " + m.search.Placeholder)))
	} else {
		b.WriteString(searchStyle.Render("🔍 " + m.search.Value()))
	}
	b.WriteString("\n\n")

	if len(m.filtered) == 0 {
		b.WriteString(hintStyle.Render("  No resources found"))
		b.WriteString("\n")
		return b.String()
	}

	// Calculate visible range
	startIdx := 0
	if m.selected > m.maxVisible/2 {
		startIdx = m.selected - m.maxVisible/2
	}
	if startIdx > len(m.filtered)-m.maxVisible {
		startIdx = len(m.filtered) - m.maxVisible
	}
	if startIdx < 0 {
		startIdx = 0
	}
	endIdx := startIdx + m.maxVisible
	if endIdx > len(m.filtered) {
		endIdx = len(m.filtered)
	}

	// Style definitions
	groupStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#89b4fa")).
		Bold(true).
		PaddingLeft(2)

	subgroupStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#7f849c")).
		PaddingLeft(4)

	cursorStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#f38ba8"))

	enabledStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#a6e3a1"))

	disabledStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#7f849c"))

	selectedItemStyle := lipgloss.NewStyle().Bold(true)

	for i := startIdx; i < endIdx; i++ {
		e := m.filtered[i]
		isSelected := i == m.selected

		switch e.Type {
		case "group":
			if e.Group != nil {
				label := e.Group.Label
				if e.Group.Scope == "user" {
					label = "User (~/.xihu/)"
				} else if e.Group.Scope == "project" {
					label = "Project (.xihu/)"
				} else if e.Group.Scope == "agents" {
					label = "Agents (~/.agents/)"
				} else if e.Group.Scope == "pi" {
					label = "Pi (~/.pi/agent/)"
				}
				b.WriteString(groupStyle.Render(label))
			}
			b.WriteString("\n")
		case "subgroup":
			if e.Subgroup != nil {
				b.WriteString(subgroupStyle.Render(e.Subgroup.Label))
			}
			b.WriteString("\n")
		case "item":
			if e.Item != nil {
				cursor := "  "
				if isSelected {
					cursor = cursorStyle.Render("> ")
				}

				checkbox := "[ ]"
				if e.Item.Enabled {
					checkbox = enabledStyle.Render("[✓]")
				} else {
					checkbox = disabledStyle.Render("[ ]")
				}

				name := e.Item.Name
				if isSelected {
					name = selectedItemStyle.Render(name)
				}

				b.WriteString(fmt.Sprintf("    %s %s %s\n", cursor, checkbox, name))
			}
		}
	}

	// Scroll indicator
	if startIdx > 0 || endIdx < len(m.filtered) {
		itemCount := 0
		currentItemIndex := 0
		for i, e := range m.filtered {
			if e.Type == "item" {
				itemCount++
				if i <= m.selected {
					currentItemIndex = itemCount
				}
			}
		}
		if itemCount > 0 {
			b.WriteString(hintStyle.Render(fmt.Sprintf("  (%d/%d)", currentItemIndex, itemCount)))
			b.WriteString("\n")
		}
	}

	return b.String()
}

// RunConfigSelector launches the config selector TUI.
func RunConfigSelector(groups []config.ResourceGroup, allItems []config.ResourceItem) error {
	model := NewConfigSelectorModel(groups, allItems)
	p := tea.NewProgram(model, tea.WithAltScreen())
	if _, err := p.Run(); err != nil {
		return fmt.Errorf("config selector: %w", err)
	}
	return nil
}
