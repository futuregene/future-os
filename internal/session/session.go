package session

import (
	"bufio"
	"crypto/rand"
	"encoding/base32"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"time"

	"github.com/huichen/xihu/pkg/types"
)

// CURRENT_SESSION_VERSION is the current session format version.
// When loading older sessions, migration functions are applied.
const CURRENT_SESSION_VERSION = 3

// Entry type constants for SessionEntry.Type
const (
	EntryTypeUser        = "user"
	EntryTypeAssistant   = "assistant"
	EntryTypeTool        = "tool"
	EntryTypeSystem      = "system"
	EntryTypeCompaction  = "compaction"
	EntryTypeModelChange = "model_change"
	EntryTypeLabel               = "label"
	EntryTypeSessionInfo         = "session_info"
	EntryTypeThinkingLevelChange  = "thinking_level_change"
	EntryTypeBranchSummary        = "branch_summary"
	EntryTypeCustom               = "custom"
	EntryTypeCustomMessage        = "custom_message"
)

// BranchSummaryMeta holds metadata for branch_summary entries.
type BranchSummaryMeta struct {
	FromID   string `json:"from_id,omitempty"`
	FromHook *bool  `json:"from_hook,omitempty"`
}

// SessionEntry is a single node in the conversation tree.
// Entries form a tree via ParentID, enabling forks, branches, and compaction.
type SessionEntry struct {
	ID        string           `json:"id"`
	ParentID  string           `json:"parent_id,omitempty"`
	Type      string           `json:"type"` // "user"|"assistant"|"tool"|"system"|"compaction"|"model_change"|"label"|"session_info"
	Role      string           `json:"role,omitempty"`
	Content   json.RawMessage  `json:"content,omitempty"`
	ToolCalls []types.ToolCall `json:"tool_calls,omitempty"`
	Timestamp time.Time        `json:"timestamp"`
	Summary   string           `json:"summary,omitempty"` // for compaction entries
	Model     string           `json:"model,omitempty"`   // for model_change entries
	Label           string           `json:"label,omitempty"`            // for label entries
	ThinkingLevel   string           `json:"thinking_level,omitempty"`    // for thinking_level_change
	BranchSummary   *BranchSummaryMeta `json:"branch_summary,omitempty"`   // for branch_summary
	CustomType      string            `json:"custom_type,omitempty"`       // for custom/custom_message
	CustomData      json.RawMessage   `json:"custom_data,omitempty"`       // for custom entries
	Display         string            `json:"display,omitempty"`           // for custom_message display
	Provider        string            `json:"provider,omitempty"`          // for model_change
}

// Session stores a conversation session with tree-structured entries.
type Session struct {
	ID                string         `json:"id"`
	Version           int            `json:"version"`
	CWD               string         `json:"cwd"`
	Model             string         `json:"model"`
	BaseURL           string         `json:"base_url"`
	Name              string         `json:"name,omitempty"`
	ParentSessionID string `json:"parent_session_id,omitempty"` // session ID this was forked from
	LeafID            string         `json:"leaf_id,omitempty"` // explicit leaf pointer (tree navigation)
	Entries           []SessionEntry `json:"entries"`
	CreatedAt         time.Time      `json:"created_at"`
	UpdatedAt         time.Time      `json:"updated_at"`
}

// GetSessionName returns the session name, or empty string if not set.
func (s *Session) GetSessionName() string { return s.Name }

// SetSessionName sets the session name.
func (s *Session) SetSessionName(name string) { s.Name = name }

// BaseURL returns the session's base URL.
func (s *Session) GetBaseURL() string { return s.BaseURL }

// SetBaseURL sets the session's base URL.
func (s *Session) SetBaseURL(u string) { s.BaseURL = u }

// Manager handles session persistence in CWD-based directories.
type Manager struct {
	Dir string // root sessions dir, e.g. ~/.xihu/sessions
}

// NewManager creates a session manager.
func NewManager(dir string) *Manager {
	return &Manager{Dir: dir}
}

// DefaultDir returns the default session directory for a given CWD.
func DefaultDir(cwd string) string {
	home, err := os.UserHomeDir()
	if err != nil {
		return filepath.Join(os.TempDir(), ".xihu", "sessions", encodeCWD(cwd))
	}
	return filepath.Join(home, ".xihu", "sessions", encodeCWD(cwd))
}

