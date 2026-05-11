package session

import (
	"crypto/rand"
	"encoding/base32"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
	"time"

	"github.com/huichen/xihu/pkg/types"
)

// CURRENT_SESSION_VERSION is the current session format version.
// When loading older sessions, migration functions are applied.
const CURRENT_SESSION_VERSION = 3

// Entry type constants for SessionEntry.Type
const (
	EntryTypeUser               = "user"
	EntryTypeAssistant          = "assistant"
	EntryTypeTool               = "tool"
	EntryTypeSystem             = "system"
	EntryTypeCompaction         = "compaction"
	EntryTypeModelChange        = "model_change"
	EntryTypeLabel              = "label"
	EntryTypeSessionInfo        = "session_info"
	EntryTypeThinkingLevelChange = "thinking_level_change"
	EntryTypeBranchSummary       = "branch_summary"
	EntryTypeCustom              = "custom"
	EntryTypeCustomMessage       = "custom_message"
)

// BranchSummaryMeta holds metadata for branch_summary entries.
type BranchSummaryMeta struct {
	FromID   string `json:"from_id,omitempty"`
	FromHook *bool  `json:"from_hook,omitempty"`
}

// SessionEntry is a single node in the conversation tree.
// Entries form a tree via ParentID, enabling forks, branches, and compaction.
type SessionEntry struct {
	ID             string            `json:"id"`
	ParentID       string            `json:"parent_id,omitempty"`
	Type           string            `json:"type"` // "user"|"assistant"|"tool"|"system"|"compaction"|"model_change"|"label"|"session_info"
	Role           string            `json:"role,omitempty"`
	Content        json.RawMessage   `json:"content,omitempty"`
	ToolCalls      []types.ToolCall  `json:"tool_calls,omitempty"`
	Timestamp      time.Time         `json:"timestamp"`
	Summary        string            `json:"summary,omitempty"`          // for compaction entries
	Model          string            `json:"model,omitempty"`            // for model_change entries
	Label          string            `json:"label,omitempty"`            // for label entries
	ThinkingLevel  string            `json:"thinking_level,omitempty"`   // for thinking_level_change
	BranchSummary  *BranchSummaryMeta `json:"branch_summary,omitempty"`  // for branch_summary
	CustomType     string            `json:"custom_type,omitempty"`      // for custom/custom_message
	CustomData     json.RawMessage   `json:"custom_data,omitempty"`      // for custom entries
	Display        string            `json:"display,omitempty"`          // for custom_message display
	Provider       string            `json:"provider,omitempty"`         // for model_change
}

// Session stores a conversation session with tree-structured entries.
type Session struct {
	ID              string         `json:"id"`
	Version         int            `json:"version"`
	CWD             string         `json:"cwd"`
	Model           string         `json:"model"`
	BaseURL         string         `json:"base_url"`
	Name            string         `json:"name,omitempty"`
	ParentSessionID string         `json:"parent_session_id,omitempty"` // session ID this was forked from
	LeafID          string         `json:"leaf_id,omitempty"`           // explicit leaf pointer (tree navigation)
	Entries         []SessionEntry `json:"entries"`
	CreatedAt       time.Time      `json:"created_at"`
	UpdatedAt       time.Time      `json:"updated_at"`
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
