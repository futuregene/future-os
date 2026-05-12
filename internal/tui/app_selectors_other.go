// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"fmt"
	"path/filepath"
	"strings"
	"time"


	"github.com/huichen/xihu/internal/tui/components"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) cycleThinking() {
	modelName := m.agent.Loop().Model
	if idx := strings.Index(modelName, "/"); idx >= 0 {
		modelName = modelName[idx+1:]
	}
	if !supportsThinking(modelName) {
		m.chat.AppendSystem("Current model does not support thinking")
		return
	}

	current := m.thinkingLevel
	if current == "" {
		current = "off"
	}

	// Find next level
	next := "off" // default wrap-around
	for i, level := range thinkingLevels {
		if level == current && i+1 < len(thinkingLevels) {
			next = thinkingLevels[i+1]
			break
		}
	}

	m.thinkingLevel = next

	// Propagate to LLM client via engine
	if eng := m.agent.Engine(); eng != nil {
		eng.SetThinkingLevel(next)
	}

	// Update footer display
	_, provider := parseModelString(m.agent.Loop().Model)
	m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), m.session.GetSessionName(), modelName, next, provider)
	m.footer.SetHasReasoning(supportsThinking(modelName))
	m.footer.SetEntryCount(len(m.session.Entries))
	m.input.SetBorderColor(m.theme.ThinkingBorderColor(next))

	m.chat.AppendSystem("Thinking level: " + next)
}

// ─── Git Branch Detection ──────────────────────────────────────────────────

// getGitBranch returns the current git branch name for the given directory.
func (m *AppModel) showThemeSelector() {
	currentName := "dark"
	if m.theme != nil {
		currentName = m.theme.Name
	}
	items := []components.SelectorItem{
		{Label: "Dark (default)", Description: "Catppuccin Mocha inspired dark theme", Value: "dark"},
		{Label: "Light", Description: "Catppuccin Latte inspired light theme", Value: "light"},
	}

	// Discover custom themes from ~/.pi/themes/
	customPaths, _ := DiscoverThemes("")
	for _, p := range customPaths {
		t, err := LoadTheme(p)
		if err != nil || t.Name == "" {
			continue
		}
		// Skip built-in names
		if t.Name == "dark" || t.Name == "light" {
			continue
		}
		name := filepath.Base(p)
		name = name[:len(name)-len(filepath.Ext(name))]
		marker := ""
		if t.Name == currentName {
			marker = " (current)"
		}
		items = append(items, components.SelectorItem{
			Label:       name + marker,
			Description: "Custom theme: " + p,
			Value:       "custom:" + p,
		})
	}

	h := len(items) + 4
	if h < 7 {
		h = 7
	}
	if h > 16 {
		h = 16
	}
	m.overlay.ShowSelector("Select Theme (current: "+currentName+")", items, func(value string) {
		switch {
		case value == "dark":
			m.ApplyTheme(DefaultTheme())
		case value == "light":
			m.ApplyTheme(LightTheme())
		case strings.HasPrefix(value, "custom:"):
			path := strings.TrimPrefix(value, "custom:")
			t, err := LoadTheme(path)
			if err != nil {
				m.chat.AppendError(fmt.Sprintf("Failed to load theme \"%s\": %s", path, err.Error()))
				return
			}
			m.ApplyTheme(t)
		}
	}, 56, h)
}

// showSettingsSelector shows current settings in an interactive overlay (TS pi-mono: /settings).
// Selecting an item toggles/changes the setting and re-opens for live feedback.
func (m *AppModel) showThinkingSelector() {
	current := m.thinkingLevel
	if current == "" {
		current = "off"
	}

	var items []components.SelectorItem
	for i, level := range thinkingLevels {
		desc := ""
		switch level {
		case "off":
			desc = "No reasoning"
		case "minimal":
			desc = "Very brief reasoning (~1k tokens)"
		case "low":
			desc = "Light reasoning (~2k tokens)"
		case "medium":
			desc = "Moderate reasoning (~8k tokens)"
		case "high":
			desc = "Deep reasoning (~16k tokens)"
		case "xhigh":
			desc = "Maximum reasoning (~32k tokens)"
		}
		label := level
		if level == current {
			label = "✓ " + level
		}
		items = append(items, components.SelectorItem{
			Label:       label,
			Description: fmt.Sprintf("[%d/%d] %s", i+1, len(thinkingLevels), desc),
			Value:       level,
		})
	}

	h := len(items) + 4
	if h > 14 {
		h = 14
	}
	m.overlay.ShowSelector("Thinking Level", items, func(value string) {
		if value != "" && value != m.thinkingLevel {
			m.thinkingLevel = value
			m.saveSettings()
			// Propagate to LLM client via engine
			if eng := m.agent.Engine(); eng != nil {
				eng.SetThinkingLevel(value)
			}
			// Update footer
			_, provider := parseModelString(m.agent.Loop().Model)
			modelName := m.agent.Loop().Model
			if idx := strings.Index(modelName, "/"); idx >= 0 {
				modelName = modelName[idx+1:]
			}
			m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), m.session.GetSessionName(), modelName, value, provider)
			m.footer.SetHasReasoning(supportsThinking(modelName))
			m.input.SetBorderColor(m.theme.ThinkingBorderColor(value))
			m.chat.AppendSystem("Thinking level: " + value)
		}
		// Re-show settings after closing thinking selector
		go func() {
			time.Sleep(50 * time.Millisecond)
			if m.program != nil {
				m.program.Send(refreshSettingsMsg{})
			}
		}()
	}, 60, h)
}

// showModelSelector opens a model selector overlay (TS pi-mono: Ctrl+L).
// Shows model name, provider, context window size, and pricing.
func (m *AppModel) showWarningsSelector() {
	items := []components.SelectorItem{
		{Label: "Anthropic Extra Usage: " + boolToStr(m.anthropicExtraUsage), Description: "Warn when API responses include anthropic extra usage pricing", Value: "anthropic_extra_usage"},
	}

	h := len(items) + 4
	onSelect := func(value string) {
		switch value {
		case "anthropic_extra_usage":
			m.anthropicExtraUsage = !m.anthropicExtraUsage
		}
		m.saveSettings()
		if m.program != nil {
			go func() {
				time.Sleep(50 * time.Millisecond)
				m.program.Send(refreshWarningsMsg{})
			}()
		}
	}
	m.overlay.ShowSelectorStayOnSelect("Warnings (Enter to toggle, Esc to back)", items, onSelect, nil, 54, h)
}

// refreshWarningsMsg refreshes the warnings submenu.
