package session

import (
	"encoding/json"
	"fmt"
	"time"

	"github.com/huichen/xihu/pkg/types"
)

// ModelChangeEntry creates a model_change entry.
func ModelChangeEntry(model, provider, parentID string) SessionEntry {
	return SessionEntry{
		ID:        GenerateEntryID(),
		ParentID:  parentID,
		Type:      EntryTypeModelChange,
		Model:     model,
		Provider:  provider,
		Timestamp: time.Now(),
		Content:   json.RawMessage(fmt.Sprintf(`{"new_model":%q,"provider":%q}`, model, provider)),
	}
}

// ThinkingLevelChangeEntry creates a thinking_level_change entry.
func ThinkingLevelChangeEntry(level, parentID string) SessionEntry {
	return SessionEntry{
		ID:            GenerateEntryID(),
		ParentID:      parentID,
		Type:          EntryTypeThinkingLevelChange,
		ThinkingLevel: level,
		Timestamp:     time.Now(),
		Content:       json.RawMessage(fmt.Sprintf(`{"new_thinking_level":%q}`, level)),
	}
}

// BranchSummaryEntry creates a branch_summary entry.
func BranchSummaryEntry(summary, fromID, parentID string, fromHook bool) SessionEntry {
	meta := &BranchSummaryMeta{FromID: fromID}
	if fromHook {
		meta.FromHook = &fromHook
	}
	metaJSON, _ := json.Marshal(meta)
	return SessionEntry{
		ID:            GenerateEntryID(),
		ParentID:      parentID,
		Type:          EntryTypeBranchSummary,
		Summary:       summary,
		BranchSummary: meta,
		Content:       json.RawMessage(metaJSON),
		Timestamp:     time.Now(),
	}
}

// CustomEntry creates a custom entry for extension data storage.
func CustomEntry(customType string, data json.RawMessage, parentID string) SessionEntry {
	return SessionEntry{
		ID:         GenerateEntryID(),
		ParentID:   parentID,
		Type:       EntryTypeCustom,
		CustomType: customType,
		CustomData: data,
		Content:    data,
		Timestamp:  time.Now(),
	}
}

// CompactionEntry creates a compaction entry that summarizes history.
func CompactionEntry(summary string, firstKeptEntryID string, parentID string) SessionEntry {
	meta, _ := json.Marshal(map[string]string{
		"first_kept_entry_id": firstKeptEntryID,
	})
	return SessionEntry{
		ID:        GenerateEntryID(),
		ParentID:  parentID,
		Type:      EntryTypeCompaction,
		Summary:   summary,
		Content:   json.RawMessage(meta),
		Timestamp: time.Now(),
	}
}

// sessionInfoEntry creates a session_info metadata entry (first line in JSONL).
func sessionInfoEntry(s *Session) SessionEntry {
	content := fmt.Sprintf(
		`{"cwd":%q,"base_url":%q,"created_at":%q,"updated_at":%q`,
		s.CWD, s.BaseURL, s.CreatedAt.Format(time.RFC3339), s.UpdatedAt.Format(time.RFC3339),
	)
	if s.Name != "" {
		content += fmt.Sprintf(`,"name":%q`, s.Name)
	}
	content += "}"
	return SessionEntry{
		ID:        s.ID,
		Type:      EntryTypeSessionInfo,
		Model:     s.Model,
		Timestamp: s.CreatedAt,
		Content:   json.RawMessage(content),
	}
}

// sessionInfoEntryByName creates a session_info entry that only sets the name field.
func sessionInfoEntryByName(name, parentID string) SessionEntry {
	return SessionEntry{
		ID:        GenerateEntryID(),
		ParentID:  parentID,
		Type:      EntryTypeSessionInfo,
		Timestamp: time.Now(),
		Content:   json.RawMessage(fmt.Sprintf(`{"name":%q}`, name)),
	}
}

