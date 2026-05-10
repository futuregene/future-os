package apiregistry

import (
	"testing"

	"github.com/huichen/xihu/internal/llm"
	"github.com/huichen/xihu/pkg/types"
)

func TestRegisterAndGet(t *testing.T) {
	var called bool
	var lastBaseURL, lastAPIKey string
	factory := func(baseURL, apiKey string, opts *llm.StreamOptions) types.LLMProvider {
		called = true
		lastBaseURL = baseURL
		lastAPIKey = apiKey
		return nil
	}
	Register(API("test-api"), factory)
	f, err := Get(API("test-api"))
	if err != nil {
		t.Fatalf("expected factory, got error: %v", err)
	}
	f("http://test-url", "test-key", nil)
	if !called {
		t.Error("factory was not called")
	}
	if lastBaseURL != "http://test-url" {
		t.Errorf("baseURL = %q", lastBaseURL)
	}
	if lastAPIKey != "test-key" {
		t.Errorf("apiKey = %q", lastAPIKey)
	}

	// Get unregistered API should fail
	_, err = Get(API("nonexistent"))
	if err == nil {
		t.Error("expected error for unregistered API")
	}
}

func TestBuiltinsExist(t *testing.T) {
	for _, api := range []API{APIOpenAICompletions, APIAnthropicMessages, APIOpenAIResponses} {
		f, err := Get(api)
		if err != nil {
			t.Errorf("built-in API %q not registered: %v", api, err)
		}
		if f == nil {
			t.Errorf("built-in API %q has nil factory", api)
		}
	}
}

func TestLookupAPI(t *testing.T) {
	tests := []struct {
		url      string
		expected API
	}{
		{"https://api.openai.com/v1", APIOpenAICompletions},
		{"https://api.anthropic.com/v1", APIAnthropicMessages},
		{"https://api.deepseek.com/v1", APIOpenAICompletions},
		{"https://unknown.example.com/v1", APIOpenAICompletions},
	}
	for _, tt := range tests {
		got := LookupAPI(tt.url)
		if got != tt.expected {
			t.Errorf("LookupAPI(%q) = %q, want %q", tt.url, got, tt.expected)
		}
	}
}

func TestRegisterFromExtensionAndUnregister(t *testing.T) {
	extAPI := API("ext-api")
	factory := func(baseURL, apiKey string, opts *llm.StreamOptions) types.LLMProvider { return nil }
	RegisterFromExtension(extAPI, factory, "ext-1")

	f, err := Get(extAPI)
	if err != nil {
		t.Fatalf("expected factory: %v", err)
	}
	if f == nil {
		t.Fatal("nil factory")
	}

	UnregisterFromSource("ext-1")
	_, err = Get(extAPI)
	if err == nil {
		t.Error("expected error after unregister")
	}
}
