// Package models provides a model registry with built-in model metadata
// and JSON persistence. Models are loaded from ~/.pi/agent/models.json on
// startup. If the file doesn't exist, the built-in set is saved automatically
// so users can customize it.
package models

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"strings"
)

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

// Pricing represents cost per 1K tokens for a model.
type Pricing struct {
	Prompt      float64 `json:"prompt"`
	Completion  float64 `json:"completion"`
}

// ModelInfo describes a known LLM model with its capabilities and pricing.
// It extends the simpler types.Model in pkg/types with richer metadata.
type ModelInfo struct {
	ID               string  `json:"id"`
	Name             string  `json:"name"`
	Provider         string  `json:"provider"`
	API              string  `json:"api"`               // "openai-completions" or "anthropic-messages"
	BaseURL          string  `json:"base_url"`
	MaxTokens        int     `json:"max_tokens"`
	SupportsVision   bool    `json:"supports_vision"`
	SupportsThinking bool    `json:"supports_thinking"`
	SupportsTools    bool    `json:"supports_tools"`
	Pricing          Pricing `json:"pricing"`
}

// Registry holds a map of model ID → ModelInfo and tracks the file path.
type Registry struct {
	Models map[string]ModelInfo `json:"models"`
	path   string               // file path this registry was loaded from / saved to
}

// ---------------------------------------------------------------------------
// Constructors / I/O
// ---------------------------------------------------------------------------

// LoadRegistry reads a model registry from a JSON file. If the file does not
// exist, it creates one from BuiltinModels and saves it to path. Returns a
// populated Registry on success.
func LoadRegistry(path string) (*Registry, error) {
	r := &Registry{path: path}

	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			// Bootstrap from builtins
			r.Models = builtinMap()
			if saveErr := r.save(); saveErr != nil {
				return nil, fmt.Errorf("bootstrap models file %s: %w", path, saveErr)
			}
			return r, nil
		}
		return nil, fmt.Errorf("read models file %s: %w", path, err)
	}

	var raw struct {
		Models map[string]ModelInfo `json:"models"`
	}
	if err := json.Unmarshal(data, &raw); err != nil {
		return nil, fmt.Errorf("parse models file %s: %w", path, err)
	}
	r.Models = raw.Models
	if r.Models == nil {
		r.Models = builtinMap()
	}
	return r, nil
}

// SaveRegistry persists the registry to its file path (JSON, indented).
func (r *Registry) SaveRegistry() error {
	if r == nil {
		return fmt.Errorf("registry is nil")
	}
	return r.save()
}

// save is the internal implementation that handles path/dir creation.
func (r *Registry) save() error {
	dir := filepath.Dir(r.path)
	if err := os.MkdirAll(dir, 0755); err != nil {
		return fmt.Errorf("create models directory %s: %w", dir, err)
	}

	raw := struct {
		Models map[string]ModelInfo `json:"models"`
	}{Models: r.Models}

	data, err := json.MarshalIndent(raw, "", "  ")
	if err != nil {
		return fmt.Errorf("marshal models: %w", err)
	}
	data = append(data, '\n')

	if err := os.WriteFile(r.path, data, 0644); err != nil {
		return fmt.Errorf("write models file %s: %w", r.path, err)
	}
	return nil
}

// DefaultRegistryPath returns ~/.pi/agent/models.json.
func DefaultRegistryPath() string {
	home, err := os.UserHomeDir()
	if err != nil {
		home = os.TempDir()
	}
	return filepath.Join(home, ".pi", "agent", "models.json")
}

// ---------------------------------------------------------------------------
// Built-in models
// ---------------------------------------------------------------------------

