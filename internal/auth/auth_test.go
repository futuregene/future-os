package auth

import (
	"os"
	"path/filepath"
	"testing"
	"time"
)

func TestAuthEntryIsExpired(t *testing.T) {
	tests := []struct {
		name     string
		expireAt time.Time
		want     bool
	}{
		{"zero time (never expires)", time.Time{}, false},
		{"future time", time.Now().Add(time.Hour), false},
		{"past time", time.Now().Add(-time.Hour), true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			e := AuthEntry{ExpiresAt: tt.expireAt}
			got := e.IsExpired()
			if got != tt.want {
				t.Errorf("IsExpired() = %v, want %v", got, tt.want)
			}
		})
	}
}

func TestLoadAuthStorageEmpty(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "nonexistent.json")

	s, err := LoadAuthStorage(path)
	if err != nil {
		t.Fatalf("LoadAuthStorage error: %v", err)
	}
	if s == nil {
		t.Fatal("got nil storage")
	}
	if len(s.entries) != 0 {
		t.Errorf("entries = %d, want 0", len(s.entries))
	}
}

func TestLoadAuthStorageInvalidJSON(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "invalid.json")
	os.WriteFile(path, []byte("not json"), 0644)

	_, err := LoadAuthStorage(path)
	if err == nil {
		t.Fatal("expected error for invalid JSON")
	}
}

func TestLoadAuthStorageValid(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "auth.json")
	os.WriteFile(path, []byte(`{"entries":{"openai":{"provider":"openai","api_key":"sk-123"}}}`), 0644)

	s, err := LoadAuthStorage(path)
	if err != nil {
		t.Fatalf("LoadAuthStorage error: %v", err)
	}
	if len(s.entries) != 1 {
		t.Fatalf("entries = %d, want 1", len(s.entries))
	}
	key, err := s.GetKey("openai")
	if err != nil {
		t.Fatalf("GetKey: %v", err)
	}
	if key != "sk-123" {
		t.Errorf("key = %s, want sk-123", key)
	}
}

func TestLoadAuthStorageWithNullEntries(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "auth.json")
	os.WriteFile(path, []byte(`{}`), 0644)

	s, err := LoadAuthStorage(path)
	if err != nil {
		t.Fatalf("LoadAuthStorage error: %v", err)
	}
	if s == nil {
		t.Fatal("got nil storage")
	}
}

func TestSaveAuthStorage(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "auth.json")

	s := &Storage{
		entries: map[string]AuthEntry{
			"openai": {Provider: "openai", APIKey: "sk-456", BaseURL: "https://api.openai.com/v1"},
		},
		path: path,
	}

	err := s.SaveAuthStorage()
	if err != nil {
		t.Fatalf("SaveAuthStorage: %v", err)
	}

	// Verify file was written
	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read saved file: %v", err)
	}
	if len(data) == 0 {
		t.Fatal("saved file is empty")
	}
}

func TestSaveAuthStorageNil(t *testing.T) {
	var s *Storage
	err := s.SaveAuthStorage()
	if err == nil {
		t.Fatal("expected error for nil storage")
	}
}

func TestSaveAuthStorageSubdir(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "sub", "dir", "auth.json")

	s := &Storage{
		entries: map[string]AuthEntry{},
		path:    path,
	}

	err := s.SaveAuthStorage()
	if err != nil {
		t.Fatalf("SaveAuthStorage: %v", err)
	}
	if _, err := os.Stat(path); err != nil {
		t.Errorf("file not created: %v", err)
	}
}

func TestDefaultAuthPath(t *testing.T) {
	path := DefaultAuthPath()
	if path == "" {
		t.Fatal("DefaultAuthPath returned empty")
	}
	if !filepath.IsAbs(path) {
		// might be relative if UserHomeDir fails
		t.Logf("default auth path: %s", path)
	}
}

func TestGetKey(t *testing.T) {
	s := &Storage{
		entries: map[string]AuthEntry{
			"openai": {Provider: "openai", APIKey: "sk-789"},
		},
	}

	tests := []struct {
		name     string
		provider string
		wantKey  string
		wantErr  bool
	}{
		{"found", "openai", "sk-789", false},
		{"not found", "anthropic", "", true},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			key, err := s.GetKey(tt.provider)
			if tt.wantErr {
				if err == nil {
					t.Error("expected error")
				}
				return
			}
			if err != nil {
				t.Fatalf("unexpected error: %v", err)
			}
			if key != tt.wantKey {
				t.Errorf("key = %s, want %s", key, tt.wantKey)
			}
		})
	}
}

