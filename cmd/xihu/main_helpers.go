package main

import (
	"encoding/json"
	"fmt"
	"os"
	"strings"

	"github.com/huichen/xihu/internal/modelregistry"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/pkg/types"
)

// listModels prints available models (with optional fuzzy search).
func listModels(query string) {
	mreg := modelregistry.New()
	all := mreg.GetAll()

	if query != "true" {
		// Filter by query
		var filtered []types.Model
		for _, m := range all {
			if strings.Contains(strings.ToLower(m.ID), strings.ToLower(query)) ||
				strings.Contains(strings.ToLower(m.Provider), strings.ToLower(query)) {
				filtered = append(filtered, m)
			}
		}
		all = filtered
	}

	fmt.Printf("Available models (%d):\n", len(all))
	for _, m := range all {
		full := m.Provider + "/" + m.ID
		reasoning := ""
		if m.Reasoning {
			reasoning = " [reasoning]"
		}
		vision := ""
		for _, t := range m.InputTypes {
			if t == "image" {
				vision = " [vision]"
				break
			}
		}
		cost := ""
		if m.Cost.Input > 0 {
			cost = fmt.Sprintf(" ($%.2f/$%.2f)", m.Cost.Input, m.Cost.Output)
		}
		ctx := ""
		if m.ContextWindow > 0 {
			ctx = fmt.Sprintf(" [%dK ctx]", m.ContextWindow/1000)
		}
		fmt.Printf("  %-40s%s%s%s%s\n", full, reasoning, vision, cost, ctx)
	}
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

// exportSession exports a session to HTML.
func exportSession(sess *session.Session, path string) {
	// Simple JSON export for now
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

// processSentinel handles sentinel commands from slash command output.
func processSentinel(result string, eng interface{}, mgr *session.Manager, sess *session.Session, ctx interface{}) bool {
	// Check for sentinel markers in command output
	if strings.Contains(result, "__XIHU_SENTINEL__") {
		// Handle special commands
		return true
	}
	return false
}
