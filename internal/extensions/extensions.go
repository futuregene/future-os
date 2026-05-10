// Package extensions provides a plugin architecture for xihu.
// Extensions can register tools, slash commands, and prompt templates,
// and are loaded dynamically from Go plugins (.so) or JSON configs.
package extensions

import (
	"encoding/json"
	"time"

	"github.com/huichen/xihu/internal/modelregistry"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/settings"
	"github.com/huichen/xihu/pkg/types"
)

// ---------------------------------------------------------------------------
// ExtensionUI — UI methods available to extensions at runtime
// ---------------------------------------------------------------------------

// TerminalInputResult is returned by TerminalInputHandler.
type TerminalInputResult struct {
	Consume bool   // if true, the input is consumed and not processed further
	Data    string // if non-empty, replaces the original input string
}

// TerminalInputHandler processes raw terminal input before normal key handling.
// Return nil to pass the input through normally.
type TerminalInputHandler func(data string) *TerminalInputResult

// ExtensionUIDialogOptions configures an extension UI dialog.
type ExtensionUIDialogOptions struct {
	Timeout time.Duration
	Signal  chan struct{} // close to programmatically dismiss
}

// ExtensionUI provides UI interaction methods for extensions.
// Each method blocks until the user responds or the dialog is dismissed.
type ExtensionUI interface {
	// Select shows a list selector and returns the user's choice.
	Select(title string, options []string, opts *ExtensionUIDialogOptions) (string, error)

	// Confirm shows a confirmation dialog. Returns true if confirmed.
	Confirm(title, message string, opts *ExtensionUIDialogOptions) (bool, error)

	// Input shows a text input dialog and returns the user's input.
	Input(title, placeholder string, opts *ExtensionUIDialogOptions) (string, error)

	// Editor shows a multi-line editor and returns the user's text.
	Editor(title, prefill string) (string, error)

	// Notify shows a notification (info, warning, error).
	Notify(message string, notifyType string)

	// SetStatus sets a status line in the footer. Pass empty text to clear.
	SetStatus(key, text string)

	// SetTitle sets the terminal window/tab title.
	SetTitle(title string)

	// SetHiddenThinkingLabel sets the label for hidden thinking blocks.
	// Pass empty string to restore the default ("Thinking…").
	SetHiddenThinkingLabel(label string)

	// SetWorkingMessage sets the working message shown during streaming.
	// Pass empty string to restore the default ("Generating…").
	SetWorkingMessage(message string)

	// SetWorkingVisible shows or hides the working loader during streaming.
	SetWorkingVisible(visible bool)

	// SetWorkingIndicator sets the spinner frames for the streaming loader.
	// Pass nil or empty slice to restore default spinner.
	SetWorkingIndicator(frames []string, intervalMs int)

	// OnTerminalInput registers a raw terminal input handler.
	// The handler is called for every keypress before normal processing.
	// Return &TerminalInputResult{Consume: true} to stop further processing.
	// Returns an unsubscribe function. Call it to remove the handler.
	OnTerminalInput(handler TerminalInputHandler) (unsubscribe func())

	// PasteToEditor pastes text into the main editor with paste handling.
	PasteToEditor(text string)

	// SetEditorText sets the text content of the main editor.
	SetEditorText(text string)

	// GetEditorText returns the current text in the main editor.
	GetEditorText() string

	// SetWidget sets or removes a widget rendered above or below the editor.
	// key uniquely identifies the widget. content is the rendered widget text
	// (multiple lines joined by \n). Pass empty content to remove the widget.
	// placement is "aboveEditor" or "belowEditor".
	SetWidget(key, content, placement string)

	// Custom shows a custom dialog with title, content text, and action buttons.
	// Returns the value of the selected button, or an error if cancelled.
	Custom(title, content string, buttons []CustomButton, opts *ExtensionUIDialogOptions) (string, error)

	// GetAllThemes returns all available themes with their names and file paths.
	// Built-in themes have empty paths.
	GetAllThemes() []ThemeInfo

	// SetTheme applies a theme by name. Returns an error if the theme is not found.
	SetTheme(name string) error

	// GetCurrentThemeName returns the name of the currently active theme.
	GetCurrentThemeName() string

	// GetToolsExpanded returns whether tool outputs are currently expanded.
	GetToolsExpanded() bool

	// SetToolsExpanded sets the tool output expansion state.
	SetToolsExpanded(expanded bool)

	// AddAutocompleteProvider registers an autocomplete provider function.
	// The provider is called with the current query prefix and returns candidate strings.
	AddAutocompleteProvider(provider AutocompleteProvider)

	// SetFooter replaces the footer with a custom component factory.
	// factory is a func() FooterComponent. Pass nil to restore the default footer.
	// FooterComponent must implement: View(width int) string.
	SetFooter(factory interface{})

	// SetHeader replaces the header with a custom component factory.
	// factory is a func() HeaderComponent. Pass nil to restore the default header.
	// HeaderComponent must implement: View(width int) string.
	SetHeader(factory interface{})

	// GetTheme loads a theme by name. Returns nil if not found.
	GetTheme(name string) interface{}

	// SetEditorComponent replaces the entire editor component.
	// factory is a func() EditorComponent. Pass nil to restore the default editor.
	// EditorComponent must implement:
	//   Init() tea.Cmd
	//   Update(msg tea.Msg) (tea.Model, tea.Cmd)
	//   View() string
	//   Value() string
	//   SetValue(string)
	//   Reset()
	//   Focus() tea.Cmd
	//   Blur()
	//   SetWidth(int), SetHeight(int), Height() int, Empty() bool
	SetEditorComponent(factory interface{})

	// GetEditorComponent returns the current custom editor factory, or nil.
	GetEditorComponent() interface{}
}

