// Package modelregistry provides model discovery, registration, and resolution.
// Mirrors TS pi-mono's ModelRegistry + model-resolver.
//
// Models are defined in models.go as an embedded catalog and can be augmented
// at runtime via RegisterProvider (from extension API) or configuration overrides.
package modelregistry

import (
	"log"
	"os"
	"path/filepath"
	"sort"
	"strings"
	"sync"

	"github.com/huichen/xihu/internal/apiregistry"
	"github.com/huichen/xihu/pkg/types"
)

// ProviderOverride stores runtime provider configuration overrides.
type ProviderOverride struct {
	Name    string            `json:"name"`
	BaseURL string            `json:"baseUrl,omitempty"`
	APIKey  string            `json:"apiKey,omitempty"`
	Headers map[string]string `json:"headers,omitempty"`
	Models  []types.Model     `json:"models,omitempty"` // if set, replaces catalog models
}

// Registry holds all known models and runtime overrides.
type Registry struct {
	mu       sync.RWMutex
	catalog  []types.Model      // embedded static catalog
	overrides map[string]ProviderOverride // runtime overrides by provider name

	// scopedModels restricts available models (glob patterns). Empty = no restriction.
	scopedModels []string
}

// New creates a new Registry with the embedded catalog, augmented with
// user-defined models from ~/.xihu/models.json (if present).
func New() *Registry {
	r := &Registry{
		catalog:   modelsCatalog,
		overrides: make(map[string]ProviderOverride),
	}

	// Auto-load user models from ~/.xihu/models.json
	home, err := os.UserHomeDir()
	if err != nil {
		log.Printf("[modelregistry] cannot determine home dir: %v", err)
		return r
	}
	userModelsPath := filepath.Join(home, ".xihu", "models.json")
	userModels, err := LoadUserModels(userModelsPath)
	if err != nil {
		log.Printf("[modelregistry] failed to load user models: %v", err)
		return r
	}
	if len(userModels) > 0 {
		r.catalog = append(r.catalog, userModels...)
		log.Printf("[modelregistry] loaded %d user model(s) from %s", len(userModels), userModelsPath)
	}

	return r
}

// ─── Query ───────────────────────────────────────────────────────────────────

// Find locates a model by exact provider + modelID.
// Also matches when modelID contains the provider prefix (e.g. "openai/gpt-4o").
func (r *Registry) Find(provider, modelID string) (types.Model, bool) {
	r.mu.RLock()
	defer r.mu.RUnlock()
	return r.findLocked(provider, modelID)
}

func (r *Registry) findLocked(provider, modelID string) (types.Model, bool) {
	// Extract provider from "provider/model" format
	if idx := strings.Index(modelID, "/"); idx >= 0 {
		provider = modelID[:idx]
		modelID = modelID[idx+1:]
	}

	// Check overrides first
	if ov, ok := r.overrides[provider]; ok {
		if len(ov.Models) > 0 {
			for _, m := range ov.Models {
				if m.ID == modelID {
					return r.applyOverride(m, ov), true
				}
			}
		}
	}

	// Search catalog
	for _, m := range r.catalog {
		if m.Provider == provider && m.ID == modelID {
			// Apply override if base URL or headers are set
			if ov, ok := r.overrides[provider]; ok {
				return r.applyOverride(m, ov), true
			}
			return m, true
		}
	}
	return types.Model{}, false
}

// Resolve tries to find a model by various patterns, mirroring TS resolveModelPattern.
// Supports: "modelID", "provider/modelID", partial matches, alias preference.
func (r *Registry) Resolve(pattern string) (types.Model, bool) {
	r.mu.RLock()
	defer r.mu.RUnlock()

	// 1. Exact "provider/model" match
	if idx := strings.Index(pattern, "/"); idx >= 0 {
		provider := pattern[:idx]
		modelID := pattern[idx+1:]
		if m, ok := r.findLocked(provider, modelID); ok {
			return m, true
		}
	}

	// 2. Search by model ID across all providers
	for _, m := range r.catalog {
		if m.ID == pattern {
			if ov, ok := r.overrides[m.Provider]; ok {
				return r.applyOverride(m, ov), true
			}
			return m, true
		}
	}

	// 3. Search override models
	for provider, ov := range r.overrides {
		for _, m := range ov.Models {
			if m.ID == pattern {
				// ensure provider field
				if m.Provider == "" {
					m.Provider = provider
				}
				return r.applyOverride(m, ov), true
			}
		}
	}

	// 4. Prefix match (e.g. "claude-sonnet" matches "claude-sonnet-4-20250514")
	var candidates []types.Model
	for _, m := range r.catalog {
		if strings.HasPrefix(m.ID, pattern) {
			candidates = append(candidates, m)
		}
	}
	// Sort by ID length ascending (shortest = best match)
	sort.Slice(candidates, func(i, j int) bool {
		return len(candidates[i].ID) < len(candidates[j].ID)
	})
	if len(candidates) > 0 {
		m := candidates[0]
		if ov, ok := r.overrides[m.Provider]; ok {
			return r.applyOverride(m, ov), true
		}
		return m, true
	}

	return types.Model{}, false
}

