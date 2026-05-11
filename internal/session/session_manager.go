package session

import (
	"bufio"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/huichen/xihu/pkg/types"
)

// SessionSummary is a lightweight summary of a session for cross-project lookups.
type SessionSummary struct {
	ID        string    `json:"id"`
	CWD       string    `json:"cwd"`
	UpdatedAt time.Time `json:"updated_at"`
	Model     string    `json:"model"`
	Name      string    `json:"name,omitempty"`
}

// List returns all saved sessions for a CWD, sorted by update time (newest first).
func (m *Manager) List(cwd string) ([]Session, error) {
	dir := m.sessionDir(cwd)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return nil, err
	}

	entries, err := os.ReadDir(dir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}

	var sessions []Session
	for _, entry := range entries {
		if entry.IsDir() {
			continue
		}
		if filepath.Ext(entry.Name()) != ".jsonl" {
			continue
		}
		s, err := m.loadFromPath(filepath.Join(dir, entry.Name()))
		if err != nil || s == nil {
			continue
		}
		sessions = append(sessions, *s)
	}

	// Sort by UpdatedAt descending
	sort.Slice(sessions, func(i, j int) bool {
		return sessions[j].UpdatedAt.Before(sessions[i].UpdatedAt)
	})

	return sessions, nil
}

// ListAll lists all sessions across all CWD directories (TS pi-mono: SessionManager.listAll).
// Returns lightweight SessionSummary structs instead of full sessions for efficient cross-project lookups.
func (m *Manager) ListAll() ([]SessionSummary, error) {
	if err := os.MkdirAll(m.Dir, 0755); err != nil {
		return nil, err
	}

	cwdDirs, err := os.ReadDir(m.Dir)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}

	var summaries []SessionSummary
	for _, cwdEntry := range cwdDirs {
		if !cwdEntry.IsDir() {
			continue
		}
		cwdPath := filepath.Join(m.Dir, cwdEntry.Name())
		entries, err := os.ReadDir(cwdPath)
		if err != nil {
			continue
		}
		for _, entry := range entries {
			if entry.IsDir() {
				continue
			}
			if filepath.Ext(entry.Name()) != ".jsonl" {
				continue
			}
			summary := loadSessionSummary(filepath.Join(cwdPath, entry.Name()))
			if summary != nil {
				summaries = append(summaries, *summary)
			}
		}
	}

	// Sort by UpdatedAt descending
	sort.Slice(summaries, func(i, j int) bool {
		return summaries[j].UpdatedAt.Before(summaries[i].UpdatedAt)
	})

	return summaries, nil
}

// loadSessionSummary parses only the first line (session_info entry) of a JSONL file
// to extract metadata without loading the full session.
func loadSessionSummary(path string) *SessionSummary {
	f, err := os.Open(path)
	if err != nil {
		return nil
	}
	defer f.Close()

	scanner := bufio.NewScanner(f)
	scanner.Buffer(make([]byte, 1024*1024), 10*1024*1024)
	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" {
			continue
		}
		var entry SessionEntry
		if err := json.Unmarshal([]byte(line), &entry); err != nil {
			return nil
		}
		if entry.Type != EntryTypeSessionInfo {
			return nil
		}
		var meta struct {
			CWD       string `json:"cwd"`
			UpdatedAt string `json:"updated_at"`
			CreatedAt string `json:"created_at"`
			Name      string `json:"name"`
		}
		if err := json.Unmarshal(entry.Content, &meta); err != nil {
			return nil
		}
		summary := &SessionSummary{
			ID:    entry.ID,
			CWD:   meta.CWD,
			Model: entry.Model,
			Name:  meta.Name,
		}
		if t, err := time.Parse(time.RFC3339, meta.UpdatedAt); err == nil {
			summary.UpdatedAt = t
		} else if t, err := time.Parse(time.RFC3339, meta.CreatedAt); err == nil {
			summary.UpdatedAt = t
		} else {
			summary.UpdatedAt = entry.Timestamp
		}
		return summary
	}
	return nil
}

// Load loads a session by ID and CWD.
func (m *Manager) Load(id, cwd string) (*Session, error) {
	path := m.sessionPath(cwd, id)
	return m.loadFromPath(path)
}

