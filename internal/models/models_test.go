package models

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestLoadRegistryFileNotExist(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "nonexistent.json")

	r, err := LoadRegistry(path)
	if err != nil {
		t.Fatalf("LoadRegistry: %v", err)
	}
	if r == nil {
		t.Fatal("registry is nil")
	}
	if r.Models == nil {
		t.Fatal("models is nil")
	}
	if len(r.Models) == 0 {
		t.Fatal("models is empty — should have builtins")
	}

	// File should have been created
	if _, err := os.Stat(path); err != nil {
		t.Errorf("bootstrap file not created: %v", err)
	}
}

func TestLoadRegistryInvalidJSON(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "invalid.json")
	os.WriteFile(path, []byte("not json"), 0644)

	_, err := LoadRegistry(path)
	if err == nil {
		t.Fatal("expected error for invalid JSON")
	}
}

func TestLoadRegistryValid(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "models.json")
	os.WriteFile(path, []byte(`{"models":{"gpt-4o":{"id":"gpt-4o","provider":"openai","api":"openai-completions","base_url":"https://api.openai.com/v1","max_tokens":128000,"supports_tools":true}}}`), 0644)

	r, err := LoadRegistry(path)
	if err != nil {
		t.Fatalf("LoadRegistry: %v", err)
	}
	if _, ok := r.Models["gpt-4o"]; !ok {
		t.Fatal("gpt-4o not found")
	}
}

func TestLoadRegistryNullModels(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "models.json")
	os.WriteFile(path, []byte(`{}`), 0644)

	r, err := LoadRegistry(path)
	if err != nil {
		t.Fatalf("LoadRegistry: %v", err)
	}
	if len(r.Models) == 0 {
		t.Fatal("should fall back to builtins")
	}
}

func TestSaveRegistry(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "models.json")

	r := &Registry{
		Models: map[string]ModelInfo{
			"gpt-4o": {ID: "gpt-4o", Provider: "openai", MaxTokens: 128000},
		},
		path: path,
	}

	err := r.SaveRegistry()
	if err != nil {
		t.Fatalf("SaveRegistry: %v", err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read saved file: %v", err)
	}
	if len(data) == 0 {
		t.Fatal("saved file is empty")
	}
}

func TestSaveRegistryNil(t *testing.T) {
	var r *Registry
	err := r.SaveRegistry()
	if err == nil {
		t.Fatal("expected error for nil registry")
	}
}

func TestSaveRegistrySubdir(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "sub", "models.json")

	r := &Registry{
		Models: map[string]ModelInfo{},
		path:   path,
	}
	err := r.SaveRegistry()
	if err != nil {
		t.Fatalf("SaveRegistry: %v", err)
	}
	if _, err := os.Stat(path); err != nil {
		t.Errorf("file not created: %v", err)
	}
}

func TestDefaultRegistryPath(t *testing.T) {
	path := DefaultRegistryPath()
	if path == "" {
		t.Fatal("path is empty")
	}
	if !strings.Contains(path, "models.json") {
		t.Errorf("path = %s", path)
	}
}

func TestBuiltinModels(t *testing.T) {
	models := BuiltinModels()
	if len(models) < 20 {
		t.Errorf("builtins = %d, want >= 20", len(models))
	}

	// Check known models
	ids := make(map[string]bool)
	for _, m := range models {
		ids[m.ID] = true
	}

	required := []string{"gpt-4o", "claude-sonnet-4", "deepseek-chat"}
	for _, id := range required {
		if !ids[id] {
			t.Errorf("missing builtin: %s", id)
		}
	}
}

func TestResolveModelExactMatch(t *testing.T) {
	r := &Registry{Models: builtinMap()}

	mi, err := r.ResolveModel("gpt-4o")
	if err != nil {
		t.Fatalf("ResolveModel: %v", err)
	}
	if mi.ID != "gpt-4o" {
		t.Errorf("id = %s", mi.ID)
	}
}

func TestResolveModelCaseInsensitive(t *testing.T) {
	r := &Registry{Models: builtinMap()}

	mi, err := r.ResolveModel("GPT-4O")
	if err != nil {
		t.Fatalf("ResolveModel: %v", err)
	}
	if !strings.EqualFold(mi.ID, "gpt-4o") {
		t.Errorf("id = %s", mi.ID)
	}
}