// GenerateID creates a session ID from the current timestamp.
func GenerateID() string {
	return time.Now().Format("20060102-150405")
}

// GenerateEntryID creates a time-sortable UUID v7 style entry ID.
// Format: "20260508-090513-a1b2c3" (date-time-6randomhex)
func GenerateEntryID() string {
	now := time.Now()
	ts := now.Format("20060102-150405")
	random := make([]byte, 3) // 3 bytes = 6 hex chars
	if _, err := rand.Read(random); err != nil {
		// fallback: use nanoseconds as pseudo-random
		ns := uint32(now.UnixNano())
		random = []byte{byte(ns >> 16), byte(ns >> 8), byte(ns)}
	}
	return fmt.Sprintf("%s-%s", ts, hex.EncodeToString(random))
}

// encodeCWD converts a filesystem path into a safe directory name using base32.
func encodeCWD(cwd string) string {
	s := filepath.ToSlash(cwd)
	s = strings.TrimPrefix(s, "/")
	if s == "" || s == "." {
		s = "root"
	}
	return base32.StdEncoding.WithPadding(base32.NoPadding).EncodeToString([]byte(s))
}

// sessionDir returns the CWD-specific subdirectory.
func (m *Manager) sessionDir(cwd string) string {
	return filepath.Join(m.Dir, encodeCWD(cwd))
}

// sessionPath returns the full path for a session's JSONL file.
func (m *Manager) sessionPath(cwd, id string) string {
	return filepath.Join(m.sessionDir(cwd), id+".jsonl")
}

// SessionFilePath returns the full path for a session's JSONL file (exported).
func (m *Manager) SessionFilePath(cwd, id string) string {
	return m.sessionPath(cwd, id)
}

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

// SessionSummary is a lightweight summary of a session for cross-project lookups.
type SessionSummary struct {
	ID        string    `json:"id"`
	CWD       string    `json:"cwd"`
	UpdatedAt time.Time `json:"updated_at"`
	Model     string    `json:"model"`
	Name      string    `json:"name,omitempty"`
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

// ForkSession creates a new session from a specific entry in the parent session.
// The new session contains all ancestor entries of fromEntryID (including it).
func ForkSession(parent *Session, fromEntryID string) *Session {
	chain := ForEachEntry(parent.Entries, fromEntryID)

	// Reverse to get root-to-leaf order
	entries := make([]SessionEntry, 0, len(chain))
	for i := len(chain) - 1; i >= 0; i-- {
		entries = append(entries, chain[i])
	}

	now := time.Now()
	return &Session{
		ID:              GenerateID(),
		CWD:             parent.CWD,
		Model:           parent.Model,
		BaseURL:         parent.BaseURL,
		ParentSessionID: parent.ID,
		Entries:         entries,
		CreatedAt:       now,
		UpdatedAt:       now,
		Version:         CURRENT_SESSION_VERSION,
	}
}

// ReadonlySessionManager is an interface for read-only session access.
// Used when the session should not be modified (e.g., imported/archived sessions).
type ReadonlySessionManager interface {
	// List returns all saved sessions for a CWD.
	List(cwd string) ([]Session, error)

	// Load loads a session by ID and CWD.
	Load(id, cwd string) (*Session, error)
}

// MigrateSessionV1ToV2 migrates a v1 session to v2 format.
// Currently a no-op that bumps version to 2.
func MigrateSessionV1ToV2(s *Session) {
	if s.Version < 2 {
		s.Version = 2
	}
}

// MigrateSessionV2ToV3 migrates a v2 session to v3 format.
// Currently a no-op that bumps version to 3.
func MigrateSessionV2ToV3(s *Session) {
	if s.Version < 3 {
		s.Version = 3
	}
}

// MigrateSession applies all necessary migrations to bring a session to the current version.
func MigrateSession(s *Session) {
	if s.Version < 2 {
		MigrateSessionV1ToV2(s)
	}
	if s.Version < 3 {
		MigrateSessionV2ToV3(s)
	}
	s.Version = CURRENT_SESSION_VERSION
}

// SetVersion sets the session version to the current version.
func (s *Session) SetVersion() {
	s.Version = CURRENT_SESSION_VERSION
}
