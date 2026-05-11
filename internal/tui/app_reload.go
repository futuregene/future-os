// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (



	"github.com/huichen/xihu/internal/events"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/settings"
	"github.com/huichen/xihu/internal/tui/components"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) reload() {
	// Reload settings from global + project config files
	newSettings, err := settings.LoadAll()
	if err != nil {
		m.chat.AppendError("Reload failed: " + err.Error())
		return
	}

	// Reload keybindings from ~/.xihu/keybindings.json
	if m.keybindings != nil {
		userKB, _ := LoadUserBindings()
		m.keybindings.Reload(userKB)
		// Update tool toggle key hint with potentially changed keybinding
		if tk := formatKeyStr(m.keybindings, GlobalToggleTools); tk != "" {
			m.chat.SetToolToggleKey(tk)
		}
	}
	m.updateHeaderHints()

	// Re-apply settings (same pattern as constructor)
	if newSettings != nil {
		m.settingsObj = newSettings
		if newSettings.DoubleEscapeAction != "" {
			m.doubleEscapeAction = newSettings.DoubleEscapeAction
		}
		if newSettings.TreeFilterMode != "" {
			m.defaultTreeFilter = newSettings.TreeFilterMode
		}
		if newSettings.QuietStartup != nil {
			m.quietStartup = *newSettings.QuietStartup
		}
		if newSettings.CompactionEnabled != nil {
			m.autoCompact = *newSettings.CompactionEnabled
		}
		if newSettings.HideThinkingBlock != nil {
			m.chat.HideAllThinking = *newSettings.HideThinkingBlock
		}
		if newSettings.SteeringMode != "" {
			m.steeringMode = newSettings.SteeringMode
			if m.agent != nil {
				m.agent.Loop().SteeringQueue.Mode = newSettings.SteeringMode
			}
		}
		if newSettings.FollowUpMode != "" {
			m.followUpMode = newSettings.FollowUpMode
		}
		if newSettings.Transport != "" {
			m.transport = newSettings.Transport
		}
		if newSettings.ShowHardwareCursor != nil {
			m.showHardwareCursor = *newSettings.ShowHardwareCursor
		}
		if newSettings.Terminal != nil {
			if newSettings.Terminal.ShowTerminalProgress != nil {
				m.terminalProgress = *newSettings.Terminal.ShowTerminalProgress
			}
			if newSettings.Terminal.ClearOnShrink != nil {
				m.clearOnShrink = *newSettings.Terminal.ClearOnShrink
			}
			if newSettings.Terminal.ShowImages != nil {
				m.showImages = *newSettings.Terminal.ShowImages
				m.chat.SetShowImages(m.showImages)
			}
			if newSettings.Terminal.ImageWidthCells > 0 {
				m.imageWidthCells = newSettings.Terminal.ImageWidthCells
				m.chat.SetImageWidth(m.imageWidthCells)
			}
		}
		if newSettings.Images != nil {
			if newSettings.Images.AutoResize != nil {
				m.autoResizeImages = *newSettings.Images.AutoResize
			}
			if newSettings.Images.BlockImages != nil {
				m.blockImages = *newSettings.Images.BlockImages
			}
		}
		if newSettings.EnableSkillCommands != nil {
			m.skillCommands = *newSettings.EnableSkillCommands
		}
		if newSettings.CollapseChangelog != nil {
			m.collapseChangelog = *newSettings.CollapseChangelog
		}
		if newSettings.EnableInstallTelemetry != nil {
			m.installTelemetry = *newSettings.EnableInstallTelemetry
		}
		if newSettings.EditorPaddingX > 0 {
			m.editorPadding = newSettings.EditorPaddingX
			m.input.SetPaddingX(m.editorPadding)
		}
		if newSettings.AutocompleteMaxVisible > 0 {
			m.autocompleteMax = newSettings.AutocompleteMaxVisible
			m.autocomplete.SetMaxVisible(m.autocompleteMax)
		}
		if newSettings.Warnings != nil && newSettings.Warnings.AnthropicExtraUsage != nil {
			m.anthropicExtraUsage = *newSettings.Warnings.AnthropicExtraUsage
		}
		if newSettings.LastChangelogVersion != "" {
			m.lastChangelogVersion = newSettings.LastChangelogVersion
		}
		// Scoped models
		if len(newSettings.ScopedModels) > 0 {
			m.scopedModels = make(map[string]bool)
			for _, mod := range newSettings.ScopedModels {
				m.scopedModels[mod] = true
			}
		}
		// Theme - reload if changed
		if newSettings.Theme != "" && m.theme != nil && newSettings.Theme != m.theme.Name {
			switch newSettings.Theme {
			case "dark":
				m.ApplyTheme(DefaultTheme())
			case "light":
				m.ApplyTheme(LightTheme())
			default:
				// Try to load custom theme
				customPaths, _ := DiscoverThemes("")
				for _, p := range customPaths {
					t, err := LoadTheme(p)
					if err == nil && t.Name == newSettings.Theme {
						m.ApplyTheme(t)
						break
					}
				}
			}
		} else if m.theme != nil {
			// Re-apply current theme to refresh chat/editor colors
			m.ApplyTheme(m.theme)
		}

		// Re-propagate thinking border color
		if m.theme != nil {
			m.chat.SetThinkingBorderColor(m.theme.ThinkingBorderColor(m.thinkingLevel))
			m.input.SetBorderColor(m.theme.ThinkingBorderColor(m.thinkingLevel))
		}
	}
}

