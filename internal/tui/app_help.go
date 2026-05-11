// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"strings"

	"github.com/charmbracelet/lipgloss"

	"github.com/huichen/xihu/internal/extensions"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) showHelpOverlay() {
	helpText := m.buildHelpOverlay()
	w := m.width - 8
	if w < 40 {
		w = 40
	}
	if w > 80 {
		w = 80
	}
	h := m.height - 4
	if h < 10 {
		h = 10
	}
	m.overlay.ShowScrollableText(helpText, w, h)
}

func (m *AppModel) buildHelpOverlay() string {
	var sb strings.Builder

	// Title
	titleStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color(m.theme.Accent)).
		Bold(true)
	sb.WriteString(titleStyle.Render("Keyboard Shortcuts"))
	sb.WriteString("\n\n")

	// Keybindings by category — resolved from KeybindingsManager (user-customizable)
	categoryOrder := []string{"global", "editor", "tools"}
	categoryTitles := map[string]string{
		"global": "Global",
		"editor": "Editor",
		"tools":  "Tools",
	}

	// Group resolved bindings by category
	groups := make(map[string][]ResolvedBinding)
	if m.keybindings != nil {
		for _, b := range m.keybindings.GetResolvedBindings() {
			if b.Category == "" {
				continue
			}
			groups[b.Category] = append(groups[b.Category], b)
		}
	}

	keyStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#89b4fa")).
		Width(22)
	descStyle := lipgloss.NewStyle().
		Foreground(lipgloss.Color("#abb2bf"))

	for _, cat := range categoryOrder {
		bindings, ok := groups[cat]
		if !ok || len(bindings) == 0 {
			continue
		}
		sb.WriteString(titleStyle.Render("▸ " + categoryTitles[cat]))
		sb.WriteByte('\n')
		for _, b := range bindings {
			keyStr := strings.Join(b.Keys, " / ")
			sb.WriteString("  ")
			sb.WriteString(keyStyle.Render(keyStr))
			sb.WriteString(descStyle.Render(b.Description))
			sb.WriteByte('\n')
		}
		sb.WriteByte('\n')
	}

	// Loaded resources
	sb.WriteString(titleStyle.Render("▸ Loaded Resources"))
	sb.WriteByte('\n')

	// Skills (TS pi-mono: showLoadedResources with source grouping)
	if len(m.Skills) > 0 {
		skillNames := make([]string, len(m.Skills))
		for i, s := range m.Skills {
			skillNames[i] = s.Name
		}
		sb.WriteString("  [Skills]")
		sb.WriteByte('\n')
		sb.WriteString("    " + strings.Join(skillNames, ", "))
	} else {
		sb.WriteString("  [Skills] none")
	}
	sb.WriteByte('\n')

	// Extensions
	if m.extRunner != nil {
		loaded := m.extRunner.Initialized()
		if len(loaded) > 0 {
			names := make([]string, len(loaded))
			for i, e := range loaded {
				names[i] = e.Name()
			}
			sb.WriteString("  [Extensions]")
			sb.WriteByte('\n')
			sb.WriteString("    " + strings.Join(names, ", "))
		} else {
			sb.WriteString("  [Extensions] none")
		}
	} else if len(m.Extensions) > 0 {
		sb.WriteString("  [Extensions] " + strings.Join(m.Extensions, ", "))
	} else {
		sb.WriteString("  [Extensions] none")
	}
	sb.WriteByte('\n')

	// Extension commands
	if m.extRunner != nil {
		extCmds := extensions.GetAllSlashCommands()
		if len(extCmds) > 0 {
			cmdNames := make([]string, 0, len(extCmds))
			for name := range extCmds {
				cmdNames = append(cmdNames, name)
			}
			sb.WriteString("  [Extension Cmds]")
			sb.WriteByte('\n')
			sb.WriteString("    " + strings.Join(cmdNames, ", "))
		} else {
			sb.WriteString("  [Extension Cmds] none")
		}
		sb.WriteByte('\n')
	}

	// Prompt templates
	if len(m.promptTemplates) > 0 {
		ptNames := make([]string, len(m.promptTemplates))
		for i, pt := range m.promptTemplates {
			ptNames[i] = "/" + pt.Name
		}
		sb.WriteString("  [Prompts]")
		sb.WriteByte('\n')
		sb.WriteString("    " + strings.Join(ptNames, ", "))
	} else {
		sb.WriteString("  [Prompts] none (place .md files in ~/.xihu/prompts/ or .xihu/prompts/)")
	}
	sb.WriteByte('\n')
	// Themes
	sb.WriteString("  [Themes] default, light + custom themes")
	sb.WriteByte('\n')

	return sb.String()
}

// ─── Slash Command Autocomplete ────────────────────────────────────────────

// fuzzyMatchScore checks if all characters in query appear in text in order.
// Returns (matches, score) where lower score = better match.
