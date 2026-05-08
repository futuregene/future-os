// Package auth reads and manages API authentication credentials.
// It mirrors pi's ~/.pi/agent/auth.json format.
package auth

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// Entry represents a single provider's API credentials.
type Entry struct {
	Type string `json:"type"` // "api_key", "oauth", etc.
	Key  string `json:"key"`
}

// Store holds provider-indexed API credentials.
type Store struct {
	Entries map[string]Entry `json:"-"`
	raw     map[string]json.RawMessage
}

// LoadAuth reads auth.json from ~/.xihu/auth.json.
// Falls back to ~/.pi/agent/auth.json for migration compatibility.
func LoadAuth() (*Store, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return nil, err
	}

	paths := []string{
		filepath.Join(home, ".xihu", "auth.json"),
		filepath.Join(home, ".pi", "agent", "auth.json"),
	}

	var data []byte
	var usedPath string
	for _, p := range paths {
		d, err := os.ReadFile(p)
		if err == nil {
			data = d
			usedPath = p
			break
		}
	}
	if data == nil {
		return &Store{Entries: map[string]Entry{}}, nil
	}

	var raw map[string]json.RawMessage
	if err := json.Unmarshal(data, &raw); err != nil {
		return nil, fmt.Errorf("parse %s: %w", usedPath, err)
	}

	store := &Store{
		Entries: make(map[string]Entry, len(raw)),
		raw:     raw,
	}

	for name, entryJSON := range raw {
		var e Entry
		if err := json.Unmarshal(entryJSON, &e); err != nil {
			// Silently skip unparseable entries
			continue
		}
		store.Entries[name] = e
	}

	return store, nil
}

// Get returns the API key for a given provider name.
// Provider matching is case-insensitive and supports partial matches
// (e.g., "deepseek" matches "deepseek", "dashscope" matches "dashscope-coding").
func (s *Store) Get(provider string) string {
	if s == nil {
		return ""
	}
	provider = strings.ToLower(provider)

	// Exact match first
	if e, ok := s.Entries[provider]; ok && e.Key != "" {
		return e.Key
	}

	// Prefix match (e.g., "dashscope" → "dashscope-coding")
	for name, e := range s.Entries {
		if strings.HasPrefix(strings.ToLower(name), provider) && e.Key != "" {
			return e.Key
		}
	}

	return ""
}

// DefaultKey returns the key for the default provider, or the first available key.
func (s *Store) DefaultKey() string {
	if s == nil || len(s.Entries) == 0 {
		return ""
	}
	// Return the first entry's key as default
	for _, e := range s.Entries {
		if e.Key != "" {
			return e.Key
		}
	}
	return ""
}