// triggerManualCompaction runs manual compaction via the configured TransformContext.
// Emits CompactionStart("manual") / CompactionEnd events so the UI shows correct messages.
func (m *AppModel) triggerManualCompaction() {
	if m.compacting {
		m.chat.AppendError("Compaction already in progress")
		return
	}
	if m.agent == nil || m.agent.Loop().Config.TransformContext == nil {
		m.chat.AppendError("Compaction is not configured. Set compaction_reserve_tokens in settings.")
		return
	}
	if m.session == nil {
		m.chat.AppendError("No active session to compact")
		return
	}

	// TS pi-mono: check if there are enough messages for compaction
	messageCount := 0
	for _, e := range m.session.Entries {
		if e.Type == session.EntryTypeUser || e.Type == session.EntryTypeAssistant {
			messageCount++
		}
	}
	if messageCount < 2 {
		m.chat.AppendWarning("Nothing to compact (no messages yet)")
		return
	}

	m.compacting = true
	m.compactionQueue = nil

	// Emit manual compaction start event
	if m.eventBus != nil {
		m.eventBus.Emit(events.CompactionStart("manual"))
	}

	// Build messages from session entries and run compaction
	leafID := session.EffectiveLeafID(m.session)
	messages := session.BuildContextFromLeaf(m.session.Entries, leafID)
	compactedMessages := m.agent.Loop().Config.TransformContext(messages, "manual")

	if len(compactedMessages) < len(messages) {
		// Compaction happened — get result from agent
		tokensBefore := 0
		summary := ""
		firstKeptID := ""
		if m.agent.Loop().LastCompactionResult != nil {
			tokensBefore = m.agent.Loop().LastCompactionResult.TokensBefore
			summary = m.agent.Loop().LastCompactionResult.Summary
			firstKeptID = m.agent.Loop().LastCompactionResult.FirstKeptEntryID
			m.agent.Loop().LastCompactionResult = nil
		}

		// Record compaction as a session entry BEFORE emitting event
		// (TS pi-mono: session entry must exist before rebuildChatFromMessages)
		if m.session != nil && m.sessMgr != nil {
			parentID := session.EffectiveLeafID(m.session)
			entry := session.CompactionEntry(summary, firstKeptID, parentID)
			if err := m.sessMgr.AddEntry(m.session, entry); err != nil {
				m.chat.AppendSystem("Warning: failed to save compaction entry: " + err.Error())
			}
		}

		// Emit event (handler will rebuild chat from session, TS pi-mono alignment)
		if m.eventBus != nil {
			m.eventBus.Emit(events.CompactionEnd(tokensBefore, summary, false, "manual"))
		}
	} else {
		if m.eventBus != nil {
			m.eventBus.Emit(events.CompactionEnd(0, "", false, "manual"))
		}
	}

	m.compacting = false

	// Flush queued messages
	if len(m.compactionQueue) > 0 {
		queued := m.compactionQueue
		m.compactionQueue = nil
		for _, qm := range queued {
			m.program.Send(components.SubmitMsg(qm))
		}
	}
}

// saveSettings persists current runtime settings to the global settings file.
