package extensions

import (
	"encoding/json"

	"github.com/huichen/xihu/pkg/types"
)

// ---------------------------------------------------------------------------
// Convenience package-level functions that operate on the global registry
// ---------------------------------------------------------------------------

// RegisterTool registers a tool in the global registry.
func RegisterTool(name string, handler func(args json.RawMessage) (string, error), description string, parameters json.RawMessage) error {
	return globalRegistry.RegisterTool(name, handler, description, parameters)
}

// RegisterSlashCommand registers a slash command in the global registry.
func RegisterSlashCommand(cmd string, handler SlashCommandHandler) error {
	return globalRegistry.RegisterSlashCommand(cmd, handler)
}

// RegisterPrompt registers a prompt template in the global registry.
func RegisterPrompt(name string, template string) error {
	return globalRegistry.RegisterPrompt(name, template)
}

// GetAllTools returns all tools from the global registry.
func GetAllTools() []types.AgentTool {
	return globalRegistry.GetAllTools()
}

// GetSlashCommand returns a registered slash command handler by name, or nil.
func GetSlashCommand(cmd string) SlashCommandHandler {
	return globalRegistry.GetSlashCommand(cmd)
}

// GetAllSlashCommands returns all slash commands from the global registry.
func GetAllSlashCommands() map[string]SlashCommandHandler {
	return globalRegistry.GetAllSlashCommands()
}

// GetAllPrompts returns all prompt templates from the global registry.
func GetAllPrompts() map[string]string {
	return globalRegistry.GetAllPrompts()
}

// RegisterShortcut registers a keyboard shortcut in the global registry.
func RegisterShortcut(key string, handler ShortcutHandler, description string) error {
	return globalRegistry.RegisterShortcut(key, handler, description)
}

// GetAllShortcuts returns all shortcuts from the global registry.
func GetAllShortcuts() map[string]ShortcutDef {
	return globalRegistry.GetAllShortcuts()
}

// RegisterFlag registers a CLI flag in the global registry.
func RegisterFlag(name string, description string, flagType FlagType, defaultVal interface{}) error {
	return globalRegistry.RegisterFlag(name, description, flagType, defaultVal)
}

// GetFlag returns a flag value from the global registry.
func GetFlag(name string) interface{} {
	return globalRegistry.GetFlag(name)
}

// SetFlagValue sets a flag value in the global registry.
func SetFlagValue(name string, value interface{}) {
	globalRegistry.SetFlagValue(name, value)
}

// GetAllFlags returns all flags from the global registry.
func GetAllFlags() map[string]FlagDef {
	return globalRegistry.GetAllFlags()
}

// AddAutocompleteProvider registers an autocomplete provider in the global registry.
func AddAutocompleteProvider(provider AutocompleteProvider) {
	globalRegistry.AddAutocompleteProvider(provider)
}

// GetAllAutocompleteProviders returns all autocomplete providers from the global registry.
func GetAllAutocompleteProviders() []AutocompleteProvider {
	return globalRegistry.GetAllAutocompleteProviders()
}
