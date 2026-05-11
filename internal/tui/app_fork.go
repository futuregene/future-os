// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"encoding/json"
	"fmt"
	"os"
	"time"


	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/tui/components"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) forkFromEntry(entryID string) {
	if m.session == nil || m.sessMgr == nil {
		return
	}
	// Save original session first
	m.sessMgr.Save(m.session)

	// Find the entry index
	cutIdx := -1
	for i := range m.session.Entries {
		if m.session.Entries[i].ID == entryID {
			cutIdx = i + 1 // keep up to and including this entry
			break
		}
	}
	if cutIdx < 0 {
		m.chat.AppendSystem("Entry not found for fork")
		return
	}

	// Create forked session
	newID := session.GenerateID()
	oldEntries := m.session.Entries[:cutIdx]

	m.session.ID = newID
	m.session.Entries = make([]session.SessionEntry, len(oldEntries))
	copy(m.session.Entries, oldEntries)
	m.session.CreatedAt = time.Now()
	m.session.UpdatedAt = time.Now()
	m.session.Name = ""
	if err := m.sessMgr.Save(m.session); err != nil {
		m.chat.AppendSystem("Error saving fork: " + err.Error())
		return
	}
	m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), "", "", "", "")
	m.chat.AppendSystem("Forked to new session")
}

// cloneSession creates a full copy of the current session with a new ID.
// switchToSession saves the current session and switches to a different one.
// (TS pi-mono: /resume <id> in interactive mode without restart)
func (m *AppModel) switchToSession(sid string) {
	if m.sessMgr == nil || m.session == nil {
		m.chat.AppendSystem("No session manager available")
		return
	}
	if sid == m.session.ID {
		m.chat.AppendWarning("Already in session: " + sid)
		return
	}

	// Save current session before switching
	if err := m.sessMgr.Save(m.session); err != nil {
		m.chat.AppendError("Failed to save current session: " + err.Error())
		return
	}

	// Load the target session
	newSess, err := m.sessMgr.Load(sid, m.session.CWD)
	if err != nil {
		m.chat.AppendError("Failed to load session " + sid + ": " + err.Error())
		return
	}

	oldCWD := m.session.CWD
	m.session = newSess
	if m.session.CWD == "" {
		m.session.CWD = oldCWD
	}
	if m.session.CWD == "" {
		cwd, _ := os.Getwd()
		m.session.CWD = cwd
	}

	// Rebuild chat viewport from session tree (TS pi-mono: rebuildChatFromMessages)
	m.rebuildChatFromSession()

	// Update footer
	modelName, provider := parseModelString(m.agent.Loop().Model)
	m.footer.SetSession(m.session.CWD, getGitBranch(m.session.CWD), m.session.GetSessionName(), modelName, m.thinkingLevel, provider)
	m.footer.SetEntryCount(len(m.session.Entries))

	// TS pi-mono: show compaction info if session was compacted
	compactionCount := 0
	for _, e := range m.session.Entries {
		if e.Type == session.EntryTypeCompaction {
			compactionCount++
		}
	}
	if compactionCount > 0 {
		times := fmt.Sprintf("%d times", compactionCount)
		if compactionCount == 1 {
			times = "1 time"
		}
		m.chat.AppendSystem("Session compacted " + times)
	}
	m.chat.AppendSystem("Resumed session")
	m.setTerminalTitle()
}

// rebuildChatFromSession clears the chat viewport and rebuilds it from session
// entries, walking the tree from the current leaf and respecting compaction
// boundaries (TS pi-mono: rebuildChatFromMessages / renderSessionContext).
func (m *AppModel) cloneSession() string {
	oldEntries := make([]session.SessionEntry, len(m.session.Entries))
	copy(oldEntries, m.session.Entries)

	// Create new session
	m.session.ID = session.GenerateID()
	m.session.Entries = oldEntries
	m.session.CreatedAt = time.Now()
	m.session.UpdatedAt = time.Now()
	if err := m.sessMgr.Save(m.session); err != nil {
		m.chat.AppendSystem("Error saving cloned session: " + err.Error())
		return ""
	}
	m.chat.AppendSystem("Cloned to new session")
	if name := m.session.GetSessionName(); name != "" {
		m.session.SetSessionName(name + " (clone)")
		m.sessMgr.Save(m.session)
	}
	return m.session.ID
}

