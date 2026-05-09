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

// DefaultKeyMap returns the default set of keybindings, aligned with TS pi-mono.
func DefaultKeyMap() *KeyMap {
	return &KeyMap{
		bindings: []Keybinding{
			// Global
			{Key: "Ctrl+C", Description: "Cancel / interrupt", Category: "global"},
			{Key: "Ctrl+C twice", Description: "Exit", Category: "global"},
			{Key: "Ctrl+D", Description: "Exit (on empty line)", Category: "global"},
			{Key: "Ctrl+Z", Description: "Suspend (background)", Category: "global"},

			// Editor navigation
			{Key: "Enter", Description: "Send message", Category: "editor"},
			{Key: "Alt+Enter", Description: "Queue follow-up message", Category: "editor"},
			{Key: "Shift+Enter", Description: "New line", Category: "editor"},
			{Key: "Ctrl+J", Description: "Insert newline", Category: "editor"},
			{Key: "Ctrl+A / Home", Description: "Beginning of line", Category: "editor"},
			{Key: "Ctrl+E / End", Description: "End of line", Category: "editor"},
			{Key: "Ctrl+K", Description: "Delete to end of line", Category: "editor"},
			{Key: "Ctrl+U", Description: "Delete to beginning of line", Category: "editor"},
			{Key: "Ctrl+W / Alt+Bksp", Description: "Delete word backwards", Category: "editor"},
			{Key: "Alt+D / Alt+Delete", Description: "Delete word forward", Category: "editor"},
			{Key: "Alt+Left / Ctrl+Left", Description: "Cursor word left", Category: "editor"},
			{Key: "Alt+Right / Ctrl+Right", Description: "Cursor word right", Category: "editor"},
			{Key: "Ctrl+Y", Description: "Yank (paste deleted text)", Category: "editor"},
			{Key: "Alt+Y", Description: "Yank pop (cycle kill ring)", Category: "editor"},
			{Key: "Ctrl+_", Description: "Undo", Category: "editor"},
			{Key: "Ctrl+]", Description: "Jump forward to character", Category: "editor"},
			{Key: "Ctrl+Alt+]", Description: "Jump backward to character", Category: "editor"},
			{Key: "Tab", Description: "Autocomplete", Category: "editor"},
			{Key: "Ctrl+G", Description: "Open external editor ($EDITOR)", Category: "editor"},

			// Chat
			{Key: "PgUp / Ctrl+U", Description: "Scroll up", Category: "chat"},
			{Key: "PgDn / Ctrl+D", Description: "Scroll down", Category: "chat"},
			{Key: "gg", Description: "Jump to top", Category: "chat"},
			{Key: "G", Description: "Jump to bottom (follow)", Category: "chat"},

			// Tools
			{Key: "Ctrl+O", Description: "Toggle tool output expand", Category: "tools"},
			{Key: "Ctrl+T", Description: "Toggle thinking blocks", Category: "tools"},
			{Key: "Shift+Tab", Description: "Cycle thinking level", Category: "tools"},
			{Key: "Ctrl+L", Description: "Open model selector", Category: "tools"},
			{Key: "Ctrl+P", Description: "Cycle model forward", Category: "tools"},
			{Key: "Ctrl+Shift+P", Description: "Cycle model backward", Category: "tools"},
			{Key: "Alt+Up", Description: "Dequeue follow-up messages", Category: "tools"},
			{Key: "Ctrl+H", Description: "Toggle header (compact/expanded)", Category: "tools"},

			// Session management (pi-mono aligned)
			{Key: "Ctrl+N", Description: "Toggle named session filter", Category: "tools"},
			{Key: "Ctrl+R", Description: "Rename session (in selector)", Category: "tools"},
			{Key: "Ctrl+S", Description: "Toggle session sort mode", Category: "tools"},
			{Key: "Ctrl+D", Description: "Delete session (in selector)", Category: "tools"},
			{Key: "Ctrl+Backspace", Description: "Delete session (in selector)", Category: "tools"},
			{Key: "Ctrl+P", Description: "Toggle session path display", Category: "tools"},
			{Key: "Ctrl+S", Description: "Save scoped model selection", Category: "tools"},
			{Key: "Ctrl+A", Description: "Enable all scoped models", Category: "tools"},
			{Key: "Ctrl+X", Description: "Clear all scoped models", Category: "tools"},
			{Key: "Alt+Up", Description: "Move model up in order", Category: "tools"},
			{Key: "Alt+Down", Description: "Move model down in order", Category: "tools"},

			// Tree navigation (pi-mono: session tree)
			{Key: "Ctrl+Left / Alt+Left", Description: "Fold tree branch / move up", Category: "tools"},
			{Key: "Ctrl+Right / Alt+Right", Description: "Unfold tree branch / move down", Category: "tools"},
			{Key: "Enter", Description: "Toggle fold on branch / select leaf", Category: "tools"},
			{Key: "Shift+L", Description: "Edit tree label", Category: "tools"},
			{Key: "Shift+T", Description: "Toggle label timestamp", Category: "tools"},

			// Tree filter modes (pi-mono: session tree filters)
			{Key: "Ctrl+D", Description: "Tree filter: default view", Category: "tools"},
			{Key: "Ctrl+T", Description: "Tree filter: hide tool results", Category: "tools"},
			{Key: "Ctrl+U", Description: "Tree filter: user messages only", Category: "tools"},
			{Key: "Ctrl+L", Description: "Tree filter: labeled only", Category: "tools"},
			{Key: "Ctrl+A", Description: "Tree filter: show all entries", Category: "tools"},
			{Key: "Ctrl+O", Description: "Tree filter: cycle forward", Category: "tools"},
			{Key: "Ctrl+Shift+O", Description: "Tree filter: cycle backward", Category: "tools"},

			// Clipboard
			{Key: "Ctrl+V", Description: "Paste text from clipboard", Category: "tools"},

			// Bash
			{Key: "!", Description: "Run bash command (with context)", Category: "global"},
			{Key: "!!", Description: "Run bash command (no context)", Category: "global"},

			// Slash commands overview (pi-mono aligned)
			{Key: "/model [name]", Description: "Set or show model", Category: "global"},
			{Key: "/session", Description: "Show session info and stats", Category: "global"},
			{Key: "/name [name]", Description: "Set session display name", Category: "global"},
			{Key: "/new", Description: "Start a new session", Category: "global"},
			{Key: "/sessions", Description: "Show session selector", Category: "global"},
			{Key: "/settings", Description: "Show current settings", Category: "global"},
			{Key: "/scoped-models", Description: "Show model configuration", Category: "global"},
			{Key: "/export [path]", Description: "Export session (HTML or .jsonl)", Category: "global"},
			{Key: "/import <file>", Description: "Import session from JSONL", Category: "global"},
			{Key: "/share", Description: "Share session as secret gist", Category: "global"},
			{Key: "/copy", Description: "Copy last agent message to clipboard", Category: "global"},
			{Key: "/fork [entry_id]", Description: "Fork session from entry", Category: "global"},
			{Key: "/clone", Description: "Clone current session", Category: "global"},
			{Key: "/tree", Description: "Show session tree", Category: "global"},
			{Key: "/compact", Description: "Manual context compaction", Category: "global"},
			{Key: "/resume <id>", Description: "Resume different session", Category: "global"},
			{Key: "/theme [name]", Description: "Show or change theme", Category: "global"},
			{Key: "/hotkeys", Description: "Show this help", Category: "global"},
			{Key: "/changelog", Description: "Show changelog", Category: "global"},
			{Key: "/login", Description: "Configure provider auth", Category: "global"},
			{Key: "/logout", Description: "Clear provider auth", Category: "global"},
			{Key: "/reload", Description: "Reload configuration", Category: "global"},
			{Key: "/quit", Description: "Exit xihu", Category: "global"},
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
