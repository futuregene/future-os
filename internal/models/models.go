// Package models provides a model registry aligned with pi's models.json format.
// Models are loaded from ~/.xihu/models.json.
// The format uses a provider-centric structure:
//
//	{
//	  "providers": {
//	    "provider-name": {
//	      "baseUrl": "...",
//	      "api": "openai-completions",
//	      "apiKey": "...",
//	      "compat": { "supportsDeveloperRole": false, ... },
//	      "models": [{
//	        "id": "qwen3.6-plus",
//	        "name": "Qwen3.6 Plus",
//	        "reasoning": true,
//	        "input": ["text", "image"],
//	        "contextWindow": 1000000,
//	        "maxTokens": 65536,
//	        "cost": { "input": 2, "output": 8, "cacheRead": 0, "cacheWrite": 0 },
//	        "thinkingLevelMap": { "high": "32768" },
//	        "headers": {},
//	        "compat": {}
//	      }],
//	      "modelOverrides": {
//	        "qwen3.6-plus": { "contextWindow": 500000 }
//	      }
//	    }
//	  }
//	}

package models

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// ---------------------------------------------------------------------------
// Types — aligned with pi's model-registry.ts schema
// ---------------------------------------------------------------------------

// ModelCost represents pricing per 1M tokens (matching pi's cost format).
type ModelCost struct {
	Input      float64 `json:"input"`
	Output     float64 `json:"output"`
	CacheRead  float64 `json:"cacheRead"`
	CacheWrite float64 `json:"cacheWrite"`
}

// ThinkingLevelMap stores per-level provider-specific values (e.g., max tokens).
type ThinkingLevelMap struct {
	Off      *string `json:"off,omitempty"`
	Minimal  *string `json:"minimal,omitempty"`
	Low      *string `json:"low,omitempty"`
	Medium   *string `json:"medium,omitempty"`
	High     *string `json:"high,omitempty"`
	XHigh    *string `json:"xhigh,omitempty"`
}

// OpenAICompletionsCompat holds compatibility flags for OpenAI completions API.
type OpenAICompletionsCompat struct {
	SupportsStore                *bool   `json:"supportsStore,omitempty"`
	SupportsDeveloperRole        *bool   `json:"supportsDeveloperRole,omitempty"`
	SupportsReasoningEffort      *bool   `json:"supportsReasoningEffort,omitempty"`
	SupportsUsageInStreaming     *bool   `json:"supportsUsageInStreaming,omitempty"`
	MaxTokensField               *string `json:"maxTokensField,omitempty"`                // "max_completion_tokens" | "max_tokens"
	RequiresToolResultName       *bool   `json:"requiresToolResultName,omitempty"`
	RequiresAssistantAfterTool   *bool   `json:"requiresAssistantAfterToolResult,omitempty"`
	RequiresThinkingAsText       *bool   `json:"requiresThinkingAsText,omitempty"`
	RequiresReasoningOnAssistant *bool   `json:"requiresReasoningContentOnAssistantMessages,omitempty"`
	ThinkingFormat               *string `json:"thinkingFormat,omitempty"`                // "openai" | "openrouter" | "deepseek" | "zai" | "qwen" | "qwen-chat-template"
	CacheControlFormat           *string `json:"cacheControlFormat,omitempty"`            // "anthropic"
}

// AnthropicMessagesCompat holds compatibility flags for Anthropic messages API.
type AnthropicMessagesCompat struct {
	SupportsEagerToolInputStreaming *bool `json:"supportsEagerToolInputStreaming,omitempty"`
	SupportsLongCacheRetention      *bool `json:"supportsLongCacheRetention,omitempty"`
}

// OpenAIResponsesCompat holds compatibility flags for OpenAI responses API.
type OpenAIResponsesCompat struct {
	SendSessionIDHeader      *bool `json:"sendSessionIdHeader,omitempty"`
	SupportsLongCacheRetention *bool `json:"supportsLongCacheRetention,omitempty"`
}