// parseSessionInfo populates session metadata from a session_info entry.
func parseSessionInfo(entry SessionEntry, s *Session) {
	s.ID = entry.ID
	s.Model = entry.Model
	var meta struct {
		CWD       string `json:"cwd"`
		BaseURL   string `json:"base_url"`
		CreatedAt string `json:"created_at"`
		UpdatedAt string `json:"updated_at"`
		Name      string `json:"name"`
	}
	if err := json.Unmarshal(entry.Content, &meta); err == nil {
		s.CWD = meta.CWD
		s.BaseURL = meta.BaseURL
		s.Name = meta.Name
		if t, err := time.Parse(time.RFC3339, meta.CreatedAt); err == nil {
			s.CreatedAt = t
		}
		if t, err := time.Parse(time.RFC3339, meta.UpdatedAt); err == nil {
			s.UpdatedAt = t
		}
	}
	if s.CreatedAt.IsZero() {
		s.CreatedAt = entry.Timestamp
	}
}

// ForEachEntry walks entries from a given entry ID to root, calling fn for each.
// Returns the chain in leaf-to-root order.
func ForEachEntry(entries []SessionEntry, fromID string) []SessionEntry {
	byID := make(map[string]*SessionEntry, len(entries))
	for i := range entries {
		byID[entries[i].ID] = &entries[i]
	}

	var chain []SessionEntry
	currentID := fromID
	visited := make(map[string]bool)
	for currentID != "" {
		if visited[currentID] {
			break
		}
		visited[currentID] = true
		entry, ok := byID[currentID]
		if !ok {
			break
		}
		chain = append(chain, *entry)
		currentID = entry.ParentID
	}
	return chain
}

// BuildContext walks entries from the latest leaf to root, handles compaction
// entries (uses Summary field), and returns an ordered []types.Message suitable
// for LLM context. Messages are in chronological order (root to leaf).
func BuildContext(entries []SessionEntry) []types.Message {
	if len(entries) == 0 {
		return nil
	}

	// Index entries by ID for fast lookup
	byID := make(map[string]*SessionEntry, len(entries))
	for i := range entries {
		byID[entries[i].ID] = &entries[i]
	}

	// Find leaf entries: entries that are NOT a parent of any other entry
	parents := make(map[string]bool)
	for i := range entries {
		if entries[i].ParentID != "" {
			parents[entries[i].ParentID] = true
		}
	}

	// Find the latest leaf
	var latestLeaf *SessionEntry
	for i := range entries {
		if parents[entries[i].ID] {
			continue // has children, not a leaf
		}
		if latestLeaf == nil || entries[i].Timestamp.After(latestLeaf.Timestamp) {
			latestLeaf = &entries[i]
		}
	}

	if latestLeaf == nil {
		// All entries have children (unusual) — use the latest entry
		latest := entries[len(entries)-1]
		latestLeaf = &latest
	}

	// Walk from leaf to root
	chain := ForEachEntry(entries, latestLeaf.ID)

	// Build messages from root to leaf, handling compaction
	var messages []types.Message
	skipUntil := "" // if set, skip entries until this ID is found

	// Process chain in reverse (root to leaf)
	for i := len(chain) - 1; i >= 0; i-- {
		entry := chain[i]

		// Handle skip logic (from compaction)
		if skipUntil != "" {
			if entry.ID == skipUntil {
				skipUntil = "" // found the first kept entry, resume
			} else {
				continue // still skipping
			}
		}

		switch entry.Type {
		case EntryTypeCompaction:
			// Emit a system message with the summary
			sysContent, _ := json.Marshal([]types.TextContent{
				{Type: "text", Text: entry.Summary},
			})
			messages = append(messages, types.Message{
				Role:    "system",
				Content: sysContent,
			})

			// Parse first_kept_entry_id to resume from there
			var meta struct {
				FirstKeptEntryID string `json:"first_kept_entry_id"`
			}
			if err := json.Unmarshal(entry.Content, &meta); err == nil && meta.FirstKeptEntryID != "" {
				skipUntil = meta.FirstKeptEntryID
			}

		case EntryTypeModelChange, EntryTypeLabel,
			EntryTypeSessionInfo, EntryTypeSystem:
			// Skip metadata entries — they don't produce messages

		default:
			// Convert to types.Message (user, assistant, tool)
			msg := entryToMessage(entry)
			messages = append(messages, msg)
		}
	}

	return messages
}