// GetAll returns all models from the catalog, with overrides applied.
func (r *Registry) GetAll() []types.Model {
	r.mu.RLock()
	defer r.mu.RUnlock()

	seen := make(map[string]bool)
	var result []types.Model

	// Override models first
	for provider, ov := range r.overrides {
		if len(ov.Models) > 0 {
			for _, m := range ov.Models {
				if m.Provider == "" {
					m.Provider = provider
				}
				key := m.Provider + "/" + m.ID
				if !seen[key] {
					seen[key] = true
					result = append(result, r.applyOverride(m, ov))
				}
			}
		}
	}

	// Catalog models (not replaced by overrides)
	for _, m := range r.catalog {
		if ov, ok := r.overrides[m.Provider]; ok && len(ov.Models) > 0 {
			continue // provider fully replaced
		}
		key := m.Provider + "/" + m.ID
		if !seen[key] {
			seen[key] = true
			if ov, ok := r.overrides[m.Provider]; ok {
				result = append(result, r.applyOverride(m, ov))
			} else {
				result = append(result, m)
			}
		}
	}

	return result
}

// GetAvailable returns models that can be authenticated (have API key configured).
func (r *Registry) GetAvailable(hasAuth func(provider string) bool) []types.Model {
	all := r.GetAll()
	var available []types.Model
	for _, m := range all {
		if hasAuth(m.Provider) {
			available = append(available, m)
		}
	}
	return available
}

// ─── Scoped Models ───────────────────────────────────────────────────────────

// SetScopedModels sets the allowed model list. Empty = no restriction.
// Patterns support glob-like matching: "provider/*", "provider/modelID".
func (r *Registry) SetScopedModels(patterns []string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.scopedModels = patterns
}

// MatchScoped checks if a model is in the scoped list.
func (r *Registry) MatchScoped(provider, modelID string) bool {
	r.mu.RLock()
	defer r.mu.RUnlock()
	if len(r.scopedModels) == 0 {
		return true // no restriction
	}
	full := provider + "/" + modelID
	for _, p := range r.scopedModels {
		if matchPattern(p, full, provider, modelID) {
			return true
		}
	}
	return false
}

// matchPattern checks if a scoped pattern matches a model.
func matchPattern(pattern, full, provider, modelID string) bool {
	if pattern == full || pattern == modelID {
		return true
	}
	if strings.HasSuffix(pattern, "/*") {
		prefix := strings.TrimSuffix(pattern, "/*")
		return provider == prefix
	}
	if idx := strings.Index(pattern, "/"); idx >= 0 {
		return pattern == full
	}
	return pattern == modelID
}

// ─── Provider Overrides ──────────────────────────────────────────────────────

// RegisterProvider adds or replaces a provider with the given configuration.
// Mirrors TS's pi.registerProvider().
func (r *Registry) RegisterProvider(name string, override ProviderOverride) {
	r.mu.Lock()
	defer r.mu.Unlock()
	r.overrides[name] = override
}

// UnregisterProvider removes a dynamically registered provider.
func (r *Registry) UnregisterProvider(name string) {
	r.mu.Lock()
	defer r.mu.Unlock()
	delete(r.overrides, name)
}

// GetProviderOverride returns the override for a provider, if any.
func (r *Registry) GetProviderOverride(name string) (ProviderOverride, bool) {
	r.mu.RLock()
	defer r.mu.RUnlock()
	ov, ok := r.overrides[name]
	return ov, ok
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

// applyOverride applies a provider override to a catalog model.
func (r *Registry) applyOverride(m types.Model, ov ProviderOverride) types.Model {
	result := m
	if ov.BaseURL != "" {
		result.BaseURL = ov.BaseURL
	}
	if len(ov.Headers) > 0 {
		if result.Headers == nil {
			result.Headers = make(map[string]string)
		}
		for k, v := range ov.Headers {
			result.Headers[k] = v
		}
	}
	return result
}

// DetermineAPI determines the API type from a base URL or model.
func DetermineAPI(baseURL string) apiregistry.API {
	return apiregistry.LookupAPI(baseURL)
}

// DefaultModel returns a sensible default model ID for a provider.
func DefaultModel(provider string) string {
	switch provider {
	case "anthropic":
		return "claude-sonnet-4-20250514"
	case "openai":
		return "gpt-4o"
	case "deepseek":
		return "deepseek-chat"
	case "google":
		return "gemini-2.5-flash"
	case "alibaba":
		return "qwen3.6-plus"
	case "xai":
		return "grok-5-mini"
	case "moonshot":
		return "kimi-k2-turbo"
	case "zhipu":
		return "glm-5"
	case "minimax":
		return "minimax-m2.5"
	case "bytedance":
		return "doubao-pro-256k"
	case "mistral":
		return "mistral-large"
	case "cloudflare":
		return "@cf/meta/llama-4-maverick-17b-128e-instruct"
	case "aws":
		return "us.anthropic.claude-sonnet-4-20250514-v1:0"
	case "groq":
		return "llama-4-maverick-17b-128e-instruct"
	case "fireworks":
		return "accounts/fireworks/models/llama-v3p1-405b-instruct"
	case "together":
		return "meta-llama/Llama-4-Maverick-17B-128E-Instruct-FP8"
	default:
		return ""
	}
}
