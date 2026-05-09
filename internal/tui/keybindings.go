package tui

import (
	"encoding/json"
	"fmt"
	"os"
	"path/filepath"
	"sync"
)

// KeybindingID is a dot-separated binding identifier matching pi-mono conventions.
type KeybindingID string

// Editor navigation and editing bindings (pi-mono aligned).
const (
	// Navigation
	EditorCursorUp        KeybindingID = "tui.editor.cursorUp"
	EditorCursorDown      KeybindingID = "tui.editor.cursorDown"
	EditorCursorLeft      KeybindingID = "tui.editor.cursorLeft"
	EditorCursorRight     KeybindingID = "tui.editor.cursorRight"
	EditorCursorWordLeft  KeybindingID = "tui.editor.cursorWordLeft"
	EditorCursorWordRight KeybindingID = "tui.editor.cursorWordRight"
	EditorCursorLineStart KeybindingID = "tui.editor.cursorLineStart"
	EditorCursorLineEnd   KeybindingID = "tui.editor.cursorLineEnd"
	EditorJumpForward     KeybindingID = "tui.editor.jumpForward"
	EditorJumpBackward    KeybindingID = "tui.editor.jumpBackward"
	EditorPageUp          KeybindingID = "tui.editor.pageUp"
	EditorPageDown        KeybindingID = "tui.editor.pageDown"

	// Editing
	EditorDeleteCharBackward KeybindingID = "tui.editor.deleteCharBackward"
	EditorDeleteCharForward  KeybindingID = "tui.editor.deleteCharForward"
	EditorDeleteWordBackward KeybindingID = "tui.editor.deleteWordBackward"
	EditorDeleteWordForward  KeybindingID = "tui.editor.deleteWordForward"
	EditorDeleteToLineStart  KeybindingID = "tui.editor.deleteToLineStart"
	EditorDeleteToLineEnd    KeybindingID = "tui.editor.deleteToLineEnd"
	EditorYank               KeybindingID = "tui.editor.yank"
	EditorYankPop            KeybindingID = "tui.editor.yankPop"
	EditorUndo               KeybindingID = "tui.editor.undo"

	// Input actions
	InputNewLine KeybindingID = "tui.input.newLine"
	InputSubmit  KeybindingID = "tui.input.submit"
	InputTab     KeybindingID = "tui.input.tab"
	InputCopy    KeybindingID = "tui.input.copy"

	// Selection actions
	SelectUp     KeybindingID = "tui.select.up"
	SelectDown   KeybindingID = "tui.select.down"
	SelectPageUp KeybindingID = "tui.select.pageUp"
	SelectPageDn KeybindingID = "tui.select.pageDown"

	// Global TUI actions (used by app.go dispatch)
	GlobalInterrupt         KeybindingID = "tui.global.interrupt"
	GlobalClear             KeybindingID = "tui.global.clear"
	GlobalExit              KeybindingID = "tui.global.exit"
	GlobalToggleHeader      KeybindingID = "tui.global.toggleHeader"
	GlobalToggleTools       KeybindingID = "tui.global.toggleTools"
	GlobalToggleThinking    KeybindingID = "tui.global.toggleThinking"
	GlobalModelSelector     KeybindingID = "tui.global.modelSelector"
	GlobalCycleModelFwd     KeybindingID = "tui.global.cycleModelForward"
	GlobalCycleModelBack    KeybindingID = "tui.global.cycleModelBackward"
	GlobalCycleThinking     KeybindingID = "tui.global.cycleThinking"
	GlobalExternalEditor    KeybindingID = "tui.global.externalEditor"
)

// KeybindingDef defines a keybinding with its default keys and description.
type KeybindingDef struct {
	DefaultKeys []string
	Description string
}

