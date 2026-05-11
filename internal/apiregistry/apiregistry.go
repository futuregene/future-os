// Package apiregistry maps API type strings to stream provider constructors.
// Mirrors TypeScript pi-mono's packages/ai/src/api-registry.ts.
//
// The core insight: different LLM APIs (openai-completions, anthropic-messages, etc.)
// have different request/response formats. Instead of hardcoding two client types,
// we register named API handlers that can be extended by third-party providers.
package apiregistry

import (
	"fmt"
	"strings"
	"sync"

	"github.com/huichen/xihu/internal/llm"
	"github.com/huichen/xihu/pkg/types"
)

// API is the identifier for an LLM API protocol.
// Mirrors TS's Api type: "openai-completions", "anthropic-messages", etc.
type API string

const (
	APIOpenAICompletions API = "openai-completions"
	APIAnthropicMessages API = "anthropic-messages"
	APIOpenAIResponses   API = "openai-responses"
	APIGoogleGemini      API = "google-gemini"
	APIMistral           API = "mistral"
	APICloudflare        API = "cloudflare-workers"
)

// StreamFactory creates a types.LLMProvider for the given base URL and API key.
// The returned provider's StreamChat method sends events conforming to the
// API type's protocol.
type StreamFactory func(baseURL, apiKey string, opts *llm.StreamOptions) types.LLMProvider

// ThinkingBudgetSetter is implemented by providers that support setting a
// thinking budget after construction (e.g., LazyProvider and llm.Client).
type ThinkingBudgetSetter interface {
	SetThinkingBudget(budget int)
}

// entry holds a registered API handler.
type entry struct {
	factory  StreamFactory
	sourceID string // which extension registered this (empty = built-in)
}

// newAnthropicClient creates a lazily-constructed Anthropic client.
func newAnthropicClient(baseURL, apiKey string, opts *llm.StreamOptions) types.LLMProvider {
	return NewLazyProvider(baseURL, apiKey, opts, func(bu, ak string, o *llm.StreamOptions) types.LLMProvider {
		return llm.NewAnthropicClient(bu, ak)
	})
}

// newOpenAIClient creates a lazily-constructed OpenAI-compatible client.
func newOpenAIClient(baseURL, apiKey string, opts *llm.StreamOptions) types.LLMProvider {
	return NewLazyProvider(baseURL, apiKey, opts, func(bu, ak string, o *llm.StreamOptions) types.LLMProvider {
		return llm.NewClient(bu, ak)
	})
}

// ─── LazyProvider ─────────────────────────────────────────────────────────────

// LazyProvider wraps a StreamFactory and defers client construction until
// the first StreamChat call. This avoids constructing HTTP clients for
// unused providers (e.g., providers registered by extensions that are
// never selected by the user).
type LazyProvider struct {
	baseURL string
	apiKey  string
	opts    *llm.StreamOptions
	factory StreamFactory

	mu             sync.Mutex
	provider       types.LLMProvider // constructed on first use
	thinkingBudget int               // budget to apply to llm.Client on construction
}

// NewLazyProvider creates a LazyProvider that defers calling factory until
// StreamChat is first invoked.
func NewLazyProvider(baseURL, apiKey string, opts *llm.StreamOptions, factory StreamFactory) *LazyProvider {
	return &LazyProvider{
		baseURL: baseURL,
		apiKey:  apiKey,
		opts:    opts,
		factory: factory,
	}
}

// SetThinkingBudget sets the thinking budget to apply when the underlying
// provider is constructed. If the provider has already been materialized and
// is an *llm.Client, the budget is applied immediately.
func (lp *LazyProvider) SetThinkingBudget(budget int) {
	lp.mu.Lock()
	defer lp.mu.Unlock()
	lp.thinkingBudget = budget
	if lp.provider != nil {
		if cl, ok := lp.provider.(*llm.Client); ok {
			cl.ThinkingBudget = budget
		}
	}
}

