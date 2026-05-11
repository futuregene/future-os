// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"encoding/json"
	"os"
	"os/exec"
	"path/filepath"
	"runtime"
	"strings"


	"github.com/huichen/xihu/internal/auth"
	"github.com/huichen/xihu/internal/tui/components"


)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) showLoginDialog() {
	// Load current auth store to show configured providers
	authStore, _ := auth.LoadAuth()
	configuredProviders := make(map[string]bool)
	if authStore != nil {
		for name := range authStore.Entries {
			configuredProviders[name] = true
		}
	}

	status := func(name string) string {
		if configuredProviders[name] {
			return " \u2713 configured"
		}
		// Check environment variables
		envKeys := map[string]string{
			"anthropic": "ANTHROPIC_API_KEY",
			"openai":    "OPENAI_API_KEY",
			"google":    "GOOGLE_API_KEY",
		}
		if key, ok := envKeys[name]; ok && os.Getenv(key) != "" {
			return " (env:" + key + ")"
		}
		return ""
	}

	items := []components.SelectorItem{
		{Label: "Anthropic" + status("anthropic"), Description: "api.anthropic.com \u2014 Claude models", Value: "anthropic"},
		{Label: "OpenAI" + status("openai"), Description: "api.openai.com \u2014 GPT models", Value: "openai"},
		{Label: "Google" + status("google"), Description: "generativelanguage.googleapis.com \u2014 Gemini models", Value: "google"},
		{Label: "Custom Provider", Description: "Any OpenAI-compatible endpoint", Value: "custom"},
	}

	onSelect := func(value string) {
		// Open browser to provider's API key page
		urls := map[string]string{
			"anthropic": "https://console.anthropic.com/settings/keys",
			"openai":    "https://platform.openai.com/api-keys",
			"google":    "https://aistudio.google.com/app/apikey",
		}
		if url, ok := urls[value]; ok {
			go func() {
				cmd := exec.Command("open", url) // macOS
				if runtime.GOOS == "linux" {
					cmd = exec.Command("xdg-open", url)
				} else if runtime.GOOS == "windows" {
					cmd = exec.Command("rundll32", "url.dll,FileProtocolHandler", url)
				}
				cmd.Start()
			}()
		}
		m.showAPIKeyInput(value)
	}

	m.overlay.ShowSelectorStayOnSelect("Select provider to configure:", items, onSelect, nil, 60, 10)
}

// showAPIKeyInput shows a text input overlay for entering an API key.
func (m *AppModel) showAPIKeyInput(provider string) {
	label := "Enter API key:"

	onSubmit := func(value string) {
		value = strings.TrimSpace(value)
		if value == "" {
			m.chat.AppendSystem("No key entered for " + provider)
			return
		}
		if err := m.saveAPIKey(provider, value); err != nil {
			m.chat.AppendError("Failed to save API key for " + provider + ": " + err.Error())
			return
		}
		home, _ := os.UserHomeDir()
		authPath := filepath.Join(home, ".xihu", "auth.json")
		m.chat.AppendSystem("Saved API key for " + provider + ". Credentials saved to " + authPath)
	}

	m.overlay.ShowInput(label, onSubmit, nil, 50, 6)
}

// saveAPIKey saves an API key to the auth store.
func (m *AppModel) saveAPIKey(provider, key string) error {
	home, err := os.UserHomeDir()
	if err != nil {
		return err
	}
	authDir := filepath.Join(home, ".xihu")
	if err := os.MkdirAll(authDir, 0755); err != nil {
		return err
	}
	authPath := filepath.Join(authDir, "auth.json")

	// Load existing store
	store, err := auth.LoadAuth()
	if err != nil {
		store = &auth.Store{Entries: make(map[string]auth.Entry)}
	}
	store.Entries[provider] = auth.Entry{Type: "api_key", Key: key}

	// Serialize
	data := make(map[string]map[string]string)
	for name, entry := range store.Entries {
		data[name] = map[string]string{"type": entry.Type, "key": entry.Key}
	}
	raw, err := json.MarshalIndent(data, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(authPath, raw, 0600)
}

// showLogoutDialog shows a confirmation and removes stored credentials.
func (m *AppModel) showLogoutDialog() {
	authStore, err := auth.LoadAuth()
	if err != nil || authStore == nil || len(authStore.Entries) == 0 {
		m.chat.AppendSystem("No stored credentials to remove. /logout only removes credentials saved by /login; environment variables and models.json config are unchanged.")
		return
	}

	items := make([]components.SelectorItem, 0, len(authStore.Entries)+1)
	for name, entry := range authStore.Entries {
		keyPreview := strings.Repeat("*", len(entry.Key)-4) + entry.Key[len(entry.Key)-4:]
		if len(entry.Key) <= 4 {
			keyPreview = strings.Repeat("*", len(entry.Key))
		}
		items = append(items, components.SelectorItem{
			Label:       name + " (" + keyPreview + ")",
			Description: "Remove stored " + name + " API key",
			Value:       name,
		})
	}
	items = append(items, components.SelectorItem{
		Label:       "Remove All",
		Description: "Clear all stored credentials",
		Value:       "__all__",
	})

	onSelect := func(value string) {
		if value == "__all__" {
			if err := m.removeAllAPIKeys(); err != nil {
				m.chat.AppendError("Failed to clear credentials: " + err.Error())
				return
			}
			m.chat.AppendSystem("All stored API keys removed. Environment variables and models.json config are unchanged.")
			return
		}
		if err := m.removeAPIKey(value); err != nil {
			m.chat.AppendError("Failed to remove " + value + " key: " + err.Error())
			return
		}
		m.chat.AppendSystem("Removed stored API key for " + value + ". Environment variables and models.json config are unchanged.")
	}

	m.overlay.ShowSelectorStayOnSelect("Select provider to logout:", items, onSelect, nil, 56, len(items)+5)
}

// removeAPIKey removes a single provider's credentials.
func (m *AppModel) removeAPIKey(provider string) error {
	home, err := os.UserHomeDir()
	if err != nil {
		return err
	}
	authPath := filepath.Join(home, ".xihu", "auth.json")

	store, err := auth.LoadAuth()
	if err != nil {
		return err
	}
	delete(store.Entries, provider)

	data := make(map[string]map[string]string)
	for name, entry := range store.Entries {
		data[name] = map[string]string{"type": entry.Type, "key": entry.Key}
	}
	raw, err := json.MarshalIndent(data, "", "  ")
	if err != nil {
		return err
	}
	return os.WriteFile(authPath, raw, 0600)
}

// removeAllAPIKeys clears all stored credentials.
func (m *AppModel) removeAllAPIKeys() error {
	home, err := os.UserHomeDir()
	if err != nil {
		return err
	}
	authPath := filepath.Join(home, ".xihu", "auth.json")
	return os.WriteFile(authPath, []byte("{}\n"), 0600)
}

// handleShare exports the session as HTML and creates a secret GitHub gist.
// (TS pi-mono: interactive-mode.ts handleShareCommand)