// ThemeInfo describes an available theme for GetAllThemes.
type ThemeInfo struct {
	Name string
	Path string // empty for built-in themes
}

// CustomButton represents an action button in a custom dialog.
type CustomButton struct {
	Label string // display text
	Value string // value returned when selected
}

// noopUI is a no-op ExtensionUI used when no TUI is available.
type noopUI struct{}

func (n *noopUI) Select(string, []string, *ExtensionUIDialogOptions) (string, error) { return "", nil }
func (n *noopUI) Confirm(string, string, *ExtensionUIDialogOptions) (bool, error)   { return false, nil }
	func (n *noopUI) GetAllThemes() []ThemeInfo { return nil }
	func (n *noopUI) SetTheme(string) error     { return nil }
	func (n *noopUI) GetCurrentThemeName() string { return "" }
	func (n *noopUI) GetToolsExpanded() bool    { return false }
	func (n *noopUI) SetToolsExpanded(bool)     {}
	func (n *noopUI) AddAutocompleteProvider(AutocompleteProvider) {}
func (n *noopUI) SetFooter(interface{})                        {}
func (n *noopUI) SetHeader(interface{})                        {}
func (n *noopUI) GetTheme(string) interface{}                                   { return nil }
func (n *noopUI) SetEditorComponent(interface{})                                {}
func (n *noopUI) GetEditorComponent() interface{}                               { return nil }
func (n *noopUI) Input(string, string, *ExtensionUIDialogOptions) (string, error)    { return "", nil }
func (n *noopUI) Editor(string, string) (string, error)                              { return "", nil }
func (n *noopUI) Notify(string, string)                                              {}
func (n *noopUI) SetStatus(string, string)                                           {}
func (n *noopUI) SetTitle(string)                                                    {}
func (n *noopUI) SetHiddenThinkingLabel(string)                                       {}
func (n *noopUI) SetWorkingMessage(string)                                             {}
func (n *noopUI) SetWorkingVisible(bool)                                               {}
func (n *noopUI) SetWorkingIndicator([]string, int)                                     {}
func (n *noopUI) OnTerminalInput(TerminalInputHandler) func()                            { return func() {} }
func (n *noopUI) PasteToEditor(string)                                                  {}
func (n *noopUI) SetEditorText(string)                                                  {}
func (n *noopUI) GetEditorText() string                                                 { return "" }
func (n *noopUI) SetWidget(string, string, string)                                      {}
func (n *noopUI) Custom(string, string, []CustomButton, *ExtensionUIDialogOptions) (string, error) {
	return "", nil
}

// NoopUI is a no-op ExtensionUI that returns empty/zero values.
var NoopUI ExtensionUI = &noopUI{}

// ---------------------------------------------------------------------------
// Logger — simple leveled logger interface for extensions
// ---------------------------------------------------------------------------

// Logger is the logging interface available to extensions via ExtensionContext.
type Logger interface {
	Info(format string, args ...interface{})
	Warn(format string, args ...interface{})
	Error(format string, args ...interface{})
	Debug(format string, args ...interface{})
}

// StdLogger is a basic Logger implementation that writes to a Printf-style function.
type StdLogger struct {
	Infof  func(format string, args ...interface{})
	Warnf  func(format string, args ...interface{})
	Errorf func(format string, args ...interface{})
	Debugf func(format string, args ...interface{})
}

