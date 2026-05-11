// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"encoding/json"
	"fmt"
	"strings"


	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/tui/components"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) rebuildChatFromSession() {
	if m.session == nil {
		return
	}
	m.chat.Clear()

	// Walk the tree from the current leaf to root
	leafID := session.EffectiveLeafID(m.session)
	chain := session.ForEachEntry(m.session.Entries, leafID)
	if len(chain) == 0 {
		return
	}

	// Process root-to-leaf, respecting compaction boundaries.
	// When a CompactionEntry is found, parse its first_kept_entry_id and
	// skip earlier entries (they were compacted away). The compaction
	// summary itself is rendered as a fresh card at the end, not here.
	skipUntil := ""
	for i := len(chain) - 1; i >= 0; i-- {
		entry := chain[i]

		if skipUntil != "" {
			if entry.ID == skipUntil {
				skipUntil = ""
			} else {
				continue
			}
		}

		if entry.Type == session.EntryTypeCompaction {
			if entry.Content != nil {
				var meta struct {
					FirstKeptEntryID string `json:"first_kept_entry_id"`
				}
				if err := json.Unmarshal(entry.Content, &meta); err == nil && meta.FirstKeptEntryID != "" {
					skipUntil = meta.FirstKeptEntryID
				}
			}
			continue
		}

		ce := sessionEntryToChatEntry(entry)
		if ce != nil {
			m.chat.AppendChatEntry(*ce)
		}
	}
}

// sessionEntryToChatEntry converts a session.SessionEntry to a ChatEntry for display.
// Returns nil for entries that should be skipped (e.g. labels, branch summaries).
func sessionEntryToChatEntry(entry session.SessionEntry) *components.ChatEntry {
	var contentBlocks []struct {
		Type string `json:"type"`
		Text string `json:"text"`
	}
	_ = json.Unmarshal(entry.Content, &contentBlocks)
	var textParts []string
	for _, block := range contentBlocks {
		if block.Type == "text" && block.Text != "" {
			textParts = append(textParts, block.Text)
		}
	}
	contentText := strings.Join(textParts, "\n")

	switch entry.Type {
	case session.EntryTypeUser:
		return &components.ChatEntry{Type: "user_message", Content: contentText, ID: entry.ID}
	case session.EntryTypeAssistant:
		if contentText == "" && len(entry.ToolCalls) > 0 {
			// Assistant entry with only tool calls — skip, tool entries carry the detail
			return nil
		}
		return &components.ChatEntry{Type: "text", Content: contentText, ID: entry.ID}
	case session.EntryTypeTool:
		return &components.ChatEntry{Type: "tool_result", Content: contentText, ID: entry.ID}
	case session.EntryTypeSystem, session.EntryTypeCompaction:
		// Show system and compaction messages in chat
		if contentText == "" {
			return nil
		}
		return &components.ChatEntry{Type: "system", Content: contentText, ID: entry.ID}
	case session.EntryTypeModelChange, session.EntryTypeThinkingLevelChange, session.EntryTypeSessionInfo:
		// Pi-mono hides these in chat (tree-only entries) — show as dim system message
		if contentText == "" {
			return nil
		}
		return &components.ChatEntry{Type: "system", Content: contentText, ID: entry.ID}
	case session.EntryTypeLabel:
		return nil // skip structural entries
	case session.EntryTypeBranchSummary:
		// Show branch summary as a system message with metadata
		if entry.BranchSummary != nil {
			summaryText := fmt.Sprintf("[branch] %s (from %s)", contentText, entry.BranchSummary.FromID)
			return &components.ChatEntry{Type: "custom_message", Content: summaryText, CustomType: "branch", ID: entry.ID}
		}
		return &components.ChatEntry{Type: "system", Content: "[branch] " + contentText, ID: entry.ID}
	case session.EntryTypeCustom:
		return &components.ChatEntry{Type: "system", Content: contentText, ID: entry.ID}
	case session.EntryTypeCustomMessage:
		return &components.ChatEntry{Type: "custom_message", Content: entry.Display, CustomType: entry.CustomType, ID: entry.ID}
	default:
		return nil
	}
}

