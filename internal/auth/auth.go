// Package auth provides credential management for LLM providers.
// API keys, base URLs, and optional scopes are stored in
// ~/.pi/agent/auth.json with JSON encoding. Entries may carry an
// expiry time; keys past expiry are treated as invalid.
package auth

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"time"
)

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

// AuthEntry holds credentials and metadata for a single provider.
type AuthEntry struct {
	Provider  string    `json:"provider"`
	APIKey    string    `json:"api_key"`
	BaseURL   string    `json:"base_url,omitempty"`
	Scopes    []string  `json:"scopes,omitempty"`
	ExpiresAt time.Time `json:"expires_at"`
}

// IsExpired returns true if the entry has an expiry set and that time has passed.
func (e *AuthEntry) IsExpired() bool {
	if e.ExpiresAt.IsZero() {
		return false // no expiry set
	}
	return time.Now().After(e.ExpiresAt)
}

// Storage manages a collection of AuthEntry records persisted to a JSON file.
type Storage struct {
	entries map[string]AuthEntry // provider → entry
	path    string               // file path
}

// ---------------------------------------------------------------------------
// Constructors / I/O
// ---------------------------------------------------------------------------

// LoadAuthStorage reads auth credentials from a JSON file. If the file does
// not exist, an empty Storage is returned (no error). The file is created on
// the first call to SetKey.
func LoadAuthStorage(path string) (*Storage, error) {
	s := &Storage{
		entries: make(map[string]AuthEntry),
		path:    path,
	}

	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return s, nil
		}
		return nil, fmt.Errorf("read auth file %s: %w", path, err)
	}

	var raw struct {
		Entries map[string]AuthEntry `json:"entries"`
	}
	if err := json.Unmarshal(data, &raw); err != nil {
		return nil, fmt.Errorf("parse auth file %s: %w", path, err)
	}

	if raw.Entries != nil {
		s.entries = raw.Entries
	}
	return s, nil
}

// SaveAuthStorage persists all credentials to the storage file.
func (s *Storage) SaveAuthStorage() error {
	if s == nil {
		return fmt.Errorf("auth storage is nil")
	}
	return s.save()
}

// save writes entries to the JSON file, creating parent directories if needed.
func (s *Storage) save() error {
	dir := filepath.Dir(s.path)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return fmt.Errorf("create auth directory %s: %w", dir, err)
	}

	raw := struct {
		Entries map[string]AuthEntry `json:"entries"`
	}{Entries: s.entries}

	data, err := json.MarshalIndent(raw, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal auth: %w", err)
	}
	data = append(data, '\n')

	if err := os.WriteFile(s.path, data, 0600); err != nil {
		return fmt.Errorf("write auth file %s: %w", s.path, err)
	}
	return nil
}

// DefaultAuthPath returns ~/.pi/agent/auth.json.
func DefaultAuthPath() string {
	home, err := os.UserHomeDir()
	if err != nil {
		home = os.TempDir()
	}
	return filepath.Join(home, ".pi", "agent", "auth.json")
}

// ---------------------------------------------------------------------------
// Credential operations
// ---------------------------------------------------------------------------

// GetKey returns the API key for the given provider. It returns an error if
// the provider is not found or if the stored entry has expired.
func (s *Storage) GetKey(provider string) (string, error) {
	if s == nil || s.entries == nil {
		return "", fmt.Errorf("provider %q not found: auth storage is empty", provider)
	}

	entry, ok := s.entries[provider]
	if !ok {
		return "", fmt.Errorf("provider %q not found in auth storage", provider)
	}

	if entry.IsExpired() {
		return "", fmt.Errorf("credentials for provider %q expired at %s",
			provider, entry.ExpiresAt.Format(time.RFC3339))
	}

	return entry.APIKey, nil
}

// SetKey stores (or overwrites) credentials for a provider.
// If ttl > 0, ExpiresAt is set to now + ttl; otherwise no expiry is set.
// The storage is automatically saved to disk after modification.
func (s *Storage) SetKey(provider, key, baseURL string, scopes []string, ttl time.Duration) error {
	if provider == "" {
		return fmt.Errorf("provider name is required")
	}

	entry := AuthEntry{
		Provider: provider,
		APIKey:   key,
		BaseURL:  baseURL,
		Scopes:   scopes,
	}
	if ttl > 0 {
		entry.ExpiresAt = time.Now().Add(ttl)
	}

	s.entries[provider] = entry
	return s.save()
}

// ListProviders returns all provider names currently stored, sorted alphabetically.
func (s *Storage) ListProviders() []string {
	if s == nil || s.entries == nil {
		return nil
	}
	names := make([]string, 0, len(s.entries))
	for name := range s.entries {
		names = append(names, name)
	}
	sort.Strings(names)
	return names
}