func TestResolveModelPrefix(t *testing.T) {
	r := &Registry{Models: builtinMap()}

	t.Run("gpt-4 prefix", func(t *testing.T) {
		mi, err := r.ResolveModel("gpt-4")
		if err != nil {
			t.Fatalf("ResolveModel: %v", err)
		}
		if !strings.HasPrefix(mi.ID, "gpt-4") {
			t.Errorf("id = %s, expected gpt-4 prefix", mi.ID)
		}
	})

	t.Run("claude prefix", func(t *testing.T) {
		mi, err := r.ResolveModel("claude")
		if err != nil {
			t.Fatalf("ResolveModel: %v", err)
		}
		if !strings.HasPrefix(strings.ToLower(mi.ID), "claude") {
			t.Errorf("id = %s", mi.ID)
		}
	})
}

func TestResolveModelSubstring(t *testing.T) {
	r := &Registry{Models: builtinMap()}

	mi, err := r.ResolveModel("mini")
	if err != nil {
		t.Fatalf("ResolveModel: %v", err)
	}
	if !strings.Contains(strings.ToLower(mi.ID), "mini") {
		t.Errorf("id = %s", mi.ID)
	}
}

func TestResolveModelFuzzy(t *testing.T) {
	r := &Registry{Models: builtinMap()}

	// Slight typo should still match
	mi, err := r.ResolveModel("gpt-4oo")
	if err != nil {
		t.Fatalf("ResolveModel: %v", err)
	}
	if mi.ID == "" {
		t.Error("fuzzy match returned empty")
	}
}

func TestResolveModelEmptyName(t *testing.T) {
	r := &Registry{Models: builtinMap()}
	_, err := r.ResolveModel("")
	if err == nil {
		t.Fatal("expected error for empty name")
	}
}

func TestResolveModelNotFound(t *testing.T) {
	r := &Registry{Models: builtinMap()}
	_, err := r.ResolveModel("xyzzy-nosuchmodel-999")
	if err == nil {
		t.Fatal("expected error for unknown model")
	}
}

func TestResolveModelNilRegistry(t *testing.T) {
	var r *Registry
	_, err := r.ResolveModel("gpt-4o")
	if err == nil {
		t.Fatal("expected error for nil registry")
	}
}

func TestResolveModelNilModels(t *testing.T) {
	r := &Registry{}
	_, err := r.ResolveModel("gpt-4o")
	if err == nil {
		t.Fatal("expected error for nil models")
	}
}

func TestLevenshteinDistance(t *testing.T) {
	tests := []struct {
		a, b string
		want int
	}{
		{"", "", 0},
		{"", "abc", 3},
		{"abc", "", 3},
		{"abc", "abc", 0},
		{"abc", "abd", 1},
		{"abc", "ac", 1},
		{"abc", "ab", 1},
		{"kitten", "sitting", 3},
	}

	for _, tt := range tests {
		t.Run(tt.a+"_"+tt.b, func(t *testing.T) {
			got := levenshteinDistance(tt.a, tt.b)
			if got != tt.want {
				t.Errorf("levenshtein(%q, %q) = %d, want %d", tt.a, tt.b, got, tt.want)
			}
		})
	}
}

func TestMin3(t *testing.T) {
	tests := []struct{ a, b, c, want int }{
		{1, 2, 3, 1},
		{3, 2, 1, 1},
		{2, 1, 3, 1},
		{0, 0, 0, 0},
		{-1, 0, 1, -1},
	}
	for _, tt := range tests {
		got := min3(tt.a, tt.b, tt.c)
		if got != tt.want {
			t.Errorf("min3(%d,%d,%d) = %d, want %d", tt.a, tt.b, tt.c, got, tt.want)
		}
	}
}

func TestRoundTrip(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "models.json")

	r1 := &Registry{
		Models: map[string]ModelInfo{
			"custom-model": {ID: "custom-model", Provider: "test", MaxTokens: 1000, SupportsTools: true},
		},
		path: path,
	}
	r1.SaveRegistry()

	r2, err := LoadRegistry(path)
	if err != nil {
		t.Fatalf("LoadRegistry: %v", err)
	}
	mi, err := r2.ResolveModel("custom-model")
	if err != nil {
		t.Fatalf("ResolveModel: %v", err)
	}
	if mi.MaxTokens != 1000 {
		t.Errorf("max tokens = %d", mi.MaxTokens)
	}
}
