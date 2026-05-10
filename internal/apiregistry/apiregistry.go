// Package apiregistry maps API type strings to stream provider constructors.
// Mirrors TypeScript pi-mono's packages/ai/src/api-registry.ts.
//
// The core insight: different LLM APIs (openai-completions, anthropic-messages, etc.)
// have different request/response formats. Instead of hardcoding two client types,
// we register named API handlers that can be extended by third-party providers.
package apiregistry

import (
	"fmt"
	"sync"

	"github.com/huichen/xihu/internal/llm"
	"github.com/huichen/xihu/pkg/types"
)

// API is the identifier for an LLM API protocol.
// Mirrors TS's Api type: "openai-completions", "anthropic-messages", etc.
type API string

const (
	APIOpenAICompletions  API = "openai-completions"
	APIAnthropicMessages  API = "anthropic-messages"
	APIOpenAIResponses    API = "openai-responses"
)

// StreamFactory creates a types.LLMProvider for the given base URL and API key.
// The returned provider's StreamChat method sends events conforming to the
// API type's protocol.
type StreamFactory func(baseURL, apiKey string, opts *llm.StreamOptions) types.LLMProvider

// entry holds a registered API handler.
type entry struct {
	factory  StreamFactory
	sourceID string // which extension registered this (empty = built-in)
}

// newAnthropicClient creates an Anthropic client from the factory signature.
func newAnthropicClient(baseURL, apiKey string, opts *llm.StreamOptions) types.LLMProvider {
	return llm.NewAnthropicClient(baseURL, apiKey)
}

// newOpenAIClient creates an OpenAI-compatible client.
func newOpenAIClient(baseURL, apiKey string, opts *llm.StreamOptions) types.LLMProvider {
	return llm.NewClient(baseURL, apiKey)
}

// ─── Registry ────────────────────────────────────────────────────────────────

var (
	mu       sync.RWMutex
	registry = map[API]entry{}
)

func init() {
	// Register built-in APIs
	Register(APIOpenAICompletions, newOpenAIClient)
	Register(APIAnthropicMessages, newAnthropicClient)
	// OpenAI responses reuses completions client (responses API is a subset)
	Register(APIOpenAIResponses, newOpenAIClient)
}

// Register adds or replaces a StreamFactory for the given API type.
// sourceID is empty for built-in registrations.
func Register(api API, factory StreamFactory) {
	mu.Lock()
	defer mu.Unlock()
	registry[api] = entry{factory: factory}
}

// RegisterFromExtension adds a StreamFactory from an extension.
// The sourceID is used to bulk-unregister when the extension is removed.
func RegisterFromExtension(api API, factory StreamFactory, sourceID string) {
	mu.Lock()
	defer mu.Unlock()
	registry[api] = entry{factory: factory, sourceID: sourceID}
}

// Get returns the registered StreamFactory for the given API type.
func Get(api API) (StreamFactory, error) {
	mu.RLock()
	defer mu.RUnlock()
	e, ok := registry[api]
	if !ok {
		return nil, fmt.Errorf("apiregistry: no handler registered for API %q", api)
	}
	return e.factory, nil
}

// UnregisterFromSource removes all entries registered by a given sourceID.
func UnregisterFromSource(sourceID string) {
	mu.Lock()
	defer mu.Unlock()
	for api, e := range registry {
		if e.sourceID == sourceID {
			delete(registry, api)
		}
	}
}

// LookupAPI tries to infer the API type from a base URL.
// Falls back to openai-completions for unknown URLs.
func LookupAPI(baseURL string) API {
	if contains(baseURL, "anthropic.com") {
		return APIAnthropicMessages
	}
	// Default: OpenAI-compatible (covers OpenAI, DeepSeek, Groq, etc.)
	return APIOpenAICompletions
}

func contains(s, substr string) bool {
	return len(s) >= len(substr) && searchSubstring(s, substr)
}

func searchSubstring(s, substr string) bool {
	for i := 0; i <= len(s)-len(substr); i++ {
		if s[i:i+len(substr)] == substr {
			return true
		}
	}
	return false
}
