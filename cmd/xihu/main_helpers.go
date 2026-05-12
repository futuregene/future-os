package main

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sort"
	"strings"

	authpkg "github.com/huichen/xihu/internal/auth"
	"github.com/huichen/xihu/internal/modelregistry"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/pkg/types"
)
// listModels prints available models (with optional fuzzy search).
// Mirrors pi's listModels: uses getAvailable() (auth-filtered), tabular format,
// sorted by provider then model ID, with human-readable token counts.
func listModels(query string) {
	mreg := modelregistry.New()

	// Load auth store for auth filtering
	authStore, err := authpkg.LoadAuth()
	if err != nil {
		fmt.Fprintf(os.Stderr, "Warning: failed to load auth: %v\n", err)
		authStore = nil
	}

	// Load models.json apiKey per provider (pi reads auth from both auth.json and models.json)
	modelsAPIKeys := loadModelsAPIKeys()

	// Determine which providers have auth configured — mirrors pi's hasConfiguredAuth:
	// 1. auth.json by provider
	// 2. models.json provider apiKey field
	// 3. environment variables
	hasAuth := func(provider string) bool {
		if authStore != nil && authStore.Get(provider) != "" {
			return true
		}
		if modelsAPIKeys[provider] {
			return true
		}
		upper := strings.ToUpper(strings.ReplaceAll(provider, "-", "_"))
		if os.Getenv(upper+"_API_KEY") != "" {
			return true
		}
		if os.Getenv("LLM_API_KEY") != "" {
			return true
		}
		return false
	}

	// Use GetAvailable (auth-filtered) like pi
	all := mreg.GetAvailable(hasAuth)

	if len(all) == 0 {
		fmt.Println("No models with configured auth. Add API keys to ~/.xihu/auth.json, ~/.xihu/models.json, or set environment variables.")
		return
	}

	// Apply fuzzy filter if query provided
	if query != "true" {
		all = fuzzyFilterModels(all, query)
	}

	if len(all) == 0 {
		fmt.Printf("No models matching %q\n", query)
		return
	}

	// Sort by provider, then by model ID (same as pi)
	sort.Slice(all, func(i, j int) bool {
		if all[i].Provider != all[j].Provider {
			return all[i].Provider < all[j].Provider
		}
		return all[i].ID < all[j].ID
	})

	// Build rows for column width calculation
	type row struct {
		provider, model, context, maxOut, thinking, images string
	}
	rows := make([]row, len(all))
	for i, m := range all {
		rows[i] = row{
			provider: m.Provider,
			model:    m.ID,
			context:  formatTokenCount(m.ContextWindow),
			maxOut:   formatTokenCount(m.MaxTokens),
			thinking: func() string { if m.Reasoning { return "yes" }; return "no" }(),
			images:   func() string { for _, t := range m.InputTypes { if t == "image" { return "yes" } }; return "no" }(),
		}
	}

	// Calculate column widths
	headers := row{provider: "provider", model: "model", context: "context", maxOut: "max-out", thinking: "thinking", images: "images"}
	wp := len(headers.provider)
	wm := len(headers.model)
	wc := len(headers.context)
	wmo := len(headers.maxOut)
	wt := len(headers.thinking)
	wi := len(headers.images)
	for _, r := range rows {
		if len(r.provider) > wp {
			wp = len(r.provider)
		}
		if len(r.model) > wm {
			wm = len(r.model)
		}
		if len(r.context) > wc {
			wc = len(r.context)
		}
		if len(r.maxOut) > wmo {
			wmo = len(r.maxOut)
		}
		if len(r.thinking) > wt {
			wt = len(r.thinking)
		}
		if len(r.images) > wi {
			wi = len(r.images)
		}
	}

	// Print header
	fmt.Printf("%-*s  %-*s  %-*s  %-*s  %-*s  %-*s\n",
		wp, headers.provider,
		wm, headers.model,
		wc, headers.context,
		wmo, headers.maxOut,
		wt, headers.thinking,
		wi, headers.images)

	// Print rows
	for _, r := range rows {
		fmt.Printf("%-*s  %-*s  %-*s  %-*s  %-*s  %-*s\n",
			wp, r.provider,
			wm, r.model,
			wc, r.context,
			wmo, r.maxOut,
			wt, r.thinking,
			wi, r.images)
	}
}

// formatTokenCount formats a number as human-readable (e.g., 200000 -> "200K", 1000000 -> "1M").
// Matches pi's formatTokenCount exactly.
func formatTokenCount(count int) string {
	if count <= 0 {
		return "0"
	}
	if count >= 1_000_000 {
		millions := float64(count) / 1_000_000
		if millions == float64(int(millions)) {
			return fmt.Sprintf("%dM", int(millions))
		}
		return fmt.Sprintf("%.1fM", millions)
	}
	if count >= 1_000 {
		thousands := float64(count) / 1_000
		if thousands == float64(int(thousands)) {
			return fmt.Sprintf("%dK", int(thousands))
		}
		return fmt.Sprintf("%.1fK", thousands)
	}
	return fmt.Sprintf("%d", count)
}

// fuzzyFilterModels filters models by matching the query against provider+model ID.
func fuzzyFilterModels(models []types.Model, query string) []types.Model {
	q := strings.ToLower(query)
	var filtered []types.Model
	for _, m := range models {
		s := strings.ToLower(m.Provider + " " + m.ID)
		if strings.Contains(s, q) {
			filtered = append(filtered, m)
		}
	}
	return filtered
}