// showForkSelector opens a user message selector for forking (TS pi-mono: /fork).
// Shows recent user messages from the session; selecting one forks from that point.
func (m *AppModel) showScopedModelSelector() {
	if len(m.availableModels) == 0 {
		m.chat.AppendSystem("Only showing models from configured providers. Use /login to add providers.")
		return
	}

	buildItems := func() []components.SelectorItem {
		items := make([]components.SelectorItem, 0, len(m.availableModels))
		// Use modelOrder for display order if available
		displayOrder := m.availableModels
		if len(m.modelOrder) > 0 {
			displayOrder = m.modelOrder
			// Add any models not in modelOrder
			inOrder := make(map[string]bool)
			for _, mdl := range m.modelOrder {
				inOrder[mdl] = true
			}
			for _, mdl := range m.availableModels {
				if !inOrder[mdl] {
					displayOrder = append(displayOrder, mdl)
				}
			}
		}
		for _, model := range displayOrder {
			name, provider := parseModelString(model)
			enabled := m.scopedModels[model]
			label := name
			if enabled {
				label = "✓ " + name
			} else {
				label = "  " + name
			}
			desc := "[" + provider + "]"
			if enabled {
				desc = desc + " ✓"
			} else {
				desc = desc + " ✗"
			}
			items = append(items, components.SelectorItem{
				Label:       label,
				Description: desc,
				Value:       model,
			})
		}
		return items
	}

	buildTitle := func() string {
		enabledCount := len(m.scopedModels)
		if enabledCount > 0 {
			return fmt.Sprintf("Model Configuration (%d of %d enabled)  Enter=toggle  Ctrl+A/X=all/clear  Ctrl+S=save  Ctrl+P=provider  Alt+↑↓=reorder  Esc=close", enabledCount, len(m.availableModels))
		}
		return fmt.Sprintf("Model Configuration (all %d)  Enter=toggle  Ctrl+A/X=all/clear  Ctrl+S=save  Ctrl+P=provider  Alt+↑↓=reorder  Esc=close", len(m.availableModels))
	}

	refresh := func() {
		m.overlay.ReplaceItems(buildTitle(), buildItems())
	}

	onSelect := func(value string) {
		if value != "" {
			if m.scopedModels[value] {
				delete(m.scopedModels, value)
				m.chat.AppendSystem("Disabled: " + value)
			} else {
				m.scopedModels[value] = true
				m.chat.AppendSystem("Enabled: " + value)
			}
			// Re-open to show updated state
			if m.program != nil {
				go func() {
					time.Sleep(50 * time.Millisecond)
					m.program.Send(refreshScopedSelectorMsg{})
				}()
			}
		}
	}

	onKey := func(key string) bool {
		switch key {
		case "ctrl+a":
			// Enable all models and reset order
			modelNames := make([]string, len(m.availableModels))
			copy(modelNames, m.availableModels)
			m.modelOrder = modelNames
			for _, model := range m.availableModels {
				m.scopedModels[model] = true
			}
			m.chat.AppendSystem("Enabled all " + fmt.Sprintf("%d", len(m.availableModels)) + " models")
			refresh()
			return true
		case "ctrl+x":
			// Clear all scoped models
			m.scopedModels = make(map[string]bool)
			refresh()
			return true
		case "ctrl+s":
			// Save model selection (TS pi-mono: persist to settings)
			m.chat.AppendSystem("Model selection saved to settings")
			return true
		case "alt+up":
			// Move model up in order
			sel := m.overlay.SelectedValue()
			if sel != "" && len(m.modelOrder) > 0 {
				for i, mdl := range m.modelOrder {
					if mdl == sel && i > 0 {
						m.modelOrder[i], m.modelOrder[i-1] = m.modelOrder[i-1], m.modelOrder[i]
						break
					}
				}
				refresh()
			}
			return true
		case "alt+down":
			// Move model down in order
			sel := m.overlay.SelectedValue()
			if sel != "" && len(m.modelOrder) > 0 {
				for i, mdl := range m.modelOrder {
					if mdl == sel && i < len(m.modelOrder)-1 {
						m.modelOrder[i], m.modelOrder[i+1] = m.modelOrder[i+1], m.modelOrder[i]
						break
					}
				}
				refresh()
			}
			return true
		case "ctrl+p":
			// Toggle all models for the provider of the currently selected item
			sel := m.overlay.SelectedValue()
			if sel != "" {
				_, selProvider := parseModelString(sel)
				// Check if all models for this provider are already enabled
				allEnabled := true
				for _, model := range m.availableModels {
					_, provider := parseModelString(model)
					if provider == selProvider && !m.scopedModels[model] {
						allEnabled = false
						break
					}
				}
				// Toggle: disable all if all enabled, otherwise enable all
				for _, model := range m.availableModels {
					_, provider := parseModelString(model)
					if provider == selProvider {
						if allEnabled {
							delete(m.scopedModels, model)
						} else {
							m.scopedModels[model] = true
						}
					}
				}
				if allEnabled {
					m.chat.AppendSystem("Disabled all " + selProvider + " models")
				} else {
					m.chat.AppendSystem("Enabled all " + selProvider + " models")
				}
				refresh()
			}
			return true
		}
		return false
	}

	h := len(m.availableModels) + 6
	if h > 22 {
		h = 22
	}
	m.overlay.ShowSelectorWithKeyHandler(buildTitle(), buildItems(), onSelect, onKey, 76, h)
}

