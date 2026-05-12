package models

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func TestLoadRegistryInvalidJSON(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "invalid.json")
	os.WriteFile(path, []byte("not json"), 0644)

	_, err := LoadRegistry(path)
	if err == nil {
		t.Fatal("expected error for invalid JSON")
	}
}

func TestLoadRegistryPiFormat(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "models.json")
	os.WriteFile(path, []byte(`{
		"providers": {
			"test-provider": {
				"baseUrl": "https://api.test.com/v1",
				"api": "openai-completions",
				"models": [
					{"id": "test-model", "name": "Test Model", "contextWindow": 128000}
				]
			}
		}
	}`), 0644)

	r, err := LoadRegistry(path)
	if err != nil {
		t.Fatalf("LoadRegistry: %v", err)
	}
	if len(r.Models) != 1 {
		t.Fatalf("expected 1 model, got %d", len(r.Models))
	}
	if r.Models[0].ID != "test-model" {
		t.Errorf("id = %s", r.Models[0].ID)
	}
	if r.Models[0].ContextWindow != 128000 {
		t.Errorf("contextWindow = %d", r.Models[0].ContextWindow)
	}
}

func TestLoadRegistryLegacyFormat(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "models.json")
	os.WriteFile(path, []byte(`{"models":{"gpt-4o":{"id":"gpt-4o","provider":"openai","api":"openai-completions","base_url":"https://api.openai.com/v1","max_tokens":128000,"supports_vision":true}}}`), 0644)

	r, err := LoadRegistry(path)
	if err != nil {
		t.Fatalf("LoadRegistry: %v", err)
	}
	found := false
	for _, m := range r.Models {
		if m.ID == "gpt-4o" {
			found = true
			break
		}
	}
	if !found {
		t.Fatal("gpt-4o not found after migration")
	}
}

func TestLoadRegistryNullProviders(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "models.json")
	os.WriteFile(path, []byte(`{}`), 0644)

	r, err := LoadRegistry(path)
	if err == nil {
		if len(r.Models) > 0 {
			t.Fatal("expected no models for empty config")
		}
	}
}

func TestSaveRegistry(t *testing.T) {
	tmpDir := t.TempDir()
	path := filepath.Join(tmpDir, "models.json")

	cfg := &ModelsConfig{Providers: map[string]ProviderConfig{
		"test": {
			BaseURL: strPtr("https://api.test.com/v1"),
			API:     strPtr("openai-completions"),
			Models: []ModelDefinition{
				{ID: "test-model", Name: strPtr("Test Model")},
			},
		},
	}}
	r := &Registry{config: cfg, Models: resolveModels(cfg), rawPath: path}

	err := r.Save()
	if err != nil {
		t.Fatalf("Save: %v", err)
	}

	data, err := os.ReadFile(path)
	if err != nil {
		t.Fatalf("read saved file: %v", err)
	}
	if len(data) == 0 {
		t.Fatal("saved file is empty")
	}

	// Verify it can be reloaded
	r2, err := LoadRegistry(path)
	if err != nil {
		t.Fatalf("reload: %v", err)
	}
	if len(r2.Models) != 1 {
		t.Fatalf("expected 1 model after reload, got %d", len(r2.Models))
	}
}

func TestSaveRegistryNil(t *testing.T) {
	var r *Registry
	err := r.Save()
	if err == nil {
		t.Fatal("expected error for nil registry")
	}
}

func TestSaveRegistryNilConfig(t *testing.T) {
	r := &Registry{}
	err := r.Save()
	if err == nil {
		t.Fatal("expected error for nil config")
	}
}

func TestDefaultPath(t *testing.T) {
	path := DefaultPath()
	if path == "" {
		t.Fatal("path is empty")
	}
	if !strings.Contains(path, "models.json") {
		t.Errorf("path = %s", path)
	}
	if !strings.Contains(path, ".xihu") {
		t.Errorf("path should contain .xihu: %s", path)
	}
}

func TestXihuDefaultPath(t *testing.T) {
	path := XihuDefaultPath()
	if path == "" {
		t.Fatal("path is empty")
	}
	if !strings.Contains(path, ".xihu") {
		t.Errorf("path should contain .xihu: %s", path)
	}
}

func TestFindExactMatch(t *testing.T) {
	r := makeTestRegistry()

	m, err := r.Find("gpt-4o")
	if err != nil {
		t.Fatalf("Find: %v", err)
	}
	if m.ID != "gpt-4o" {
		t.Errorf("id = %s", m.ID)
	}
}

func TestFindProviderSlashID(t *testing.T) {
	r := makeTestRegistry()

	m, err := r.Find("openai/gpt-4o")
	if err != nil {
		t.Fatalf("Find: %v", err)
	}
	if m.ID != "gpt-4o" || m.Provider != "openai" {
		t.Errorf("id = %s, provider = %s", m.ID, m.Provider)
	}
}

