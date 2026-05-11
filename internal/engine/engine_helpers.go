package engine

import (
	"fmt"
	"os"
	"strings"

	"github.com/huichen/xihu/internal/apiregistry"
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

// clampThinkingLevel ensures the thinking level is compatible with the model.
func clampThinkingLevel(model, level string) string {
	if level == "" || level == "off" {
		return level
	}
	// o-series models support thinking; other models may not
	lowerModel := strings.ToLower(model)
	if strings.HasPrefix(lowerModel, "o1") || strings.HasPrefix(lowerModel, "o3") || strings.HasPrefix(lowerModel, "o4") {
		return level
	}
	// Claude models support thinking
	if strings.Contains(lowerModel, "claude") {
		return level
	}
	// Other models: disable thinking by default
	if level == "low" || level == "medium" {
		return level
	}
	return "low" // clamp to minimal for non-thinking models
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