// Compat is the union of all compat types (only one is used per API type).
type Compat struct {
	// OpenAI completions
	SupportsStore                *bool   `json:"supportsStore,omitempty"`
	SupportsDeveloperRole        *bool   `json:"supportsDeveloperRole,omitempty"`
	SupportsReasoningEffort      *bool   `json:"supportsReasoningEffort,omitempty"`
	SupportsUsageInStreaming     *bool   `json:"supportsUsageInStreaming,omitempty"`
	MaxTokensField               *string `json:"maxTokensField,omitempty"`
	RequiresToolResultName       *bool   `json:"requiresToolResultName,omitempty"`
	RequiresAssistantAfterTool   *bool   `json:"requiresAssistantAfterToolResult,omitempty"`
	RequiresThinkingAsText       *bool   `json:"requiresThinkingAsText,omitempty"`
	RequiresReasoningOnAssistant *bool   `json:"requiresReasoningContentOnAssistantMessages,omitempty"`
	ThinkingFormat               *string `json:"thinkingFormat,omitempty"`
	CacheControlFormat           *string `json:"cacheControlFormat,omitempty"`
	// Anthropic messages
	SupportsEagerToolInputStreaming *bool `json:"supportsEagerToolInputStreaming,omitempty"`
	SupportsLongCacheRetentionC    *bool `json:"supportsLongCacheRetention,omitempty"`
	// OpenAI responses
	SendSessionIDHeader *bool `json:"sendSessionIdHeader,omitempty"`
}

// ModelDefinition defines a model within a provider (matching pi's ModelDefinitionSchema).
type ModelDefinition struct {
	ID               string           `json:"id"`
	Name             *string          `json:"name,omitempty"`
	API              *string          `json:"api,omitempty"`
	BaseURL          *string          `json:"baseUrl,omitempty"`
	Reasoning        *bool            `json:"reasoning,omitempty"`
	ThinkingLevelMap *ThinkingLevelMap `json:"thinkingLevelMap,omitempty"`
	Input            []string         `json:"input,omitempty"` // ["text"] | ["text", "image"]
	Cost             *ModelCost       `json:"cost,omitempty"`
	ContextWindow    *int             `json:"contextWindow,omitempty"`
	MaxTokens        *int             `json:"maxTokens,omitempty"`
	Headers          map[string]string `json:"headers,omitempty"`
	Compat           *Compat          `json:"compat,omitempty"`
}

// ModelOverride defines per-model field overrides (all optional, merged with model).
type ModelOverride struct {
	Name             *string          `json:"name,omitempty"`
	Reasoning        *bool            `json:"reasoning,omitempty"`
	ThinkingLevelMap *ThinkingLevelMap `json:"thinkingLevelMap,omitempty"`
	Input            []string         `json:"input,omitempty"`
	Cost             *ModelCost       `json:"cost,omitempty"`
	ContextWindow    *int             `json:"contextWindow,omitempty"`
	MaxTokens        *int             `json:"maxTokens,omitempty"`
	Headers          map[string]string `json:"headers,omitempty"`
	Compat           *Compat          `json:"compat,omitempty"`
}

// ProviderConfig defines a provider with its models (matching pi's ProviderConfigSchema).
type ProviderConfig struct {
	Name             *string                  `json:"name,omitempty"`
	BaseURL          *string                  `json:"baseUrl,omitempty"`
	APIKey           *string                  `json:"apiKey,omitempty"`
	API              *string                  `json:"api,omitempty"`
	Headers          map[string]string        `json:"headers,omitempty"`
	Compat           *Compat                  `json:"compat,omitempty"`
	AuthHeader       *bool                    `json:"authHeader,omitempty"`
	Models           []ModelDefinition        `json:"models,omitempty"`
	ModelOverrides   map[string]ModelOverride `json:"modelOverrides,omitempty"`
}

// ModelsConfig is the top-level models.json structure.
type ModelsConfig struct {
	Providers map[string]ProviderConfig `json:"providers"`
}

