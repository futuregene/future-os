package components

// Binding ID constants matching pi-mono's keybinding identifiers.
// These are used by both the editor (for key checks) and the
// KeybindingsManager in the tui package (for definitions).
const (
	// Navigation
	BindCursorUp        = "tui.editor.cursorUp"
	BindCursorDown      = "tui.editor.cursorDown"
	BindCursorLeft      = "tui.editor.cursorLeft"
	BindCursorRight     = "tui.editor.cursorRight"
	BindCursorWordLeft  = "tui.editor.cursorWordLeft"
	BindCursorWordRight = "tui.editor.cursorWordRight"
	BindCursorLineStart = "tui.editor.cursorLineStart"
	BindCursorLineEnd   = "tui.editor.cursorLineEnd"
	BindJumpForward     = "tui.editor.jumpForward"
	BindJumpBackward    = "tui.editor.jumpBackward"
	BindPageUp          = "tui.editor.pageUp"
	BindPageDown        = "tui.editor.pageDown"

	// Editing
	BindDeleteCharBackward = "tui.editor.deleteCharBackward"
	BindDeleteCharForward  = "tui.editor.deleteCharForward"
	BindDeleteWordBackward = "tui.editor.deleteWordBackward"
	BindDeleteWordForward  = "tui.editor.deleteWordForward"
	BindDeleteToLineStart  = "tui.editor.deleteToLineStart"
	BindDeleteToLineEnd    = "tui.editor.deleteToLineEnd"
	BindYank               = "tui.editor.yank"
	BindYankPop            = "tui.editor.yankPop"
	BindUndo               = "tui.editor.undo"

	// Input
	BindNewLine = "tui.input.newLine"
	BindSubmit  = "tui.input.submit"
	BindTab     = "tui.input.tab"
	BindCopy    = "tui.input.copy"

	// Selection
	BindSelectUp      = "tui.select.up"
	BindSelectDown    = "tui.select.down"
	BindSelectPageUp  = "tui.select.pageUp"
	BindSelectPageDn  = "tui.select.pageDown"
	BindSelectConfirm = "tui.select.confirm"
	BindSelectCancel  = "tui.select.cancel"
)

// KeyMatcher is a function that checks if a key string matches a binding ID.
// The editor uses this for user-configurable keybindings. When nil, the
// editor falls back to its built-in hardcoded defaults.
type KeyMatcher func(keyString, bindingID string) bool

// IsKillKey returns true if the key string matches any kill-ring-related binding.
func IsKillKey(ks string, matchKey KeyMatcher) bool {
	if matchKey != nil {
		return matchKey(ks, BindDeleteToLineEnd) ||
			matchKey(ks, BindDeleteToLineStart) ||
			matchKey(ks, BindDeleteWordBackward) ||
			matchKey(ks, BindDeleteWordForward)
	}
	// Hardcoded fallback
	switch ks {
	case "ctrl+k", "ctrl+u", "ctrl+w", "alt+backspace", "alt+d", "alt+delete":
		return true
	}
	return false
}

// IsVerticalKey returns true if the key string matches a vertical navigation binding.
func IsVerticalKey(ks string, matchKey KeyMatcher) bool {
	if matchKey != nil {
		return matchKey(ks, BindCursorUp) ||
			matchKey(ks, BindCursorDown) ||
			matchKey(ks, BindPageUp) ||
			matchKey(ks, BindPageDown)
	}
	switch ks {
	case "up", "down", "pgup", "pgdown":
		return true
	}
	return false
}
