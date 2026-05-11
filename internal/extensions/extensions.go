// Package extensions provides a plugin architecture for xihu.
// Extensions can register tools, slash commands, and prompt templates,
// and are loaded dynamically from Go plugins (.so) or JSON configs.
package extensions

import (

	"github.com/huichen/xihu/internal/modelregistry"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/settings"
)

// ---------------------------------------------------------------------------
// Extension — the core plugin interface
// ---------------------------------------------------------------------------

// Extension is the interface that all extensions must implement.
// Go plugins export a symbol named "Extension" of type ExtensionFactory.
// Config-based extensions implement this interface via ConfigExtension.
type Extension interface {
	// Name returns the unique name of this extension.
	Name() string

	// Init is called once when the extension is loaded. The context provides
	// access to the session manager, settings, event bus, logger, and
	// registration methods for tools, slash commands, and prompts.
	Init(ctx ExtensionContext) error

	// Deinit is called on shutdown to allow the extension to clean up.
	Deinit() error
}

// ExtensionFactory is a function that creates a new Extension instance.
// Go plugins export a symbol of this type named "Extension".
type ExtensionFactory func() Extension

// ---------------------------------------------------------------------------
// Shortcut — extension keyboard shortcut
// ---------------------------------------------------------------------------

// ShortcutHandler is called when the registered shortcut key is pressed.
type ShortcutHandler func()

// ShortcutDef defines an extension keyboard shortcut.
type ShortcutDef struct {
	Key         string
	Description string
	Handler     ShortcutHandler
}

// ---------------------------------------------------------------------------
// Flag — extension CLI flag
// ---------------------------------------------------------------------------

// FlagType is the type of an extension flag.
type FlagType string

const (
	FlagString  FlagType = "string"
	FlagBool    FlagType = "boolean"
	FlagInt     FlagType = "number"
)

// FlagDef defines an extension CLI flag.
type FlagDef struct {
	Name        string
	Description string
	Type        FlagType
	Default     interface{}
}

// ---------------------------------------------------------------------------
// Autocomplete provider
// ---------------------------------------------------------------------------

// AutocompleteProvider returns candidate strings for a given query prefix.
// Extensions register providers to extend autocomplete beyond built-in
// slash commands, file paths, and models.
type AutocompleteProvider func(query string) []string

// ---------------------------------------------------------------------------
// ExtensionContext — the environment passed to extensions at Init time
// ---------------------------------------------------------------------------

// ExtensionActions provides runtime actions extensions can invoke.
// This avoids circular imports between extensions and engine/agent.
type ExtensionActions struct {
	// Abort aborts the current agent operation (like Ctrl+C).
	Abort func()

	// IsIdle returns true if the agent is not currently processing.
	IsIdle func() bool

	// SendUserMessage injects a user message (steer: interrupts; followUp: queues).
	SendUserMessage func(content string, deliverAs string) // deliverAs: "steer" | "followUp"

	// SendMessage injects a custom-typed message (mirrors TS sendMessage).
	SendMessage func(customType string, content interface{}, deliverAs string)

	// AppendEntry appends a custom session entry (mirrors TS appendEntry).
	AppendEntry func(customType string, data interface{})

	// SetModel switches the active model.
	SetModel func(provider, modelID string) error

	// GetThinkingLevel returns the current thinking level.
	GetThinkingLevel func() string

	// SetThinkingLevel sets the thinking level.
	SetThinkingLevel func(level string)

	// GetActiveTools returns the names of currently active tools.
	GetActiveTools func() []string

	// GetAllTools returns all registered tools with metadata.
	GetAllTools func() []ToolInfo

	// SetActiveTools sets the active tool names.
	SetActiveTools func(toolNames []string)

	// SetSessionName sets the session display name.
	SetSessionName func(name string)

	// GetSessionName returns the session display name.
	GetSessionName func() string

	// Exec executes a command (mirrors TS exec).
	Exec func(command string, args []string, timeoutMs int) (ExecResult, error)
}

// ToolInfo mirrors pi-mono's ToolInfo: name, description, sourceInfo.
type ToolInfo struct {
	Name        string     `json:"name"`
	Description string     `json:"description"`
	SourceInfo  SourceInfo `json:"sourceInfo"`
}

// ExecResult mirrors pi-mono's ExecResult.
type ExecResult struct {
	Stdout string `json:"stdout"`
	Stderr string `json:"stderr"`
	Code   int    `json:"code"`
}

// SourceInfo carries source metadata for extension resources.
// Mirrors pi-mono's SourceInfo type.
type SourceInfo struct {
	Path    string `json:"path"`
	Source  string `json:"source"`
	Scope   string `json:"scope"`  // "user" | "project" | "temporary"
	Origin  string `json:"origin"` // "package" | "top-level"
	BaseDir string `json:"baseDir,omitempty"`
}

// ProviderConfig mirrors pi-mono's ProviderConfig for LLM provider registration.
type ProviderConfig struct {
	Name        string            `json:"name,omitempty"`
	BaseURL     string            `json:"baseUrl,omitempty"`
	APIKey      string            `json:"apiKey,omitempty"`
	Headers     map[string]string `json:"headers,omitempty"`
	Models      []ProviderModel   `json:"models,omitempty"`
}

// ProviderModel mirrors pi-mono's ProviderModelConfig.
type ProviderModel struct {
	ID            string `json:"id"`
	Name          string `json:"name,omitempty"`
	ContextWindow int    `json:"contextWindow"`
	Reasoning     bool   `json:"reasoning"`
}

// Package-level provider registration
var providerRegistry = map[string]ProviderConfig{}

// ExtensionContext provides extensions with access to xihu internals and
// registration methods for tools, slash commands, and prompts.
type ExtensionContext struct {
	// SessionManager provides access to session persistence.
	SessionManager *session.Manager

	// Settings is the current merged settings configuration.
	Settings *settings.Settings

	// EventBus allows extensions to publish and subscribe to events.
	EventBus *EventBus

	// Logger is a logger for the extension to use.
	Logger Logger

	// CWD is the current working directory.
	CWD string

	// UI provides interactive UI methods (select, input, editor, etc.).
	// Only available in TUI mode; nil in print/CLI mode.
	UI ExtensionUI

	// Actions provides runtime actions (abort, isIdle, sendMessage, setModel).
	// Set by the engine after construction; nil-safe to call (no-ops if nil).
	Actions *ExtensionActions

	// handlers is the shared HandlerRegistry for typed event handlers with return values.
	// Set by ExtensionRunner on Init.
	handlers *HandlerRegistry

	// registry is the shared extension registry (set internally).
	registry *Registry

	// ModelRegistry is the engine-level model registry (nil if not available).
	// Extension-registered providers are automatically synced here.
	ModelRegistry interface {
		RegisterProvider(name string, override modelregistry.ProviderOverride)
		UnregisterProvider(name string)
	}
}

// On registers a typed event handler that can modify/cancel events.
