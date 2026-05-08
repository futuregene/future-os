package session

import (
	"encoding/json"
	"fmt"
	"os"
	"time"
)

// --- Label & Session Info ---

// AppendLabelChange appends a label change entry to the session.
func (m *Manager) AppendLabelChange(s *Session, targetID, label string) error {
	entry := SessionEntry{
		ID:        GenerateEntryID(),
		ParentID:  targetID,
		Type:      EntryTypeLabel,
		Label:     label,
		Timestamp: time.Now(),
	}
	return m.AddEntry(s, entry)
}

// AppendSessionInfo appends a user-editable session info entry (e.g., name).
func (m *Manager) AppendSessionInfo(s *Session, name string) error {
	meta, _ := json.Marshal(map[string]string{"name": name})
	entry := SessionEntry{
		ID:        GenerateEntryID(),
		ParentID:  s.Entries[0].ID,
		Type:      EntryTypeSessionInfo,
		Timestamp: time.Now(),
		Content:   json.RawMessage(meta),
	}
	return m.AddEntry(s, entry)
}

// GetLabel returns the current label for a target entry, or empty string.
func GetLabel(entries []SessionEntry, targetID string) string {
	for i := len(entries) - 1; i >= 0; i-- {
		if entries[i].Type == EntryTypeLabel && entries[i].ParentID == targetID {
			return entries[i].Label
		}
	}
	return ""
}

// GetSessionName returns the display name for a session from session_info entries.
func GetSessionName(entries []SessionEntry) string {
	for i := len(entries) - 1; i >= 0; i-- {
		if entries[i].Type == EntryTypeSessionInfo && entries[i].Content != nil {
			var meta map[string]string
			if json.Unmarshal(entries[i].Content, &meta) == nil {
				if name, ok := meta["name"]; ok && name != "" {
					return name
				}
			}
		}
	}
	return ""
}

// --- Branch Management ---

// BranchWithSummary appends a branch_summary entry and returns the new leaf ID.
func (m *Manager) BranchWithSummary(s *Session, summary, fromID, parentID string, fromHook bool) (string, error) {
	entry := BranchSummaryEntry(summary, fromID, parentID, fromHook)
	if err := m.AddEntry(s, entry); err != nil {
		return "", err
	}
	return entry.ID, nil
}

// GetLeafID returns the ID of the most recent leaf entry (not a parent to any other).
func GetLeafID(entries []SessionEntry) string {
	parents := make(map[string]bool)
	for _, e := range entries {
		if e.ParentID != "" {
			parents[e.ParentID] = true
		}
	}
	var latest *SessionEntry
	for i := range entries {
		if parents[entries[i].ID] {
			continue
		}
		if latest == nil || entries[i].Timestamp.After(latest.Timestamp) {
			latest = &entries[i]
		}
	}
	if latest != nil {
		return latest.ID
	}
	return ""
}

// --- CWD Validation ---

// ValidateCWD checks that the session's CWD exists and is a directory.
func ValidateCWD(cwd string) error {
	info, err := os.Stat(cwd)
	if err != nil {
		if os.IsNotExist(err) {
			return fmt.Errorf("working directory does not exist: %s", cwd)
		}
		return fmt.Errorf("access working directory %s: %w", cwd, err)
	}
	if !info.IsDir() {
		return fmt.Errorf("not a directory: %s", cwd)
	}
	return nil
}