// loadModelsAPIKeys reads ~/.xihu/models.json and returns a set of provider names
// that have an apiKey configured. Mirrors pi's storeProviderRequestConfig behavior.
func loadModelsAPIKeys() map[string]bool {
	result := make(map[string]bool)
	home, err := os.UserHomeDir()
	if err != nil {
		return result
	}
	data, err := os.ReadFile(home + "/.xihu/models.json")
	if err != nil {
		return result
	}
	var cfg struct {
		Providers map[string]json.RawMessage `json:"providers"`
	}
	if err := json.Unmarshal(data, &cfg); err != nil {
		return result
	}
	for name, raw := range cfg.Providers {
		var pc struct {
			APIKey string `json:"apiKey"`
		}
		if json.Unmarshal(raw, &pc) == nil && pc.APIKey != "" {
			result[name] = true
		}
	}
	return result
}

// parseModelString parses a model string like "provider/model:thinking" into components.
func parseModelString(model, defaultModel, defaultProvider, defaultThinking string) (resolvedModel, resolvedProvider, resolvedThinking string) {
	if model == "" {
		return "", defaultProvider, ""
	}

	// Check for thinking suffix (model:high)
	if idx := strings.LastIndex(model, ":"); idx >= 0 {
		suffix := model[idx+1:]
		validLevels := map[string]bool{
			"off": true, "minimal": true, "low": true,
			"medium": true, "high": true, "xhigh": true,
		}
		if validLevels[suffix] {
			resolvedThinking = suffix
			model = model[:idx]
		}
	}

	// Check for provider prefix
	if idx := strings.Index(model, "/"); idx >= 0 {
		resolvedProvider = model[:idx]
		resolvedModel = model[idx+1:]
	} else {
		resolvedModel = model
		resolvedProvider = defaultProvider
	}

	return resolvedModel, resolvedProvider, resolvedThinking
}

// firstNonEmpty returns the first non-empty string from the arguments.
func firstNonEmpty(strs ...string) string {
	for _, s := range strs {
		if s != "" {
			return s
		}
	}
	return ""
}

// providerBaseURL returns the default base URL for a provider.
func providerBaseURL(provider, explicitProvider string) string {
	// Only return a URL if the provider was explicitly set
	if explicitProvider == "" {
		return ""
	}
	switch provider {
	case "openai":
		return "https://api.openai.com/v1"
	case "anthropic":
		return "https://api.anthropic.com/v1"
	case "deepseek":
		return "https://api.deepseek.com/v1"
	case "google":
		return "https://generativelanguage.googleapis.com/v1beta/openai"
	case "qwen", "alibaba":
		return "https://dashscope-intl.aliyuncs.com/compatible-mode/v1"
	case "xai":
		return "https://api.x.ai/v1"
	case "mistral":
		return "https://api.mistral.ai/v1"
	default:
		return ""
	}
}

// defaultModelForURL returns a default model ID for a known base URL.
func defaultModelForURL(baseURL string) string {
	switch {
	case strings.Contains(baseURL, "openai.com"):
		return "gpt-4o"
	case strings.Contains(baseURL, "anthropic.com"):
		return "claude-sonnet-4"
	case strings.Contains(baseURL, "deepseek.com"):
		return "deepseek-chat"
	case strings.Contains(baseURL, "googleapis.com"):
		return "gemini-2.5-flash"
	case strings.Contains(baseURL, "dashscope"):
		return "qwen3.6-plus"
	case strings.Contains(baseURL, "x.ai"):
		return "grok-3"
	default:
		return ""
	}
}

// resolveNoTools determines the no-tools mode from args.
func resolveNoTools(args *Args) string {
	if args.NoBuiltinTools {
		return "builtin"
	}
	return args.NoTools
}

// isTerminal checks if stdin is a terminal.
func isTerminal() bool {
	// Simple check: if stdout is not a terminal, we're not interactive
	return isatty(1)
}

// isatty checks if fd is a terminal.
func isatty(fd int) bool {
	stat, err := os.Stdin.Stat()
	if err != nil {
		return false
	}
	return (stat.Mode() & os.ModeCharDevice) != 0
}

// exportSession exports a session to JSON (pi-mono exports to HTML with full syntax highlighting).
func exportSession(sess *session.Session, path string) {
	data, err := json.MarshalIndent(sess, "", "  ")
	if err != nil {
		fmt.Fprintf(os.Stderr, "Error marshaling session: %v\n", err)
		os.Exit(1)
	}
	if err := os.WriteFile(path, data, 0644); err != nil {
		fmt.Fprintf(os.Stderr, "Error writing export file: %v\n", err)
		os.Exit(1)
	}
	fmt.Printf("Session exported to %s\n", path)
}

// saveSession saves a session to disk.
func saveSession(mgr *session.Manager, sess *session.Session, messages []types.Message, model, baseURL string) {
	sess.Model = model
	sess.BaseURL = baseURL
	entries := session.MessagesToEntries(messages, sess.ID)
	sess.Entries = append(sess.Entries, entries...)
	if err := mgr.Save(sess); err != nil {
		fmt.Fprintf(os.Stderr, "Warning: failed to save session: %v\n", err)
	}
}

// matchModelPattern checks if a model ID matches a pattern (exact or glob).
// Pattern can be a full "provider/id" or just "id". Supports *, ?, [ globs.
func matchModelPattern(pat, fullID, modelID string) bool {
	// Check exact matches first
	if pat == fullID || pat == modelID {
		return true
	}
	// Check glob against full ID and model ID
	if matched, _ := filepath.Match(pat, fullID); matched {
		return true
	}
	if matched, _ := filepath.Match(pat, modelID); matched {
		return true
	}
	return false
}
func processSentinel(result string, eng interface{}, mgr *session.Manager, sess *session.Session, ctx interface{}) bool {
	// Check for sentinel markers in command output
	if strings.Contains(result, "__XIHU_SENTINEL__") {
		// Handle special commands
		return true
	}
	return false
}
