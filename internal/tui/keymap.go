package tui

// Keybinding represents a single keyboard shortcut.
type Keybinding struct {
	Key         string
	Description string
	Category    string // "global", "editor", "chat", "tools"
}

// KeyMap holds all keybindings for the TUI.
type KeyMap struct {
	bindings []Keybinding
}

// DefaultKeyMap returns the default set of keybindings.
func DefaultKeyMap() *KeyMap {
	return &KeyMap{
		bindings: []Keybinding{
			// Global
			{Key: "Ctrl+C", Description: "Cancel / interrupt", Category: "global"},
			{Key: "Ctrl+D", Description: "Exit (on empty line)", Category: "global"},
			{Key: "Ctrl+Z", Description: "Suspend (background)", Category: "global"},
			{Key: "Ctrl+L", Description: "Clear screen", Category: "global"},

			// Editor
			{Key: "Enter", Description: "Send message", Category: "editor"},
			{Key: "Shift+Enter", Description: "New line", Category: "editor"},
			{Key: "Ctrl+A", Description: "Beginning of line", Category: "editor"},
			{Key: "Ctrl+E", Description: "End of line", Category: "editor"},
			{Key: "Ctrl+K", Description: "Delete to end of line", Category: "editor"},
			{Key: "Ctrl+U", Description: "Delete to beginning of line", Category: "editor"},
			{Key: "Ctrl+W", Description: "Delete word backwards", Category: "editor"},
			{Key: "Tab", Description: "Autocomplete", Category: "editor"},
			{Key: "Ctrl+G", Description: "Open external editor ($EDITOR)", Category: "editor"},

			// Chat
			{Key: "PgUp / Ctrl+U", Description: "Scroll up", Category: "chat"},
			{Key: "PgDn / Ctrl+D", Description: "Scroll down", Category: "chat"},
			{Key: "gg", Description: "Jump to top", Category: "chat"},
			{Key: "G", Description: "Jump to bottom (follow)", Category: "chat"},

			// Tools
			{Key: "Ctrl+O", Description: "Toggle tool output expand", Category: "tools"},
			{Key: "Ctrl+T", Description: "Cycle thinking level", Category: "tools"},

			// Slash commands overview
			{Key: "/model [name]", Description: "Set or show model", Category: "global"},
			{Key: "/session", Description: "Show session stats", Category: "global"},
			{Key: "/settings", Description: "Interactive settings", Category: "global"},
			{Key: "/theme", Description: "Change theme", Category: "global"},
			{Key: "/hotkeys", Description: "Show this help", Category: "global"},
			{Key: "/quit", Description: "Exit cobalt", Category: "global"},
		},
	}
}

// ByCategory returns bindings grouped by category.
func (k *KeyMap) ByCategory() map[string][]Keybinding {
	groups := make(map[string][]Keybinding)
	for _, b := range k.bindings {
		groups[b.Category] = append(groups[b.Category], b)
	}
	return groups
}

// All returns all bindings.
func (k *KeyMap) All() []Keybinding {
	return k.bindings
}