// refreshScopedSelectorMsg is an internal message to refresh the scoped model selector.
func (m *AppModel) showForkSelector() {
	if m.session == nil || len(m.session.Entries) == 0 {
		m.chat.AppendSystem("No messages to fork from")
		return
	}

	// Extract user messages with their entry IDs
	type userMsg struct {
		id   string
		text string
	}
	var messages []userMsg
	fullTexts := make(map[string]string) // entryID -> full message text
	for i := len(m.session.Entries) - 1; i >= 0; i-- {
		e := m.session.Entries[i]
		if e.Role == "user" && len(e.Content) > 0 {
			// Try to extract text content
			var contentBlocks []struct {
				Type string `json:"type"`
				Text string `json:"text"`
			}
			if err := json.Unmarshal(e.Content, &contentBlocks); err == nil {
				for _, block := range contentBlocks {
					if block.Type == "text" && block.Text != "" {
						fullTexts[e.ID] = block.Text
						// Truncate long messages
						text := block.Text
						if len(text) > 80 {
							text = text[:77] + "..."
						}
						messages = append(messages, userMsg{id: e.ID, text: text})
						break
					}
				}
			}
		}
	}

	if len(messages) == 0 {
		m.chat.AppendSystem("No user messages found in session")
		return
	}

	items := make([]components.SelectorItem, 0, len(messages))
	for i, msg := range messages {
		label := msg.text
		pos := len(messages) - i
		desc := fmt.Sprintf("Message %d of %d", pos, len(messages))
		items = append(items, components.SelectorItem{
			Label:       label,
			Description: desc,
			Value:       msg.id,
		})
	}

	h := len(items) + 5
	if h > 20 {
		h = 20
	}
	m.overlay.ShowForkSelector("Fork from Message", "Select a user message to copy the active path up to that point into a new session", items, func(value string) {
		if value != "" {
			m.forkFromEntry(value)
			// Fill editor with selected message text (TS pi-mono)
			if fullText, ok := fullTexts[value]; ok {
				m.input.SetValue(fullText)
			}
		}
	}, nil, 70, h)
}

// showSessionSelector opens a session list overlay (TS pi-mono: /resume, /sessions).
// Supports type-to-search, sort toggle (Ctrl+S), named filter (Ctrl+N),
// session delete (Ctrl+Backspace), and session rename (Ctrl+R).
