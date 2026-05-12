package modelregistry

import (
	"encoding/json"
	"log"
	"os"
	"path/filepath"

	"github.com/huichen/xihu/internal/models"
	"github.com/huichen/xihu/pkg/types"
)

// LoadUserModels reads a pi-compatible models.json file and converts
// the resolved models to the runtime types.Model format.
// Returns nil, nil if the file does not exist (non-error).
func LoadUserModels(path string) ([]types.Model, error) {
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}

	// Try pi format
	var cfg models.ModelsConfig
	if err := unmarshalJSON(data, &cfg); err != nil {
		return nil, err
	}

	if cfg.Providers == nil {
		return nil, nil
	}

	resolved := models.ResolveModels(&cfg)
	result := make([]types.Model, 0, len(resolved))
	for _, rm := range resolved {
		result = append(result, resolvedToTypesModel(rm))
	}
	return result, nil
}

// resolvedToTypesModel converts a models.ResolvedModel to types.Model.
func resolvedToTypesModel(rm models.ResolvedModel) types.Model {
	m := types.Model{
		ID:            rm.ID,
		Name:          rm.Name,
		Provider:      rm.Provider,
		API:           rm.API,
		BaseURL:       rm.BaseURL,
		ContextWindow: rm.ContextWindow,
		MaxTokens:     rm.MaxTokens,
		Reasoning:     rm.Reasoning,
		InputTypes:    rm.Input,
		Headers:       rm.Headers,
	}
	m.Cost.Input = rm.Cost.Input
	m.Cost.Output = rm.Cost.Output
	m.Cost.CacheRead = rm.Cost.CacheRead
	m.Cost.CacheWrite = rm.Cost.CacheWrite

	if rm.ThinkingLevelMap != nil {
		m.ThinkingLevelMap = thinkingLevelMapToInterface(rm.ThinkingLevelMap)
	}

	return m
}

// thinkingLevelMapToInterface converts a ThinkingLevelMap to map[string]interface{}.
func thinkingLevelMapToInterface(tlm *models.ThinkingLevelMap) map[string]interface{} {
	if tlm == nil {
		return nil
	}
	result := make(map[string]interface{})
	if tlm.Off != nil {
		result["off"] = *tlm.Off
	}
	if tlm.Minimal != nil {
		result["minimal"] = *tlm.Minimal
	}
	if tlm.Low != nil {
		result["low"] = *tlm.Low
	}
	if tlm.Medium != nil {
		result["medium"] = *tlm.Medium
	}
	if tlm.High != nil {
		result["high"] = *tlm.High
	}
	if tlm.XHigh != nil {
		result["xhigh"] = *tlm.XHigh
	}
	return result
}

// unmarshalJSON is a wrapper to avoid import cycle.
func unmarshalJSON(data []byte, v interface{}) error {
	return json.Unmarshal(data, v)
}

// InitBuiltinModels initializes the models catalog from the built-in pi-compatible models.
// Called during startup to populate the runtime catalog.
func InitBuiltinModels() []types.Model {
	cfg := models.BuiltinConfig()
	resolved := models.ResolveModels(cfg)
	result := make([]types.Model, 0, len(resolved))
	for _, rm := range resolved {
		result = append(result, resolvedToTypesModel(rm))
	}
	return result
}

// UserModelsPath returns ~/.xihu/models.json.
func UserModelsPath() string {
	home, err := os.UserHomeDir()
	if err != nil {
		home = os.TempDir()
	}
	return filepath.Join(home, ".xihu", "models.json")
}

// SharedModelsPath returns ~/.pi/agent/models.json (shared with pi).
func SharedModelsPath() string {
	home, err := os.UserHomeDir()
	if err != nil {
		home = os.TempDir()
	}
	return filepath.Join(home, ".pi", "agent", "models.json")
}

// LoadUserModelsAuto tries ~/.xihu/models.json first, then ~/.pi/agent/models.json.
// This allows xihu to share models with pi.
func LoadUserModelsAuto() []types.Model {
	userPath := UserModelsPath()
	userModels, err := LoadUserModels(userPath)
	if err == nil && len(userModels) > 0 {
		log.Printf("[modelregistry] loaded %d user model(s) from %s", len(userModels), userPath)
		return userModels
	}

	sharedPath := SharedModelsPath()
	sharedModels, err := LoadUserModels(sharedPath)
	if err == nil && len(sharedModels) > 0 {
		log.Printf("[modelregistry] loaded %d model(s) from shared pi config %s", len(sharedModels), sharedPath)
		return sharedModels
	}

	return nil
}