// BuiltinModels returns a hardcoded list of 20+ common models.
// This serves as both the default set and documentation for users.
func BuiltinModels() []ModelInfo {
	return []ModelInfo{
		// --- OpenAI ---
		{
			ID: "gpt-4o", Name: "GPT-4o", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 128000,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 2.50, Completion: 10.00},
		},
		{
			ID: "gpt-4o-mini", Name: "GPT-4o Mini", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 128000,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.15, Completion: 0.60},
		},
		{
			ID: "gpt-4.1", Name: "GPT-4.1", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 1000000,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 2.00, Completion: 8.00},
		},
		{
			ID: "gpt-4.1-mini", Name: "GPT-4.1 Mini", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 1000000,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.40, Completion: 1.60},
		},
		{
			ID: "gpt-4.1-nano", Name: "GPT-4.1 Nano", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 1000000,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.10, Completion: 0.40},
		},
		{
			ID: "o1", Name: "O1", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 200000,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 15.00, Completion: 60.00},
		},
		{
			ID: "o3-mini", Name: "O3 Mini", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 200000,
			SupportsVision: false, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 1.10, Completion: 4.40},
		},
		{
			ID: "o4-mini", Name: "O4 Mini", Provider: "openai", API: "openai-completions",
			BaseURL: "https://api.openai.com/v1", MaxTokens: 200000,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 1.10, Completion: 4.40},
		},

		// --- Anthropic ---
		{
			ID: "claude-sonnet-4", Name: "Claude Sonnet 4", Provider: "anthropic", API: "anthropic-messages",
			BaseURL: "https://api.anthropic.com/v1", MaxTokens: 200000,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 3.00, Completion: 15.00},
		},
		{
			ID: "claude-opus-4", Name: "Claude Opus 4", Provider: "anthropic", API: "anthropic-messages",
			BaseURL: "https://api.anthropic.com/v1", MaxTokens: 200000,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 15.00, Completion: 75.00},
		},
		{
			ID: "claude-3.5-haiku", Name: "Claude Haiku 3.5", Provider: "anthropic", API: "anthropic-messages",
			BaseURL: "https://api.anthropic.com/v1", MaxTokens: 200000,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.80, Completion: 4.00},
		},

		// --- DeepSeek ---
		{
			ID: "deepseek-chat", Name: "DeepSeek Chat", Provider: "deepseek", API: "openai-completions",
			BaseURL: "https://api.deepseek.com/v1", MaxTokens: 128000,
			SupportsVision: false, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.27, Completion: 1.10},
		},
		{
			ID: "deepseek-reasoner", Name: "DeepSeek Reasoner", Provider: "deepseek", API: "openai-completions",
			BaseURL: "https://api.deepseek.com/v1", MaxTokens: 128000,
			SupportsVision: false, SupportsThinking: true, SupportsTools: false,
			Pricing: Pricing{Prompt: 0.55, Completion: 2.19},
		},

		// --- Qwen (Alibaba) ---
		{
			ID: "qwen3.6-plus", Name: "Qwen 3.6 Plus", Provider: "qwen", API: "openai-completions",
			BaseURL: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", MaxTokens: 131072,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.80, Completion: 3.20},
		},
		{
			ID: "qwen3.6-max", Name: "Qwen 3.6 Max", Provider: "qwen", API: "openai-completions",
			BaseURL: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", MaxTokens: 131072,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 4.00, Completion: 16.00},
		},
		{
			ID: "qwen-coder-plus", Name: "Qwen Coder Plus", Provider: "qwen", API: "openai-completions",
			BaseURL: "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", MaxTokens: 131072,
			SupportsVision: false, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 1.50, Completion: 6.00},
		},

		// --- Google Gemini ---
		{
			ID: "gemini-2.5-flash", Name: "Gemini 2.5 Flash", Provider: "google", API: "openai-completions",
			BaseURL: "https://generativelanguage.googleapis.com/v1beta/openai", MaxTokens: 1048576,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.15, Completion: 0.60},
		},
		{
			ID: "gemini-2.5-pro", Name: "Gemini 2.5 Pro", Provider: "google", API: "openai-completions",
			BaseURL: "https://generativelanguage.googleapis.com/v1beta/openai", MaxTokens: 1048576,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 1.25, Completion: 10.00},
		},

		// --- xAI Grok ---
		{
			ID: "grok-3", Name: "Grok 3", Provider: "xai", API: "openai-completions",
			BaseURL: "https://api.x.ai/v1", MaxTokens: 131072,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 3.00, Completion: 15.00},
		},
		{
			ID: "grok-3-mini", Name: "Grok 3 Mini", Provider: "xai", API: "openai-completions",
			BaseURL: "https://api.x.ai/v1", MaxTokens: 131072,
			SupportsVision: true, SupportsThinking: true, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.30, Completion: 0.50},
		},

		// --- Meta Llama (via OpenRouter-style base URL) ---
		{
			ID: "llama-3.3-70b", Name: "Llama 3.3 70B", Provider: "meta", API: "openai-completions",
			BaseURL: "https://openrouter.ai/api/v1", MaxTokens: 131072,
			SupportsVision: false, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.12, Completion: 0.30},
		},
		{
			ID: "llama-4-maverick", Name: "Llama 4 Maverick", Provider: "meta", API: "openai-completions",
			BaseURL: "https://openrouter.ai/api/v1", MaxTokens: 131072,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.20, Completion: 0.80},
		},

		// --- Mistral ---
		{
			ID: "mistral-large", Name: "Mistral Large", Provider: "mistral", API: "openai-completions",
			BaseURL: "https://api.mistral.ai/v1", MaxTokens: 128000,
			SupportsVision: true, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 2.00, Completion: 6.00},
		},
		{
			ID: "codestral", Name: "Codestral", Provider: "mistral", API: "openai-completions",
			BaseURL: "https://api.mistral.ai/v1", MaxTokens: 256000,
			SupportsVision: false, SupportsThinking: false, SupportsTools: true,
			Pricing: Pricing{Prompt: 0.30, Completion: 0.90},
		},
	}
}

