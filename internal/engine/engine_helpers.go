package engine

import (
	"fmt"
	"os"
	"strings"

	"github.com/huichen/xihu/internal/apiregistry"
	"github.com/huichen/xihu/internal/llm"
	"github.com/huichen/xihu/pkg/types"
)

// ---------------------------------------------------------------------------
// Provider auto-detection
// ---------------------------------------------------------------------------

// providerFromURL heuristically extracts a provider name from a base URL.
// Delegates to apiregistry.LookupProviderFromURL.
func providerFromURL(baseURL string) string {
	return apiregistry.LookupProviderFromURL(baseURL)
}

// ---------------------------------------------------------------------------
// Thinking level → budget mapping
// ---------------------------------------------------------------------------

// thinkingLevelToBudget converts a named thinking level to a token budget.
// Returns 0 for "off" or empty string (meaning "not set / use default").
func thinkingLevelToBudget(level string) int {
	switch level {
	case "off":
		return 0
	case "minimal":
		return 1024
	case "low":
		return 4000
	case "medium":
		return 8000
	case "high":
		return 16000
	case "xhigh":
		return 24000
	case "max":
		return 32000
	default:
		return 0
	}
}

// ---------------------------------------------------------------------------
// Thinking context propagation: model metadata → llm.Client
// ---------------------------------------------------------------------------

// applyThinkingContext populates the provider with thinking-related fields
// from a resolved model's metadata. Handles both *llm.Client and *apiregistry.LazyProvider.
// Aligns with pi's model-aware thinking config.
func applyThinkingContext(provider types.LLMProvider, modelInfo types.Model, level string) {
	if provider == nil {
		return
	}

	// Extract compat fields from the generic interface{} (map after JSON unmarshal)
	var format string
	var sre, rra bool
	if cm, ok := modelInfo.Compat.(map[string]interface{}); ok {
		if tf, ok := cm["thinkingFormat"].(string); ok {
			format = tf
		}
		if v, ok := cm["supportsReasoningEffort"].(bool); ok {
			sre = v
		}
		if v, ok := cm["requiresReasoningContentOnAssistantMessages"].(bool); ok {
			rra = v
		}
	}

	switch p := provider.(type) {
	case *llm.Client:
		p.ThinkingLevel = level
		p.ThinkingLevelMap = modelInfo.ThinkingLevelMap
		p.CompatThinkingFormat = format
		p.CompatSupportsReasoningEffort = sre
		p.CompatRequiresReasoningOnAssistant = rra
	case *apiregistry.LazyProvider:
		p.SetThinkingContext(level, modelInfo.ThinkingLevelMap, format, sre, rra)
	}
}

// fixThinkingLevel clamps the thinking level to what the model actually supports,
// using the model's ThinkingLevelMap. Mirrors pi's clampThinkingLevel().
// Returns the clamped level and whether clamping occurred.
func fixThinkingLevel(modelInfo types.Model, level string) (string, bool) {
	if modelInfo.ThinkingLevelMap == nil {
		return level, false
	}
	if level == "" {
		return "off", true
	}

	// Determine supported levels from ThinkingLevelMap
	allLevels := []string{"off", "minimal", "low", "medium", "high", "xhigh"}
	var supported []string
	for _, l := range allLevels {
		mapped, ok := modelInfo.ThinkingLevelMap[l]
		if !ok {
			// Level not in map → assume supported
			supported = append(supported, l)
			continue
		}
		if mapped == nil {
			// null means NOT supported
			continue
		}
		if l == "xhigh" {
			// xhigh only supported if explicitly mapped
			supported = append(supported, l)
		} else {
			supported = append(supported, l)
		}
	}

	// If the requested level is in supported, return it
	for _, s := range supported {
		if s == level {
			return level, false
		}
	}

	// Find nearest higher supported level
	reqIdx := -1
	for i, l := range allLevels {
		if l == level {
			reqIdx = i
			break
		}
	}
	if reqIdx == -1 {
		if len(supported) > 0 {
			return supported[0], true
		}
		return "off", true
	}

	// Try higher first
	for i := reqIdx; i < len(allLevels); i++ {
		for _, s := range supported {
			if s == allLevels[i] {
				return s, true
			}
		}
	}
	// Then lower
	for i := reqIdx - 1; i >= 0; i-- {
		for _, s := range supported {
			if s == allLevels[i] {
				return s, true
			}
		}
	}
	if len(supported) > 0 {
		return supported[0], true
	}
	return "off", true
}

// hasAuthEnv checks if any known API key env var is set.
func hasAuthEnv() bool {
	return os.Getenv("LLM_API_KEY") != "" || os.Getenv("ANTHROPIC_API_KEY") != "" || os.Getenv("OPENAI_API_KEY") != ""
}

// defaultModelForProvider returns a sensible default model for the provider.
func defaultModelForProvider(baseURL string) string {
	if strings.Contains(baseURL, "anthropic.com") {
		return "claude-sonnet-4-20250514"
	}
	return "gpt-4o"
}

// RestoreEngine creates an Engine from an existing session.
func RestoreEngine(opts EngineOptions, sessionID string) (*Engine, error) {
	eng, err := NewEngine(opts)
	if err != nil {
		return nil, err
	}
	sess, err := eng.SessionManager.Load(sessionID, opts.CWD)
	if err != nil {
		return nil, fmt.Errorf("restore session %s: %w", sessionID, err)
	}
	eng.Session = sess
	if sess.Model != "" && opts.Model == "" {
		eng.Model = sess.Model
	}
	return eng, nil
}

// ModelFallbackMessage returns a message if the configured model differs from the saved one.
func ModelFallbackMessage(savedModel, currentModel string) string {
	if savedModel != "" && currentModel != "" && savedModel != currentModel {
		return fmt.Sprintf("Note: session was created with model %s, but using %s instead.", savedModel, currentModel)
	}
	return ""
}