// StreamChat materializes the underlying provider on first call and delegates.
func (lp *LazyProvider) StreamChat(model string, messages []types.Message, tools []types.ToolDef, systemPrompt string) (<-chan types.StreamEvent, error) {
	lp.mu.Lock()
	if lp.provider == nil {
		lp.provider = lp.factory(lp.baseURL, lp.apiKey, lp.opts)
		// Apply deferred thinking budget
		if lp.thinkingBudget > 0 {
			if cl, ok := lp.provider.(*llm.Client); ok {
				cl.ThinkingBudget = lp.thinkingBudget
			}
		}
	}
	prov := lp.provider
	lp.mu.Unlock()
	return prov.StreamChat(model, messages, tools, systemPrompt)
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
	// Mistral uses OpenAI-compatible API
	Register(APIMistral, newOpenAIClient)
	// Cloudflare Workers AI — OpenAI-compatible chain for models served via Workers
	Register(APICloudflare, newOpenAIClient)
	// Google Gemini — uses OpenAI-compatible endpoint with Bearer auth
	// (when proxied through generativelanguage.googleapis.com/v1beta/openai)
	Register(APIGoogleGemini, newOpenAIClient)
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

// LookupAPI infers the API type from a base URL using provider-specific patterns.
// For OpenAI-compatible providers (DeepSeek, Groq, Alibaba, etc.), returns
// openai-completions. Falls back to openai-completions for unknown URLs.
func LookupAPI(baseURL string) API {
	lower := strings.ToLower(baseURL)

	// Anthropic
	if strings.Contains(lower, "anthropic.com") {
		return APIAnthropicMessages
	}

	// Google: both native Gemini and OpenAI-compatible proxy
	if strings.Contains(lower, "googleapis.com") || strings.Contains(lower, "generativelanguage") {
		// When the URL path contains "/openai", it's the OpenAI-compatible endpoint
		if strings.Contains(lower, "/openai") {
			return APIOpenAICompletions
		}
		return APIGoogleGemini
	}

	// Mistral
	if strings.Contains(lower, "mistral.ai") {
		return APIMistral
	}

	// Cloudflare Workers AI
	if strings.Contains(lower, "cloudflare.com") || strings.Contains(lower, "workers.dev") {
		return APICloudflare
	}

	// DeepSeek
	if strings.Contains(lower, "deepseek.com") {
		return APIOpenAICompletions
	}

	// Alibaba / Qwen
	if strings.Contains(lower, "aliyuncs.com") {
		return APIOpenAICompletions
	}

	// xAI / Grok
	if strings.Contains(lower, "x.ai") {
		return APIOpenAICompletions
	}

	// Moonshot / Kimi
	if strings.Contains(lower, "moonshot.cn") {
		return APIOpenAICompletions
	}

	// Zhipu / GLM
	if strings.Contains(lower, "bigmodel.cn") {
		return APIOpenAICompletions
	}

	// MiniMax
	if strings.Contains(lower, "minimax.chat") || strings.Contains(lower, "minimaxi.com") {
		return APIOpenAICompletions
	}

	// ByteDance / Doubao / Volcengine
	if strings.Contains(lower, "volces.com") {
		return APIOpenAICompletions
	}

	// OpenAI (catch-all for api.openai.com and unknown)
	return APIOpenAICompletions
}

// LookupProviderFromURL heuristically extracts a provider name from a base URL.
// Used as fallback when the model isn't found in the registry.
func LookupProviderFromURL(baseURL string) string {
	lower := strings.ToLower(baseURL)
	switch {
	case strings.Contains(lower, "anthropic.com"):
		return "anthropic"
	case strings.Contains(lower, "googleapis.com"), strings.Contains(lower, "generativelanguage"):
		return "google"
	case strings.Contains(lower, "mistral.ai"):
		return "mistral"
	case strings.Contains(lower, "cloudflare.com"), strings.Contains(lower, "workers.dev"):
		return "cloudflare"
	case strings.Contains(lower, "deepseek.com"):
		return "deepseek"
	case strings.Contains(lower, "aliyuncs.com"):
		return "alibaba"
	case strings.Contains(lower, "x.ai"):
		return "xai"
	case strings.Contains(lower, "moonshot.cn"):
		return "moonshot"
	case strings.Contains(lower, "bigmodel.cn"):
		return "zhipu"
	case strings.Contains(lower, "minimax.chat"), strings.Contains(lower, "minimaxi.com"):
		return "minimax"
	case strings.Contains(lower, "volces.com"):
		return "bytedance"
	case strings.Contains(lower, "openai.com"):
		return "openai"
	default:
		return "openai"
	}
}
