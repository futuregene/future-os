package modelregistry

import (
	"testing"

	"github.com/huichen/xihu/pkg/types"
)

func TestNewRegistry(t *testing.T) {
	r := New()
	if r == nil {
		t.Fatal("nil registry")
	}
	all := r.GetAll()
	if len(all) == 0 {
		t.Fatal("expected catalog models, got none")
	}
}

func TestFind(t *testing.T) {
	r := New()
	m, ok := r.Find("deepseek", "deepseek-chat")
	if !ok {
		t.Fatal("expected to find deepseek-chat")
	}
	if m.Provider != "deepseek" {
		t.Errorf("provider = %q", m.Provider)
	}
	if m.ContextWindow != 65536 {
		t.Errorf("contextWindow = %d", m.ContextWindow)
	}
}

func TestFindNotFound(t *testing.T) {
	r := New()
	_, ok := r.Find("nonexistent", "no-model")
	if ok {
		t.Error("should not find nonexistent model")
	}
}

func TestResolve(t *testing.T) {
	r := New()

	// Exact match
	m, ok := r.Resolve("deepseek/deepseek-chat")
	if !ok {
		t.Error("expected to resolve deepseek/deepseek-chat")
	}
	if m.ID != "deepseek-chat" {
		t.Errorf("resolved ID = %q", m.ID)
	}

	// Model ID only
	m, ok = r.Resolve("claude-sonnet-4-20250514")
	if !ok {
		t.Error("expected to resolve by model ID only")
	}
	if m.Provider != "anthropic" {
		t.Errorf("resolved provider = %q", m.Provider)
	}

	// Prefix match
	m, ok = r.Resolve("gpt-4o")
	if !ok {
		t.Error("expected to resolve gpt-4o by prefix")
	}
	if m.ID != "gpt-4o" {
		t.Errorf("resolved ID = %q", m.ID)
	}
}

func TestRegisterProvider(t *testing.T) {
	r := New()
	override := ProviderOverride{
		Name:    "my-proxy",
		BaseURL: "https://my-proxy.example.com/v1",
		Models: []types.Model{
			{ID: "proxy-model", Provider: "my-proxy", ContextWindow: 128000},
		},
	}
	r.RegisterProvider("my-proxy", override)

	m, ok := r.Find("my-proxy", "proxy-model")
	if !ok {
		t.Fatal("expected to find registered model")
	}
	if m.BaseURL != "https://my-proxy.example.com/v1" {
		t.Errorf("override BaseURL not applied: %q", m.BaseURL)
	}
}

func TestProviderURLOverride(t *testing.T) {
	r := New()
	override := ProviderOverride{
		Name:    "openai",
		BaseURL: "https://custom-openai.example.com/v1",
	}
	r.RegisterProvider("openai", override)

	m, ok := r.Find("openai", "gpt-4o")
	if !ok {
		t.Fatal("expected to find gpt-4o")
	}
	if m.BaseURL != "https://custom-openai.example.com/v1" {
		t.Errorf("override not applied: %q", m.BaseURL)
	}
}

func TestUnregisterProvider(t *testing.T) {
	r := New()
	override := ProviderOverride{
		Name:   "temp",
		Models: []types.Model{{ID: "temp-model", Provider: "temp"}},
	}
	r.RegisterProvider("temp", override)

	_, ok := r.Find("temp", "temp-model")
	if !ok {
		t.Fatal("expected to find temp model before unregister")
	}

	r.UnregisterProvider("temp")
	_, ok = r.Find("temp", "temp-model")
	if ok {
		t.Error("should not find temp model after unregister")
	}
}

func TestMatchScoped(t *testing.T) {
	r := New()
	r.SetScopedModels([]string{"deepseek/*"})

	if !r.MatchScoped("deepseek", "deepseek-chat") {
		t.Error("deepseek-chat should match deepseek/*")
	}
	if r.MatchScoped("openai", "gpt-4o") {
		t.Error("openai/gpt-4o should NOT match deepseek/*")
	}
}

func TestDefaultModel(t *testing.T) {
	if d := DefaultModel("anthropic"); d == "" {
		t.Error("expected default for anthropic")
	}
	if d := DefaultModel("unknown"); d != "" {
		t.Errorf("expected empty for unknown, got %q", d)
	}
}