// DefaultKeybindingDefs returns all built-in keybinding definitions,
// matching pi-mono's TUI_KEYBINDINGS.
func DefaultKeybindingDefs() map[KeybindingID]KeybindingDef {
	return map[KeybindingID]KeybindingDef{
		// Editor navigation
		EditorCursorUp:        {DefaultKeys: []string{"up"}, Description: "Move cursor up"},
		EditorCursorDown:      {DefaultKeys: []string{"down"}, Description: "Move cursor down"},
		EditorCursorLeft:      {DefaultKeys: []string{"left", "ctrl+b"}, Description: "Move cursor left"},
		EditorCursorRight:     {DefaultKeys: []string{"right", "ctrl+f"}, Description: "Move cursor right"},
		EditorCursorWordLeft:  {DefaultKeys: []string{"alt+left", "ctrl+left", "alt+b"}, Description: "Move cursor word left"},
		EditorCursorWordRight: {DefaultKeys: []string{"alt+right", "ctrl+right", "alt+f"}, Description: "Move cursor word right"},
		EditorCursorLineStart: {DefaultKeys: []string{"home", "ctrl+a"}, Description: "Move to line start"},
		EditorCursorLineEnd:   {DefaultKeys: []string{"end", "ctrl+e"}, Description: "Move to line end"},
		EditorJumpForward:     {DefaultKeys: []string{"ctrl+]"}, Description: "Jump forward to character"},
		EditorJumpBackward:    {DefaultKeys: []string{"ctrl+alt+]"}, Description: "Jump backward to character"},
		EditorPageUp:          {DefaultKeys: []string{"pgup"}, Description: "Page up"},
		EditorPageDown:        {DefaultKeys: []string{"pgdown"}, Description: "Page down"},

		// Editor editing
		EditorDeleteCharBackward: {DefaultKeys: []string{"backspace"}, Description: "Delete character backward"},
		EditorDeleteCharForward:  {DefaultKeys: []string{"delete", "ctrl+d"}, Description: "Delete character forward"},
		EditorDeleteWordBackward: {DefaultKeys: []string{"ctrl+w", "alt+backspace"}, Description: "Delete word backward"},
		EditorDeleteWordForward:  {DefaultKeys: []string{"alt+d", "alt+delete"}, Description: "Delete word forward"},
		EditorDeleteToLineStart:  {DefaultKeys: []string{"ctrl+u"}, Description: "Delete to line start"},
		EditorDeleteToLineEnd:    {DefaultKeys: []string{"ctrl+k"}, Description: "Delete to line end"},
		EditorYank:               {DefaultKeys: []string{"ctrl+y"}, Description: "Yank (paste deleted text)"},
		EditorYankPop:            {DefaultKeys: []string{"alt+y"}, Description: "Yank pop (cycle kill ring)"},
		EditorUndo:               {DefaultKeys: []string{"ctrl+-", "ctrl+_", "ctrl+/"}, Description: "Undo"},

		// Input
		InputNewLine: {DefaultKeys: []string{"shift+enter", "ctrl+j"}, Description: "Insert newline"},
		InputSubmit:  {DefaultKeys: []string{"enter"}, Description: "Submit input"},
		InputTab:     {DefaultKeys: []string{"tab"}, Description: "Tab / autocomplete"},
		InputCopy:    {DefaultKeys: []string{"ctrl+c"}, Description: "Copy selection"},

		// Selection
		SelectUp:     {DefaultKeys: []string{"up"}, Description: "Move selection up"},
		SelectDown:   {DefaultKeys: []string{"down"}, Description: "Move selection down"},
		SelectPageUp: {DefaultKeys: []string{"pgup"}, Description: "Selection page up"},
		SelectPageDn: {DefaultKeys: []string{"pgdown"}, Description: "Selection page down"},

		// Global TUI
		GlobalInterrupt:      {DefaultKeys: []string{"esc"}, Description: "Interrupt / cancel"},
		GlobalClear:          {DefaultKeys: []string{"ctrl+c"}, Description: "Clear input (double: exit)"},
		GlobalExit:           {DefaultKeys: []string{"ctrl+d"}, Description: "Exit (on empty line)"},
		GlobalToggleHeader:   {DefaultKeys: []string{"ctrl+h"}, Description: "Toggle header"},
		GlobalToggleTools:    {DefaultKeys: []string{"ctrl+o"}, Description: "Toggle tool outputs"},
		GlobalToggleThinking: {DefaultKeys: []string{"ctrl+t"}, Description: "Toggle thinking blocks"},
		GlobalModelSelector:  {DefaultKeys: []string{"ctrl+l"}, Description: "Open model selector"},
		GlobalCycleModelFwd:  {DefaultKeys: []string{"ctrl+p"}, Description: "Cycle model forward"},
		GlobalCycleModelBack: {DefaultKeys: []string{"ctrl+shift+p"}, Description: "Cycle model backward"},
		GlobalCycleThinking:  {DefaultKeys: []string{"shift+tab"}, Description: "Cycle thinking level"},
		GlobalExternalEditor: {DefaultKeys: []string{"ctrl+g"}, Description: "Open external editor"},
	}
}

