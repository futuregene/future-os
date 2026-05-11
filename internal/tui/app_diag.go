// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"fmt"
	"strings"

	"github.com/charmbracelet/lipgloss"

	"github.com/huichen/xihu/internal/extensions"
	"github.com/huichen/xihu/internal/prompt"
	"github.com/huichen/xihu/internal/skills"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) getExtDiagnostics() []extensions.ExtensionDiagnostic {
	if m.extRunner == nil {
		return nil
	}
	return m.extRunner.GetExtensionDiagnostics()
}

// getKBConflicts returns keybinding conflicts from the global keybindings manager.
func (m *AppModel) getKBConflicts() []KeybindingConflict {
	kb := GetKeybindings()
	if kb == nil {
		return nil
	}
	return kb.GetConflicts()
}

// showPostReloadDiagnostics shows diagnostics after /reload (TS pi-mono: showLoadedResources after reload).
func (m *AppModel) showPostReloadDiagnostics() {
	extDiags := m.getExtDiagnostics()
	kbConflicts := m.getKBConflicts()

	if len(extDiags) == 0 && len(kbConflicts) == 0 {
		return
	}
	m.showLoadedDiagnostics(nil, extDiags, kbConflicts)
}

// showLoadedDiagnostics renders skill collisions, extension diagnostics, and keybinding conflicts.
func (m *AppModel) showLoadedDiagnostics(skillCollisions []skills.SkillCollision, extDiags []extensions.ExtensionDiagnostic, kbConflicts []KeybindingConflict) {
	dimStyle := lipgloss.NewStyle().Faint(true)
	warningStyle := lipgloss.NewStyle().Foreground(lipgloss.Color(m.theme.Warning))

	if len(skillCollisions) > 0 {
		m.chat.AppendSystem(warningStyle.Render("[Skill conflicts]"))
		for _, c := range skillCollisions {
			m.chat.AppendSystem(warningStyle.Render(fmt.Sprintf("  %q collision:", c.Name)))
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    ✓ %s (%s)", c.WinnerPath, c.WinnerSource)))
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("    ✗ %s (%s) (skipped)", c.LoserPath, c.LoserSource)))
		}
	}
	if len(extDiags) > 0 {
		m.chat.AppendSystem(warningStyle.Render("[Extension issues]"))
		for _, d := range extDiags {
			prefix := "Error"
			if d.Type == "warning" {
				prefix = "Warning"
			}
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("  %s: %s (%s)", prefix, d.Message, d.Path)))
		}
	}
	if len(kbConflicts) > 0 {
		m.chat.AppendSystem(warningStyle.Render("[Keybinding conflicts]"))
		for _, c := range kbConflicts {
			bindingNames := make([]string, len(c.Bindings))
			for i, b := range c.Bindings {
				bindingNames[i] = string(b)
			}
			m.chat.AppendSystem(dimStyle.Render(fmt.Sprintf("  %s bound to: %s", c.Key, strings.Join(bindingNames, ", "))))
		}
	}
}

// detectPromptCollisions finds naming conflicts in prompt templates.
// Since user templates are loaded first then project templates (loaded later),
// the last template with a given name wins (project overrides user).
func (m *AppModel) detectPromptCollisions(templates []prompt.PromptTemplate) []PromptCollision {
	seen := make(map[string]int) // name -> index of first occurrence
	var collisions []PromptCollision
	for i, t := range templates {
		if firstIdx, exists := seen[t.Name]; exists {
			// Project (loaded later, i) overrides earlier template
			collisions = append(collisions, PromptCollision{
				Name:       t.Name,
				WinnerPath: t.Source,
				LoserPath:  templates[firstIdx].Source,
			})
			seen[t.Name] = i // update to new winner
		} else {
			seen[t.Name] = i
		}
	}
	return collisions
}

// detectThemeCollisions finds naming conflicts among discovered themes.
// Themes are discovered from ~/.xihu/themes/; the last theme with a given name wins.
func (m *AppModel) detectThemeCollisions() []ThemeCollision {
	paths, err := DiscoverThemes("")
	if err != nil || len(paths) == 0 {
		return nil
	}
	// Map theme name -> path
	seen := make(map[string]string)
	var collisions []ThemeCollision
	for _, p := range paths {
		t, err := LoadTheme(p)
		if err != nil || t.Name == "" {
			continue
		}
		if firstPath, exists := seen[t.Name]; exists {
			collisions = append(collisions, ThemeCollision{
				Name:       t.Name,
				WinnerPath: p,
				LoserPath:  firstPath,
			})
			seen[t.Name] = p // update to new winner
		} else {
			seen[t.Name] = p
		}
	}
	return collisions
}

// ─── Changelog ───────────────────────────────────────────────────────────────

// checkChangelog loads the changelog and shows new entries as a non-capturing banner.