func TestFindCaseInsensitive(t *testing.T) {
	r := makeTestRegistry()

	m, err := r.Find("GPT-4O")
	if err != nil {
		t.Fatalf("Find: %v", err)
	}
	if !strings.EqualFold(m.ID, "gpt-4o") {
		t.Errorf("id = %s", m.ID)
	}
}

func TestFindPrefix(t *testing.T) {
	r := makeTestRegistry()

	m, err := r.Find("gpt-4")
	if err != nil {
		t.Fatalf("Find: %v", err)
	}
	if !strings.HasPrefix(m.ID, "gpt-4") {
		t.Errorf("id = %s, expected gpt-4 prefix", m.ID)
	}
}

func TestFindSubstring(t *testing.T) {
	r := makeTestRegistry()

	m, err := r.Find("mini")
	if err != nil {
		t.Fatalf("Find: %v", err)
	}
	if !strings.Contains(strings.ToLower(m.ID), "mini") {
		t.Errorf("id = %s", m.ID)
	}
}

func TestFindEmptyQuery(t *testing.T) {
	r := makeTestRegistry()

	_, err := r.Find("")
	if err == nil {
		t.Fatal("expected error for empty query")
	}
}

func TestFindNotFound(t *testing.T) {
	r := makeTestRegistry()

	_, err := r.Find("xyzzy-nosuchmodel-999")
	if err == nil {
		t.Fatal("expected error for unknown model")
	}
}

func TestFindNilRegistry(t *testing.T) {
	var r *Registry
	_, err := r.Find("gpt-4o")
	if err == nil {
		t.Fatal("expected error for nil registry")
	}
}

func TestGetAll(t *testing.T) {
	r := makeTestRegistry()

	all := r.GetAll()
	if len(all) != 3 {
		t.Errorf("GetAll = %d, want 3", len(all))
	}
}

func TestGetByProvider(t *testing.T) {
	r := makeTestRegistry()

	openaiModels := r.GetByProvider("openai")
	if len(openaiModels) == 0 {
		t.Error("expected OpenAI models")
	}
	for _, m := range openaiModels {
		if m.Provider != "openai" {
			t.Errorf("provider = %s, want openai", m.Provider)
		}
	}
}

func TestSupportsImage(t *testing.T) {
	m := &ResolvedModel{Input: []string{"text", "image"}}
	if !m.SupportsImage() {
		t.Error("expected SupportsImage = true")
	}

	m2 := &ResolvedModel{Input: []string{"text"}}
	if m2.SupportsImage() {
		t.Error("expected SupportsImage = false")
	}
}

func TestSupportsText(t *testing.T) {
	m := &ResolvedModel{Input: []string{"text", "image"}}
	if !m.SupportsText() {
		t.Error("expected SupportsText = true")
	}
}

func TestModelOverrideMerge(t *testing.T) {
	cfg := &ModelsConfig{Providers: map[string]ProviderConfig{
		"test": {
			BaseURL: strPtr("https://api.test.com/v1"),
			API:     strPtr("openai-completions"),
			Models: []ModelDefinition{
				{
					ID:            "test-model",
					ContextWindow: intPtr(128000),
					MaxTokens:     intPtr(8192),
					Cost:          &ModelCost{Input: 1.0, Output: 3.0},
				},
			},
			ModelOverrides: map[string]ModelOverride{
				"test-model": {
					ContextWindow: intPtr(256000),
					Cost:          &ModelCost{Input: 2.0},
				},
			},
		},
	}}

	models := resolveModels(cfg)
	if len(models) != 1 {
		t.Fatalf("expected 1 model, got %d", len(models))
	}
	m := models[0]

	if m.ContextWindow != 256000 {
		t.Errorf("contextWindow = %d, want 256000", m.ContextWindow)
	}
	if m.Cost.Input != 2.0 {
		t.Errorf("cost.input = %f, want 2.0", m.Cost.Input)
	}
	if m.Cost.Output != 3.0 {
		t.Errorf("cost.output = %f, want 3.0", m.Cost.Output)
	}
}

// makeTestRegistry creates a minimal in-memory registry for Find/Get tests.
func makeTestRegistry() *Registry {
	cfg := &ModelsConfig{Providers: map[string]ProviderConfig{
		"openai": {
			BaseURL: strPtr("https://api.openai.com/v1"),
			API:     strPtr("openai-completions"),
			Models: []ModelDefinition{
				{ID: "gpt-4o", Name: strPtr("GPT-4o"), ContextWindow: intPtr(128000)},
				{ID: "gpt-4o-mini", Name: strPtr("GPT-4o Mini"), ContextWindow: intPtr(128000)},
				{ID: "o3-mini", Name: strPtr("O3 Mini"), ContextWindow: intPtr(200000)},
			},
		},
	}}
	return &Registry{config: cfg, Models: resolveModels(cfg)}
}

// Helpers
func strPtr(s string) *string { return &s }
func intPtr(i int) *int       { return &i }
func jsonUnmarshal(data []byte, v interface{}) error {
	return json.Unmarshal(data, v)
}