// UserBindings maps binding IDs to user-configured key strings.
// JSON format: {"tui.editor.cursorLeft": ["left", "ctrl+b"], ...}
// Values accept a single string or an array of strings.
type UserBindings map[KeybindingID][]string

// UnmarshalJSON handles both string and array values.
func (ub *UserBindings) UnmarshalJSON(data []byte) error {
	raw := make(map[string]interface{})
	if err := json.Unmarshal(data, &raw); err != nil {
		return err
	}
	*ub = make(UserBindings)
	for k, v := range raw {
		switch val := v.(type) {
		case string:
			(*ub)[KeybindingID(k)] = []string{val}
		case []interface{}:
			keys := make([]string, 0, len(val))
			for _, item := range val {
				if s, ok := item.(string); ok {
					keys = append(keys, s)
				}
			}
			if len(keys) > 0 {
				(*ub)[KeybindingID(k)] = keys
			}
		case nil:
			// skip nil values
		default:
			return fmt.Errorf("keybinding %q: expected string or array, got %T", k, v)
		}
	}
	return nil
}

// KeybindingConflict records when user config maps the same key to multiple bindings.
type KeybindingConflict struct {
	Key      string
	Bindings []KeybindingID
}

// KeybindingsManager holds active keybinding definitions and user overrides.
// It provides Matches() for checking if a key press matches a named binding.
type KeybindingsManager struct {
	mu        sync.RWMutex
	defs      map[KeybindingID]KeybindingDef
	user      UserBindings
	keysByID  map[KeybindingID][]string
	conflicts []KeybindingConflict
}

// NewKeybindingsManager creates a manager with defaults and optional user overrides.
func NewKeybindingsManager(user UserBindings) *KeybindingsManager {
	km := &KeybindingsManager{
		defs: DefaultKeybindingDefs(),
		user: user,
	}
	km.rebuild()
	return km
}

// LoadUserBindings loads user keybindings from ~/.xihu/keybindings.json.
// Returns nil, nil if the file doesn't exist.
func LoadUserBindings() (UserBindings, error) {
	home, err := os.UserHomeDir()
	if err != nil {
		return nil, err
	}
	path := filepath.Join(home, ".xihu", "keybindings.json")
	data, err := os.ReadFile(path)
	if err != nil {
		if os.IsNotExist(err) {
			return nil, nil
		}
		return nil, err
	}
	var ub UserBindings
	if err := json.Unmarshal(data, &ub); err != nil {
		return nil, err
	}
	return ub, nil
}

func (km *KeybindingsManager) rebuild() {
	km.keysByID = make(map[KeybindingID][]string)
	km.conflicts = nil

	// Track user key claims for conflict detection
	userClaims := make(map[string][]KeybindingID)
	for id, keys := range km.user {
		if _, exists := km.defs[id]; !exists {
			continue
		}
		for _, k := range keys {
			if k != "" {
				userClaims[k] = append(userClaims[k], id)
			}
		}
	}
	for key, bindings := range userClaims {
		if len(bindings) > 1 {
			km.conflicts = append(km.conflicts, KeybindingConflict{Key: key, Bindings: bindings})
		}
	}

	// Resolve: user overrides take priority, otherwise use defaults
	for id, def := range km.defs {
		if userKeys, ok := km.user[id]; ok && len(userKeys) > 0 {
			// Filter out empty strings from sparse user config
			filtered := make([]string, 0, len(userKeys))
			for _, k := range userKeys {
				if k != "" {
					filtered = append(filtered, k)
				}
			}
			if len(filtered) > 0 {
				km.keysByID[id] = filtered
			} else {
				km.keysByID[id] = def.DefaultKeys
			}
		} else {
			km.keysByID[id] = def.DefaultKeys
		}
	}
}