// ResolvedModel is the flattened, fully-resolved model after merging provider defaults,
// model definition, and overrides (matching pi's Model<Api>).
type ResolvedModel struct {
	ID               string           `json:"id"`
	Name             string           `json:"name"`
	Provider         string           `json:"provider"`
	API              string           `json:"api"`
	BaseURL          string           `json:"baseUrl"`
	APIKey           string           `json:"apiKey,omitempty"`
	Reasoning        bool             `json:"reasoning"`
	ThinkingLevelMap *ThinkingLevelMap `json:"thinkingLevelMap,omitempty"`
	Input            []string         `json:"input"` // ["text"] or ["text", "image"]
	Cost             ModelCost        `json:"cost"`
	ContextWindow    int              `json:"contextWindow"`
	MaxTokens        int              `json:"maxTokens"`
	Headers          map[string]string `json:"headers,omitempty"`
	Compat           *Compat          `json:"compat,omitempty"`
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

// Registry holds resolved models and the raw config for mutation.
type Registry struct {
	Models  []ResolvedModel
	config  *ModelsConfig
	rawPath string // file path this was loaded from
}

// DefaultPath returns ~/.xihu/models.json.
func DefaultPath() string {
	home, err := os.UserHomeDir()
	if err != nil {
		home = os.TempDir()
	}
	return filepath.Join(home, ".xihu", "models.json")
}

// XihuDefaultPath returns ~/.xihu/models.json (xihu-specific).
func XihuDefaultPath() string {
	home, err := os.UserHomeDir()
	if err != nil {
		home = os.TempDir()
	}
	return filepath.Join(home, ".xihu", "models.json")
}

// LoadRegistry reads a models.json file. Supports both pi format (providers)
// and legacy xihu format (models). Legacy format is auto-migrated.
func LoadRegistry(path string) (*Registry, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		return nil, fmt.Errorf("read models file %s: %w", path, err)
	}

	// Try pi format first
	var cfg ModelsConfig
	if err := json.Unmarshal(data, &cfg); err == nil && cfg.Providers != nil {
		r := &Registry{config: &cfg, rawPath: path}
		r.Models = resolveModels(&cfg)
		return r, nil
	}

	// Try legacy xihu format
	var legacy struct {
		Models map[string]LegacyModelInfo `json:"models"`
	}
	if err := json.Unmarshal(data, &legacy); err == nil && legacy.Models != nil {
		// Migrate to pi format
		cfg := migrateLegacyModels(legacy.Models)
		r := &Registry{config: cfg, rawPath: path}
		r.Models = resolveModels(cfg)
		// Save in new format
		if saveErr := r.Save(); saveErr != nil {
			return nil, fmt.Errorf("migrate models file %s: %w", path, saveErr)
		}
		return r, nil
	}

	return nil, fmt.Errorf("models file %s: unrecognized format", path)
}

// LegacyModelInfo is the old xihu model format for migration.
type LegacyModelInfo struct {
	ID               string  `json:"id"`
	Name             string  `json:"name"`
	Provider         string  `json:"provider"`
	API              string  `json:"api"`
	BaseURL          string  `json:"base_url"`
	MaxTokens        int     `json:"max_tokens"`
	SupportsVision   bool    `json:"supports_vision"`
	SupportsThinking bool    `json:"supports_thinking"`
	SupportsTools    bool    `json:"supports_tools"`
	Pricing          struct {
		Prompt     float64 `json:"prompt"`
		Completion float64 `json:"completion"`
	} `json:"pricing"`
}

// migrateLegacyModels converts old xihu format to pi's provider-centric format.
func migrateLegacyModels(legacy map[string]LegacyModelInfo) *ModelsConfig {
	cfg := &ModelsConfig{Providers: make(map[string]ProviderConfig)}

	for _, m := range legacy {
		provider := m.Provider
		if _, exists := cfg.Providers[provider]; !exists {
			cfg.Providers[provider] = ProviderConfig{
				BaseURL: &m.BaseURL,
				API:     &m.API,
				Models:  []ModelDefinition{},
			}
		}

		p := cfg.Providers[provider]
		input := []string{"text"}
		if m.SupportsVision {
			input = append(input, "image")
		}
		reasoning := m.SupportsThinking

		cost := &ModelCost{
			Input:  m.Pricing.Prompt,
			Output: m.Pricing.Completion,
		}

		def := ModelDefinition{
			ID:        m.ID,
			Name:      &m.Name,
			Reasoning: &reasoning,
			Input:     input,
			Cost:      cost,
		}
		if m.MaxTokens > 0 {
			mt := m.MaxTokens
			def.MaxTokens = &mt
		}
		if m.BaseURL != "" && p.BaseURL != nil && m.BaseURL != *p.BaseURL {
			url := m.BaseURL
			def.BaseURL = &url
		}

		p.Models = append(p.Models, def)
		cfg.Providers[provider] = p
	}

	return cfg
}