// loadFromPath reads a session from a JSONL file path.
func (m *Manager) loadFromPath(path string) (*Session, error) {
	f, err := os.Open(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, err
		}
		return nil, fmt.Errorf("open session file: %w", err)
	}
	defer f.Close()

	var s Session
	scanner := bufio.NewScanner(f)
	scanner.Buffer(make([]byte, 1024*1024), 10*1024*1024) // 10MB max line
	firstLine := true

	for scanner.Scan() {
		line := strings.TrimSpace(scanner.Text())
		if line == "" {
			continue
		}
		var entry SessionEntry
		if err := json.Unmarshal([]byte(line), &entry); err != nil {
			continue // skip malformed lines
		}
		if firstLine && entry.Type == EntryTypeSessionInfo {
			parseSessionInfo(entry, &s)
			firstLine = false
			continue
		}
		firstLine = false
		// Extract name from mid-session session_info entries
		if entry.Type == EntryTypeSessionInfo {
			var meta struct {
				Name string `json:"name"`
			}
			if json.Unmarshal(entry.Content, &meta) == nil && meta.Name != "" {
				s.Name = meta.Name
			}
		}
		s.Entries = append(s.Entries, entry)
	}

	if err := scanner.Err(); err != nil {
		return nil, fmt.Errorf("read session file: %w", err)
	}

	if s.ID == "" {
		return nil, fmt.Errorf("no session_info entry found")
	}

	return &s, nil
}

// Save persists a session to disk as a JSONL file.
func (m *Manager) Save(s *Session) error {
	dir := m.sessionDir(s.CWD)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}
	s.UpdatedAt = time.Now()
	if s.CreatedAt.IsZero() {
		s.CreatedAt = s.UpdatedAt
	}

	path := m.sessionPath(s.CWD, s.ID)
	f, err := os.Create(path)
	if err != nil {
		return fmt.Errorf("create session file: %w", err)
	}
	defer f.Close()

	enc := json.NewEncoder(f)
	enc.SetEscapeHTML(false)

	// Write session_info entry as first line
	if err := enc.Encode(sessionInfoEntry(s)); err != nil {
		return fmt.Errorf("write session info: %w", err)
	}

	// Write all entries
	for _, entry := range s.Entries {
		if err := enc.Encode(entry); err != nil {
			return fmt.Errorf("write entry: %w", err)
		}
	}

	return nil
}

// Delete removes a session file.
func (m *Manager) Delete(id, cwd string) error {
	path := m.sessionPath(cwd, id)
	return os.Remove(path)
}

// AddEntry appends a single entry to the session's JSONL file and
// adds it to the in-memory Entries slice. Creates the file if needed.
func (m *Manager) AddEntry(s *Session, entry SessionEntry) error {
	dir := m.sessionDir(s.CWD)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return err
	}
	path := m.sessionPath(s.CWD, s.ID)

	// If file doesn't exist, create it with session_info header
	_, statErr := os.Stat(path)
	if os.IsNotExist(statErr) {
		f, err := os.Create(path)
		if err != nil {
			return fmt.Errorf("create session file: %w", err)
		}
		enc := json.NewEncoder(f)
		enc.SetEscapeHTML(false)
		if err := enc.Encode(sessionInfoEntry(s)); err != nil {
			f.Close()
			return fmt.Errorf("write session info: %w", err)
		}
		f.Close()
	}

	// Append the entry
	f, err := os.OpenFile(path, os.O_APPEND|os.O_WRONLY, 0644)
	if err != nil {
		return fmt.Errorf("open session file for append: %w", err)
	}
	defer f.Close()

	enc := json.NewEncoder(f)
	enc.SetEscapeHTML(false)
	if err := enc.Encode(entry); err != nil {
		return fmt.Errorf("append entry: %w", err)
	}

	// Update in-memory state
	s.Entries = append(s.Entries, entry)
	s.LeafID = entry.ID // TS pi-mono: leafId tracks last appended entry
	s.UpdatedAt = time.Now()

	return nil
}

// MessageToEntry converts a types.Message to a SessionEntry with a generated ID.
func MessageToEntry(msg types.Message, parentID string) SessionEntry {
	entry := SessionEntry{
		ID:        GenerateEntryID(),
		ParentID:  parentID,
		Role:      msg.Role,
		ToolCalls: msg.ToolCalls,
		Timestamp: time.Now(),
	}

	// Map role to entry type
	switch msg.Role {
	case "user":
		entry.Type = EntryTypeUser
	case "assistant":
		entry.Type = EntryTypeAssistant
	case "tool":
		entry.Type = EntryTypeTool
		// Wrap tool_call_id into content
		if msg.ToolCallID != "" {
			wrapped, _ := json.Marshal(map[string]interface{}{
				"tool_call_id": msg.ToolCallID,
				"result":       msg.Content,
			})
			entry.Content = wrapped
		} else {
			entry.Content = msg.Content
		}
	default:
		entry.Type = EntryTypeSystem
		entry.Content = msg.Content
	}

	if entry.Content == nil && msg.Role != "tool" {
		entry.Content = msg.Content
	}

	return entry
}

// MessagesToEntries converts a slice of types.Message to SessionEntries.
// Each entry's ParentID chains from the previous entry (first uses rootParentID).
func MessagesToEntries(messages []types.Message, rootParentID string) []SessionEntry {
	entries := make([]SessionEntry, 0, len(messages))
	parentID := rootParentID
	for _, msg := range messages {
		entry := MessageToEntry(msg, parentID)
		entries = append(entries, entry)
		parentID = entry.ID
	}
	return entries
}