// Matches checks if a key string matches any key for the given binding.
func (km *KeybindingsManager) Matches(keyString string, binding KeybindingID) bool {
	km.mu.RLock()
	defer km.mu.RUnlock()
	for _, k := range km.keysByID[binding] {
		if k == keyString {
			return true
		}
	}
	return false
}

// MatchesAny checks if a key string matches any of the given bindings.
// Returns the first matching binding ID or empty string.
func (km *KeybindingsManager) MatchesAny(keyString string, bindings ...KeybindingID) KeybindingID {
	km.mu.RLock()
	defer km.mu.RUnlock()
	for _, binding := range bindings {
		for _, k := range km.keysByID[binding] {
			if k == keyString {
				return binding
			}
		}
	}
	return ""
}

// GetKeys returns the resolved keys for a binding.
func (km *KeybindingsManager) GetKeys(binding KeybindingID) []string {
	km.mu.RLock()
	defer km.mu.RUnlock()
	keys := km.keysByID[binding]
	result := make([]string, len(keys))
	copy(result, keys)
	return result
}

// GetConflicts returns current keybinding conflicts from user config.
func (km *KeybindingsManager) GetConflicts() []KeybindingConflict {
	km.mu.RLock()
	defer km.mu.RUnlock()
	result := make([]KeybindingConflict, len(km.conflicts))
	for i, c := range km.conflicts {
		bindings := make([]KeybindingID, len(c.Bindings))
		copy(bindings, c.Bindings)
		result[i] = KeybindingConflict{Key: c.Key, Bindings: bindings}
	}
	return result
}

// GetDefinition returns the definition for a binding.
func (km *KeybindingsManager) GetDefinition(binding KeybindingID) (KeybindingDef, bool) {
	km.mu.RLock()
	defer km.mu.RUnlock()
	def, ok := km.defs[binding]
	return def, ok
}

// GetUserBindings returns a copy of the current user bindings.
func (km *KeybindingsManager) GetUserBindings() UserBindings {
	km.mu.RLock()
	defer km.mu.RUnlock()
	result := make(UserBindings, len(km.user))
	for k, v := range km.user {
		keys := make([]string, len(v))
		copy(keys, v)
		result[k] = keys
	}
	return result
}

// SetUserBindings replaces user bindings and rebuilds.
func (km *KeybindingsManager) SetUserBindings(user UserBindings) {
	km.mu.Lock()
	defer km.mu.Unlock()
	km.user = user
	km.rebuild()
}

// Reload re-applies user bindings (for use after /reload).
func (km *KeybindingsManager) Reload(user UserBindings) {
	km.SetUserBindings(user)
}

// Global singleton (matching pi-mono's getKeybindings pattern).
var (
	globalKB   *KeybindingsManager
	globalKBmu sync.Mutex
)

// GetKeybindings returns the global KeybindingsManager singleton.
// On first call, it loads user bindings from disk and creates the manager.
func GetKeybindings() *KeybindingsManager {
	globalKBmu.Lock()
	defer globalKBmu.Unlock()
	if globalKB == nil {
		user, _ := LoadUserBindings()
		globalKB = NewKeybindingsManager(user)
	}
	return globalKB
}

// SetKeybindings replaces the global singleton (used for testing).
func SetKeybindings(km *KeybindingsManager) {
	globalKBmu.Lock()
	defer globalKBmu.Unlock()
	globalKB = km
}
