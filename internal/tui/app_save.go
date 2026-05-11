// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (
	"os"
	"path/filepath"
	"strings"

	"github.com/huichen/xihu/internal/settings"
)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) saveSettings() {
	home, err := os.UserHomeDir()
	if err != nil {
		return
	}
	path := filepath.Join(home, ".xihu", "settings.json")

	// Load existing settings to preserve unmanaged fields
	s, err := settings.LoadSettings(path)
	if err != nil || s == nil {
		s = &settings.Settings{}
	}

	// Apply runtime values
	s.Theme = m.theme.Name
	s.ThinkingLevel = m.thinkingLevel
	s.DoubleEscapeAction = m.doubleEscapeAction
	s.TreeFilterMode = m.defaultTreeFilter
	s.SteeringMode = m.steeringMode
	s.FollowUpMode = m.followUpMode
	s.Transport = m.transport
	s.EditorPaddingX = m.editorPadding
	s.AutocompleteMaxVisible = m.autocompleteMax

	// Bool pointer fields
	ac := m.autoCompact
	s.CompactionEnabled = &ac
	ht := m.chat.HideAllThinking
	s.HideThinkingBlock = &ht
	qs := m.quietStartup
	s.QuietStartup = &qs
	hc := m.showHardwareCursor
	s.ShowHardwareCursor = &hc
	sc := m.skillCommands
	s.EnableSkillCommands = &sc
	cc := m.collapseChangelog
	s.CollapseChangelog = &cc
	it := m.installTelemetry
	s.EnableInstallTelemetry = &it

	// Terminal settings
	tp := m.terminalProgress
	cs := m.clearOnShrink
	si := m.showImages
	s.Terminal = &settings.TerminalSettings{
		ShowTerminalProgress: &tp,
		ClearOnShrink:        &cs,
		ShowImages:           &si,
		ImageWidthCells:      m.imageWidthCells,
	}

	// Image settings
	ar := m.autoResizeImages
	bi := m.blockImages
	s.Images = &settings.ImageSettings{
		AutoResize:  &ar,
		BlockImages: &bi,
	}

	// Scoped models — save enabled models in display order
	var scopedList []string
	for _, model := range m.modelOrder {
		if m.scopedModels[model] {
			scopedList = append(scopedList, model)
		}
	}
	// Append any enabled models not in modelOrder
	inOrder := make(map[string]bool)
	for _, model := range m.modelOrder {
		inOrder[model] = true
	}
	for model := range m.scopedModels {
		if !inOrder[model] {
			scopedList = append(scopedList, model)
		}
	}
	s.ScopedModels = scopedList

	// Warnings
	ae := m.anthropicExtraUsage
	s.Warnings = &settings.WarningSettings{
		AnthropicExtraUsage: &ae,
	}

	settings.SaveSettings(path, s)
}

//// ApplyTheme applies a theme change immediately to all components (live reload).
func (m *AppModel) ApplyTheme(t *Theme) {
	if t == nil {
		return
	}
	m.theme = t

	// Update footer style
	fs := t.FooterStyle()
	fs = fs.Width(m.width)
	m.footer.SetStyle(fs, t.ContextGreen, t.ContextYellow, t.ContextRed)

	// Update editor input border (preserve thinking-level color overlay)
	m.input.SetSlashBorderColor(t.InputBorder)

	// Update glamour markdown style for light/dark themes
	if t.Name == "light" {
		m.chat.SetGlamourStyle("light")
	} else {
		m.chat.SetGlamourStyle("dark")
	}

	// Propagate theme colors to chat viewport (TS pi-mono: theme-driven colors)
	m.chat.SetTheme(t.Accent, t.Muted, t.Dim, t.Warning, t.Success, t.ErrorColor, t.ThinkingColor, t.ThinkingText, t.ToolPendingBg, t.ToolSuccessBg, t.ToolErrorBg)

	// Update session info to refresh display
	_, provider := parseModelString(m.agent.Loop().Model)
	modelName := m.agent.Loop().Model
	if idx := strings.Index(modelName, "/"); idx >= 0 {
		modelName = modelName[idx+1:]
	}
	m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), m.session.GetSessionName(), modelName, m.thinkingLevel, provider)

	// Try to persist theme to settings
	home, err := os.UserHomeDir()
	if err == nil {
		settingsPath := filepath.Join(home, ".xihu", "settings.json")
		s, _ := settings.LoadSettings(settingsPath)
		if s != nil {
			s.Theme = t.Name
			settings.SaveSettings(settingsPath, s)
		}
	}
}

// boolToStr renders a boolean as "true" or "false".
// fuzzyMatch returns true if each character in pattern appears in order in s (case-sensitive, caller lowercases).
func (m *AppModel) updateHeaderHints() {
}

// reload reloads settings, keybindings, and re-applies theme from disk.
// Mirrors pi-mono's handleReloadCommand + session.reload() + keybindings.reload().