// builtinMap returns a map from model ID to ModelInfo for quick lookup.
func builtinMap() map[string]ModelInfo {
	builtins := BuiltinModels()
	m := make(map[string]ModelInfo, len(builtins))
	for _, mi := range builtins {
		m[mi.ID] = mi
	}
	return m
}

// ---------------------------------------------------------------------------
// Lookup
// ---------------------------------------------------------------------------

// ResolveModel looks up a model by name with fuzzy matching. It tries, in
// order: exact ID match, exact lowercase match, prefix match (e.g. "gpt-4"
// matches "gpt-4o"), substring match, and a simple edit-distance tiebreak.
// Returns the best match and an error if nothing is found.
func (r *Registry) ResolveModel(name string) (ModelInfo, error) {
	if r == nil || r.Models == nil {
		return ModelInfo{}, fmt.Errorf("registry is empty")
	}

	name = strings.TrimSpace(name)
	if name == "" {
		return ModelInfo{}, fmt.Errorf("empty model name")
	}

	// 1. Exact match
	if mi, ok := r.Models[name]; ok {
		return mi, nil
	}

	lower := strings.ToLower(name)

	// 2. Exact case-insensitive
	for id, mi := range r.Models {
		if strings.EqualFold(id, name) {
			return mi, nil
		}
	}

	// 3. Prefix match (longest prefix wins)
	var bestPrefix ModelInfo
	bestPrefixLen := 0
	for id, mi := range r.Models {
		if strings.HasPrefix(strings.ToLower(id), lower) {
			if len(id) > bestPrefixLen {
				bestPrefix = mi
				bestPrefixLen = len(id)
			}
		}
	}
	if bestPrefixLen > 0 {
		return bestPrefix, nil
	}

	// 4. Substring match
	for id, mi := range r.Models {
		if strings.Contains(strings.ToLower(id), lower) {
			return mi, nil
		}
	}

	// 5. Fuzzy: simple edit-distance tiebreak (case-insensitive)
	bestDist := -1
	var bestMatch ModelInfo
	for id, mi := range r.Models {
		d := levenshteinDistance(lower, strings.ToLower(id))
		// Allow match if distance is ≤ half the query length (with a floor of 2)
		threshold := len(lower) / 2
		if threshold < 2 {
			threshold = 2
		}
		if d <= threshold {
			if bestDist < 0 || d < bestDist {
				bestDist = d
				bestMatch = mi
			}
		}
	}
	if bestDist >= 0 {
		return bestMatch, nil
	}

	return ModelInfo{}, fmt.Errorf("model %q not found in registry", name)
}

// levenshteinDistance computes the Levenshtein edit distance between two strings.
func levenshteinDistance(a, b string) int {
	ar, br := []rune(a), []rune(b)
	n, m := len(ar), len(br)
	if n == 0 {
		return m
	}
	if m == 0 {
		return n
	}

	// Use two rows for O(min(n,m)) space
	prev := make([]int, m+1)
	cur := make([]int, m+1)
	for j := 0; j <= m; j++ {
		prev[j] = j
	}

	for i := 1; i <= n; i++ {
		cur[0] = i
		for j := 1; j <= m; j++ {
			cost := 1
			if ar[i-1] == br[j-1] {
				cost = 0
			}
			cur[j] = min3(
				prev[j]+1,       // deletion
				cur[j-1]+1,      // insertion
				prev[j-1]+cost,  // substitution
			)
		}
		prev, cur = cur, prev
	}
	return prev[m]
}

func min3(a, b, c int) int {
	if a < b {
		if a < c {
			return a
		}
		return c
	}
	if b < c {
		return b
	}
	return c
}