func (l *StdLogger) Info(format string, args ...interface{})  { l.Infof(format, args...) }
func (l *StdLogger) Warn(format string, args ...interface{})  { l.Warnf(format, args...) }
func (l *StdLogger) Error(format string, args ...interface{}) { l.Errorf(format, args...) }
func (l *StdLogger) Debug(format string, args ...interface{}) { l.Debugf(format, args...) }

// ---------------------------------------------------------------------------
// Event — lightweight event type for the extension event bus
// ---------------------------------------------------------------------------

// Event is a named event with an arbitrary payload.
type Event struct {
	Name string
	Data interface{}
}

// EventBus provides a simple publish/subscribe mechanism for extensions
// to communicate with each other and with the host application.
type EventBus struct {
	subscribers map[string][]chan Event
}

// NewEventBus creates a new EventBus.
func NewEventBus() *EventBus {
	return &EventBus{
		subscribers: make(map[string][]chan Event),
	}
}

// Subscribe registers a channel to receive events with the given name.
// The channel should be buffered to avoid blocking the publisher.
func (eb *EventBus) Subscribe(eventName string, ch chan Event) {
	eb.subscribers[eventName] = append(eb.subscribers[eventName], ch)
}

// Unsubscribe removes a channel from the given event name.
func (eb *EventBus) Unsubscribe(eventName string, ch chan Event) {
	subs := eb.subscribers[eventName]
	for i, s := range subs {
		if s == ch {
			eb.subscribers[eventName] = append(subs[:i], subs[i+1:]...)
			return
		}
	}
}

// Publish sends an event to all subscribers. Non-blocking: if a subscriber's
// buffer is full, the event is dropped for that subscriber.
func (eb *EventBus) Publish(ev Event) {
	for _, ch := range eb.subscribers[ev.Name] {
		select {
		case ch <- ev:
		default:
			// drop if subscriber is not ready
		}
	}
}

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
// Mirrors pi-mono's pi.on("event", handler) pattern.
//
// Supported events: tool_call, tool_result, input, context, before_provider_request,
// before_agent_start, message_end, user_bash, model_select, thinking_level_select,
// session_before_switch, session_before_fork, session_before_compact, session_shutdown.
//
// Returns an unsubscribe function.
func (ctx ExtensionContext) On(event string, handler interface{}) func() {
	if ctx.handlers == nil {
		return func() {} // no-op if no registry
	}
	switch event {
	case "tool_call":
		if h, ok := handler.(ToolCallHandler); ok {
			ctx.handlers.AddToolCallHandler(h)
			return func() {} // TODO: support unregister
		}
	case "tool_result":
		if h, ok := handler.(ToolResultHandler); ok {
			ctx.handlers.AddToolResultHandler(h)
			return func() {}
		}
	case "input":
		if h, ok := handler.(InputHandler); ok {
			ctx.handlers.AddInputHandler(h)
			return func() {}
		}
	case "context":
		if h, ok := handler.(ContextHandler); ok {
			ctx.handlers.AddContextHandler(h)
			return func() {}
		}
	case "before_provider_request":
		if h, ok := handler.(BeforeProviderRequestHandler); ok {
			ctx.handlers.AddBeforeProviderRequestHandler(h)
			return func() {}
		}
	case "before_agent_start":
		if h, ok := handler.(BeforeAgentStartHandler); ok {
			ctx.handlers.AddBeforeAgentStartHandler(h)
			return func() {}
		}
	case "message_end":
		if h, ok := handler.(MessageEndHandler); ok {
			ctx.handlers.AddMessageEndHandler(h)
			return func() {}
		}
	case "user_bash":
		if h, ok := handler.(UserBashHandler); ok {
			ctx.handlers.AddUserBashHandler(h)
			return func() {}
		}
	case "model_select":
		if h, ok := handler.(ModelSelectHandler); ok {
			ctx.handlers.AddModelSelectHandler(h)
			return func() {}
		}
	case "thinking_level_select":
		if h, ok := handler.(ThinkingLevelSelectHandler); ok {
			ctx.handlers.AddThinkingLevelSelectHandler(h)
			return func() {}
		}
	case "session_before_switch":
		if h, ok := handler.(SessionBeforeSwitchHandler); ok {
			ctx.handlers.AddSessionBeforeSwitchHandler(h)
			return func() {}
		}
	case "session_before_fork":
		if h, ok := handler.(SessionBeforeForkHandler); ok {
			ctx.handlers.AddSessionBeforeForkHandler(h)
			return func() {}
		}
	case "session_before_compact":
		if h, ok := handler.(SessionBeforeCompactHandler); ok {
			ctx.handlers.AddSessionBeforeCompactHandler(h)
			return func() {}
		}
	case "session_shutdown":
		if h, ok := handler.(SessionShutdownHandler); ok {
			ctx.handlers.AddSessionShutdownHandler(h)
			return func() {}
		}
	}
	return func() {}
}