// ResolveModels is the exported version of resolveModels.
func ResolveModels(cfg *ModelsConfig) []ResolvedModel {
	return resolveModels(cfg)
}

// resolveModels flattens the provider-centric config into resolved models.
// Applies provider defaults and model overrides.
func resolveModels(cfg *ModelsConfig) []ResolvedModel {
	var models []ResolvedModel

	for providerName, provider := range cfg.Providers {
		for _, modelDef := range provider.Models {
			api := orStr(modelDef.API, provider.API, "openai-completions")
			baseURL := orStr(modelDef.BaseURL, provider.BaseURL, "")
			if baseURL == "" {
				continue
			}
			apiKey := ""
			if provider.APIKey != nil {
				apiKey = *provider.APIKey
			}

			// Apply model override if present
			override := provider.ModelOverrides[modelDef.ID]

			name := orStr(modelDef.Name, nil, modelDef.ID)
			if override.Name != nil {
				name = *override.Name
			}

			reasoning := false
			if modelDef.Reasoning != nil {
				reasoning = *modelDef.Reasoning
			}
			if override.Reasoning != nil {
				reasoning = *override.Reasoning
			}

			input := copyStrings(modelDef.Input)
			if input == nil {
				input = []string{"text"}
			}
			if override.Input != nil {
				input = copyStrings(override.Input)
			}

			cost := ModelCost{}
			if modelDef.Cost != nil {
				cost = *modelDef.Cost
			}
			if override.Cost != nil {
				mergeCost(&cost, override.Cost)
			}

			contextWindow := 128000 // pi default
			if modelDef.ContextWindow != nil {
				contextWindow = *modelDef.ContextWindow
			}
			if override.ContextWindow != nil {
				contextWindow = *override.ContextWindow
			}

			maxTokens := 16384 // pi default
			if modelDef.MaxTokens != nil {
				maxTokens = *modelDef.MaxTokens
			}
			if override.MaxTokens != nil {
				maxTokens = *override.MaxTokens
			}

			// Merge compat
			compat := mergeCompat(provider.Compat, modelDef.Compat)
			if override.Compat != nil {
				compat = mergeCompat(compat, override.Compat)
			}

			models = append(models, ResolvedModel{
				ID:               modelDef.ID,
				Name:             name,
				Provider:         providerName,
				API:              api,
				BaseURL:          baseURL,
				APIKey:           apiKey,
				Reasoning:        reasoning,
				ThinkingLevelMap: orThinkingLevelMap(modelDef.ThinkingLevelMap, override.ThinkingLevelMap),
				Input:            input,
				Cost:             cost,
				ContextWindow:    contextWindow,
				MaxTokens:        maxTokens,
				Headers:          modelDef.Headers,
				Compat:           compat,
			})
		}
	}

	return models
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

func orStr(a, b *string, fallback string) string {
	if a != nil {
		return *a
	}
	if b != nil {
		return *b
	}
	return fallback
}

func orThinkingLevelMap(a, b *ThinkingLevelMap) *ThinkingLevelMap {
	if b != nil {
		return b
	}
	return a
}

func copyStrings(s []string) []string {
	if s == nil {
		return nil
	}
	c := make([]string, len(s))
	copy(c, s)
	return c
}

func mergeCost(base, override *ModelCost) {
	if override.Input != 0 {
		base.Input = override.Input
	}
	if override.Output != 0 {
		base.Output = override.Output
	}
	if override.CacheRead != 0 {
		base.CacheRead = override.CacheRead
	}
	if override.CacheWrite != 0 {
		base.CacheWrite = override.CacheWrite
	}
}

func mergeCompat(base, override *Compat) *Compat {
	if base == nil && override == nil {
		return nil
	}
	if base == nil {
		return override
	}
	if override == nil {
		return base
	}
	// Shallow merge — for real usage, deep merge would be better
	result := *base
	if override.SupportsDeveloperRole != nil {
		result.SupportsDeveloperRole = override.SupportsDeveloperRole
	}
	if override.SupportsReasoningEffort != nil {
		result.SupportsReasoningEffort = override.SupportsReasoningEffort
	}
	if override.ThinkingFormat != nil {
		result.ThinkingFormat = override.ThinkingFormat
	}
	if override.SupportsStore != nil {
		result.SupportsStore = override.SupportsStore
	}
	if override.MaxTokensField != nil {
		result.MaxTokensField = override.MaxTokensField
	}
	return &result
}

// ---------------------------------------------------------------------------
// I/O
// ---------------------------------------------------------------------------

// Save persists the registry to its file path.
func (r *Registry) Save() error {
	if r == nil || r.config == nil {
		return fmt.Errorf("registry is nil")
	}
	dir := filepath.Dir(r.rawPath)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return fmt.Errorf("create models directory %s: %w", dir, err)
	}

	data, err := json.MarshalIndent(r.config, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal models: %w", err)
	}
	data = append(data, '\n')

	if err := os.WriteFile(r.rawPath, data, 0644); err != nil {
		return fmt.Errorf("write models file %s: %w", r.rawPath, err)
	}
	return nil
}

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

