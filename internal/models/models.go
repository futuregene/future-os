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