func TestGetKeyNilStorage(t *testing.T) {
	var s *Storage
	_, err := s.GetKey("openai")
	if err == nil {
		t.Fatal("expected error for nil storage")
	}
}

func TestGetKeyEmptyEntries(t *testing.T) {
	s := &Storage{entries: nil}
	_, err := s.GetKey("openai")
	if err == nil {
		t.Fatal("expected error for nil entries")
	}
}

func TestGetKeyExpired(t *testing.T) {
	s := &Storage{
		entries: map[string]AuthEntry{
			"openai": {Provider: "openai", APIKey: "sk-000", ExpiresAt: time.Now().Add(-time.Hour)},
		},
	}
	_, err := s.GetKey("openai")
	if err == nil {
		t.Fatal("expected error for expired key")
	}
}

func TestSetKey(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "auth.json")
	s := &Storage{entries: make(map[string]AuthEntry), path: path}

	err := s.SetKey("openai", "sk-new", "https://api.openai.com/v1", []string{"read"}, 0)
	if err != nil {
		t.Fatalf("SetKey: %v", err)
	}

	key, err := s.GetKey("openai")
	if err != nil {
		t.Fatalf("GetKey: %v", err)
	}
	if key != "sk-new" {
		t.Errorf("key = %s, want sk-new", key)
	}
}

func TestSetKeyWithTTL(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "auth.json")
	s := &Storage{entries: make(map[string]AuthEntry), path: path}

	err := s.SetKey("openai", "sk-ttl", "", nil, time.Hour)
	if err != nil {
		t.Fatalf("SetKey: %v", err)
	}

	entry := s.entries["openai"]
	if entry.ExpiresAt.IsZero() {
		t.Error("expiry should be set")
	}
	if entry.IsExpired() {
		t.Error("should not be expired yet")
	}
}

func TestSetKeyEmptyProvider(t *testing.T) {
	s := &Storage{entries: make(map[string]AuthEntry)}
	err := s.SetKey("", "sk", "", nil, 0)
	if err == nil {
		t.Fatal("expected error for empty provider")
	}
}

func TestSetKeyOverwrite(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "auth.json")
	s := &Storage{entries: make(map[string]AuthEntry), path: path}

	s.SetKey("openai", "first", "", nil, 0)
	s.SetKey("openai", "second", "", nil, 0)

	key, _ := s.GetKey("openai")
	if key != "second" {
		t.Errorf("key = %s, want second", key)
	}
}

func TestListProviders(t *testing.T) {
	tests := []struct {
		name    string
		storage *Storage
		want    []string
	}{
		{
			name:    "nil storage",
			storage: nil,
			want:    nil,
		},
		{
			name:    "nil entries",
			storage: &Storage{entries: nil},
			want:    nil,
		},
		{
			name: "empty entries",
			storage: &Storage{entries: map[string]AuthEntry{}},
			want:    []string{},
		},
		{
			name: "multiple providers",
			storage: &Storage{entries: map[string]AuthEntry{
				"openai":    {},
				"anthropic": {},
				"deepseek":  {},
			}},
			want: []string{"anthropic", "deepseek", "openai"},
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			got := tt.storage.ListProviders()
			if len(got) != len(tt.want) {
				t.Errorf("len = %d, want %d", len(got), len(tt.want))
				return
			}
			for i := range got {
				if got[i] != tt.want[i] {
					t.Errorf("[%d] = %s, want %s", i, got[i], tt.want[i])
				}
			}
		})
	}
}

func TestRoundTrip(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "auth.json")

	// Create and save
	s1 := &Storage{entries: make(map[string]AuthEntry), path: path}
	s1.SetKey("openai", "sk-abc", "https://api.openai.com/v1", nil, 0)
	s1.SetKey("anthropic", "sk-xyz", "", nil, time.Hour)

	// Load back
	s2, err := LoadAuthStorage(path)
	if err != nil {
		t.Fatalf("LoadAuthStorage: %v", err)
	}

	openaiKey, _ := s2.GetKey("openai")
	if openaiKey != "sk-abc" {
		t.Errorf("openai key = %s", openaiKey)
	}

	anthropicKey, _ := s2.GetKey("anthropic")
	if anthropicKey != "sk-xyz" {
		t.Errorf("anthropic key = %s", anthropicKey)
	}

	providers := s2.ListProviders()
	if len(providers) != 2 {
		t.Errorf("providers = %v", providers)
	}
}