// NewExtensionContext creates a new ExtensionContext with the given components.
// The registry is created internally and shared across all extensions.
// ui is optional; pass NoopUI or nil if no interactive UI is available.
func NewExtensionContext(sm *session.Manager, s *settings.Settings, bus *EventBus, logger Logger, cwd string, ui ExtensionUI) ExtensionContext {
	if ui == nil {
		ui = NoopUI
	}
	return ExtensionContext{
		SessionManager: sm,
		Settings:       s,
		EventBus:        bus,
		Logger:          logger,
		CWD:             cwd,
		UI:              ui,
		Actions:         &ExtensionActions{}, // safe no-ops by default
		registry:        globalRegistry,
	}
}

// RegisterTool registers a tool with the extension system. Tools registered
// by extensions are merged with built-in tools at runtime.
func (ctx ExtensionContext) RegisterTool(name string, handler func(args json.RawMessage) (string, error), description string, parameters json.RawMessage) error {
	return ctx.registry.RegisterTool(name, handler, description, parameters)
}

// RegisterSlashCommand registers a slash-command handler (e.g. "/mycommand").
func (ctx ExtensionContext) RegisterSlashCommand(cmd string, handler SlashCommandHandler) error {
	return ctx.registry.RegisterSlashCommand(cmd, handler)
}

// RegisterPrompt registers a named prompt template that can be injected
// into the system prompt at runtime.
func (ctx ExtensionContext) RegisterPrompt(name string, template string) error {
	return ctx.registry.RegisterPrompt(name, template)
}

// RegisterShortcut registers a keyboard shortcut. When the specified key
// combination is pressed, handler is called. In TUI mode, the key is intercepted
// via the terminal input handler so it won't interfere with normal input.
func (ctx ExtensionContext) RegisterShortcut(key string, handler ShortcutHandler, description string) error {
	if err := ctx.registry.RegisterShortcut(key, handler, description); err != nil {
		return err
	}
	// Auto-register terminal input handler to intercept the key
	ctx.UI.OnTerminalInput(func(data string) *TerminalInputResult {
		if data == key {
			handler()
			return &TerminalInputResult{Consume: true}
		}
		return nil
	})
	return nil
}

// RegisterFlag registers a CLI flag for the extension. Other code (e.g. CLI
// argument parsing) can set flag values via SetFlagValue, and extensions read
// them with GetFlag.
func (ctx ExtensionContext) RegisterFlag(name string, description string, flagType FlagType, defaultVal interface{}) error {
	return ctx.registry.RegisterFlag(name, description, flagType, defaultVal)
}

// GetFlag returns the current value of a flag registered by this or another
// extension, or nil if the flag is not found.
func (ctx ExtensionContext) GetFlag(name string) interface{} {
	return ctx.registry.GetFlag(name)
}

// RegisterProvider registers an LLM provider configuration.
// Mirrors pi-mono's registerProvider(name, config).
// Also syncs to the engine-level ModelRegistry if available.
func (ctx ExtensionContext) RegisterProvider(name string, config ProviderConfig) error {
	if ctx.ModelRegistry != nil {
		// Convert extension ProviderModel to modelregistry types
		override := modelregistry.ProviderOverride{
			Name:    config.Name,
			BaseURL: config.BaseURL,
			APIKey:  config.APIKey,
			Headers: config.Headers,
		}
		for _, m := range config.Models {
			override.Models = append(override.Models, types.Model{
				ID:            m.ID,
				Name:          m.Name,
				Provider:      name,
				ContextWindow: m.ContextWindow,
				Reasoning:     m.Reasoning,
			})
		}
		ctx.ModelRegistry.RegisterProvider(name, override)
	}
	return ctx.registry.RegisterProvider(name, config)
}

// UnregisterProvider removes a registered provider.
// Mirrors pi-mono's unregisterProvider(name).
func (ctx ExtensionContext) UnregisterProvider(name string) {
	if ctx.ModelRegistry != nil {
		ctx.ModelRegistry.UnregisterProvider(name)
	}
	ctx.registry.UnregisterProvider(name)
}

// AddAutocompleteProvider registers an autocomplete provider that supplies
// additional completion candidates when the user triggers autocomplete.
func (ctx ExtensionContext) AddAutocompleteProvider(provider AutocompleteProvider) {
	ctx.registry.AddAutocompleteProvider(provider)
}