// BuildContextFromLeaf is like BuildContext but starts from an explicit leaf ID
// instead of auto-detecting the latest leaf. Used for tree navigation.
func BuildContextFromLeaf(entries []SessionEntry, leafID string) []types.Message {
	if len(entries) == 0 || leafID == "" {
		return BuildContext(entries)
	}

	byID := make(map[string]*SessionEntry, len(entries))
	for i := range entries {
		byID[entries[i].ID] = &entries[i]
	}

	leafEntry, ok := byID[leafID]
	if !ok {
		return BuildContext(entries)
	}

	// Walk from explicit leaf to root
	chain := ForEachEntry(entries, leafEntry.ID)

	// Build messages from root to leaf, handling compaction
	var messages []types.Message
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

		switch entry.Type {
		case EntryTypeCompaction:
			sysContent, _ := json.Marshal([]types.TextContent{
				{Type: "text", Text: entry.Summary},
			})
			messages = append(messages, types.Message{
				Role:    "system",
				Content: sysContent,
			})

			var meta struct {
				FirstKeptEntryID string `json:"first_kept_entry_id"`
			}
			if err := json.Unmarshal(entry.Content, &meta); err == nil && meta.FirstKeptEntryID != "" {
				skipUntil = meta.FirstKeptEntryID
			}

		case EntryTypeModelChange, EntryTypeLabel,
			EntryTypeSessionInfo, EntryTypeSystem:
			// Skip metadata entries

		default:
			msg := entryToMessage(entry)
			messages = append(messages, msg)
		}
	}

	return messages
}

// entryToMessage converts a SessionEntry to a types.Message for LLM context.
func entryToMessage(entry SessionEntry) types.Message {
	msg := types.Message{
		Role:      entry.Role,
		Content:   entry.Content,
		ToolCalls: entry.ToolCalls,
	}

	// For tool messages, Content may wrap tool_call_id
	if entry.Role == "tool" {
		var wrapped struct {
			ToolCallID string          `json:"tool_call_id"`
			Result     json.RawMessage `json:"result"`
		}
		if json.Unmarshal(entry.Content, &wrapped) == nil && wrapped.ToolCallID != "" {
			msg.ToolCallID = wrapped.ToolCallID
			msg.Content = wrapped.Result
		}
	}

	// Infer role from type if not set
	if msg.Role == "" {
		switch entry.Type {
		case EntryTypeUser:
			msg.Role = "user"
		case EntryTypeAssistant:
			msg.Role = "assistant"
		case EntryTypeTool:
			msg.Role = "tool"
		default:
			msg.Role = entry.Type
		}
	}

	return msg
}

// GenerateBranchSummary generates a short summary of the session entries starting
// from fromEntryID, using the provided LLM summarizer function. The summary is
// stored as a branch_summary entry in the session.
//
// Parameters:
//   - s: the session to store the branch_summary entry in
//   - fromEntryID: the entry ID identifying the branch root
//   - parentID: the parent ID for the new branch_summary entry
//   - summarizer: function that takes messages and returns a short summary string
//   - fromHook: whether this summary was triggered by a hook
//
// Returns the generated summary string and any error.
func (m *Manager) GenerateBranchSummary(s *Session, fromEntryID, parentID string, summarizer func([]types.Message) (string, error), fromHook bool) (string, error) {
	// Walk the branch from fromEntryID to root to get the messages
	chain := ForEachEntry(s.Entries, fromEntryID)
	if len(chain) == 0 {
		return "", fmt.Errorf("no entries found for fromEntryID %q", fromEntryID)
	}

	// Convert chain to messages in chronological order (root to leaf)
	messages := BuildContextFromLeaf(s.Entries, fromEntryID)
	if len(messages) == 0 {
		return "", fmt.Errorf("no messages to summarize for fromEntryID %q", fromEntryID)
	}

	// Call the LLM summarizer
	summary, err := summarizer(messages)
	if err != nil {
		return "", fmt.Errorf("summarize branch: %w", err)
	}

	// Create and store the branch_summary entry
	entry := BranchSummaryEntry(summary, fromEntryID, parentID, fromHook)
	if err := m.AddEntry(s, entry); err != nil {
		return "", fmt.Errorf("store branch summary: %w", err)
	}

	return summary, nil
}
