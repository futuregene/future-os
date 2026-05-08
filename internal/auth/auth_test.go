package auth

import (
	"os"
	"path/filepath"
	"testing"
)

func TestLoadAuth_Empty(t *testing.T) {
	store, err := LoadAuth()
	if err != nil {
		t.Fatalf("LoadAuth error: %v", err)
	}
	if store == nil {
		t.Fatal("got nil store")
	}
}

func TestLoadAuth_FromFile(t *testing.T) {
	tmpDir := t.TempDir()
	authPath := filepath.Join(tmpDir, "auth.json")

	data := `{"deepseek": {"type": "api_key", "key": "sk-test123"}, "openai": {"type": "api_key", "key": "sk-openai456"}}`
	os.WriteFile(authPath, []byte(data), 0644)

	// Can't test LoadAuth directly since it uses hardcoded paths
	// Test Store.Get instead
	store := &Store{
		Entries: map[string]Entry{
			"deepseek": {Type: "api_key", Key: "sk-test123"},
			"openai":   {Type: "api_key", Key: "sk-openai456"},
		},
	}

	if key := store.Get("deepseek"); key != "sk-test123" {
		t.Errorf("Get(deepseek) = %s, want sk-test123", key)
	}
	if key := store.Get("openai"); key != "sk-openai456" {
		t.Errorf("Get(openai) = %s, want sk-openai456", key)
	}
	if key := store.Get("nonexistent"); key != "" {
		t.Errorf("Get(nonexistent) = %s, want empty", key)
	}
}

func TestStore_GetPrefixMatch(t *testing.T) {
	store := &Store{
		Entries: map[string]Entry{
			"dashscope-coding": {Type: "api_key", Key: "sk-dash123"},
		},
	}

	// Prefix match: "dashscope" should match "dashscope-coding"
	if key := store.Get("dashscope"); key != "sk-dash123" {
		t.Errorf("Get(dashscope) = %s, want sk-dash123", key)
	}
}

func TestStore_DefaultKey(t *testing.T) {
	// Nil store
	var nilStore *Store
	if key := nilStore.DefaultKey(); key != "" {
		t.Errorf("DefaultKey on nil = %s, want empty", key)
	}

	// Empty store
	emptyStore := &Store{Entries: map[string]Entry{}}
	if key := emptyStore.DefaultKey(); key != "" {
		t.Errorf("DefaultKey on empty = %s, want empty", key)
	}

	// Store with entries
	store := &Store{
		Entries: map[string]Entry{
			"first":  {Type: "api_key", Key: "key1"},
			"second": {Type: "api_key", Key: "key2"},
		},
	}
	if key := store.DefaultKey(); key != "key1" {
		t.Errorf("DefaultKey = %s, want key1", key)
	}
}

func TestStore_GetNil(t *testing.T) {
	var store *Store
	if key := store.Get("any"); key != "" {
		t.Errorf("Get on nil = %s, want empty", key)
	}
}