// Find looks up a model by "provider/id" or just "id".
func (r *Registry) Find(query string) (*ResolvedModel, error) {
	if r == nil {
		return nil, fmt.Errorf("registry is nil")
	}

	query = strings.TrimSpace(query)
	if query == "" {
		return nil, fmt.Errorf("empty query")
	}

	// Try provider/id format
	if strings.Contains(query, "/") {
		parts := strings.SplitN(query, "/", 2)
		provider, modelID := parts[0], parts[1]
		for i := range r.Models {
			m := &r.Models[i]
			if m.Provider == provider && m.ID == modelID {
				return m, nil
			}
		}
	}

	// Try exact ID match
	for i := range r.Models {
		m := &r.Models[i]
		if m.ID == query {
			return m, nil
		}
	}

	// Try case-insensitive
	lower := strings.ToLower(query)
	for i := range r.Models {
		m := &r.Models[i]
		if strings.EqualFold(m.ID, query) || strings.EqualFold(m.Provider+"/"+m.ID, query) {
			return m, nil
		}
	}

	// Try prefix match (longest wins)
	var best *ResolvedModel
	bestLen := 0
	for i := range r.Models {
		m := &r.Models[i]
		full := m.Provider + "/" + m.ID
		if strings.HasPrefix(strings.ToLower(full), lower) {
			if len(full) > bestLen {
				best = m
				bestLen = len(full)
			}
		}
		if strings.HasPrefix(strings.ToLower(m.ID), lower) {
			if len(m.ID) > bestLen {
				best = m
				bestLen = len(m.ID)
			}
		}
	}
	if best != nil {
		return best, nil
	}

	// Try substring
	for i := range r.Models {
		m := &r.Models[i]
		if strings.Contains(strings.ToLower(m.ID), lower) {
			return m, nil
		}
	}

	return nil, fmt.Errorf("model %q not found in registry", query)
}

// GetAll returns all resolved models.
func (r *Registry) GetAll() []ResolvedModel {
	if r == nil {
		return nil
	}
	return r.Models
}

// GetByProvider returns models for a specific provider.
func (r *Registry) GetByProvider(provider string) []ResolvedModel {
	var result []ResolvedModel
	for i := range r.Models {
		if r.Models[i].Provider == provider {
			result = append(result, r.Models[i])
		}
	}
	return result
}

// SupportsImage returns true if the model accepts image input.
func (m *ResolvedModel) SupportsImage() bool {
	for _, t := range m.Input {
		if t == "image" {
			return true
		}
	}
	return false
}

// SupportsText returns true if the model accepts text input.
func (m *ResolvedModel) SupportsText() bool {
	for _, t := range m.Input {
		if t == "text" {
			return true
		}
	}
	return false
}
