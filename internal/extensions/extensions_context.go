package extensions

import (
	"encoding/json"

	"github.com/huichen/xihu/internal/apiregistry"
	"github.com/huichen/xihu/internal/llm"
	"github.com/huichen/xihu/internal/modelregistry"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/settings"
	"github.com/huichen/xihu/pkg/types"
)

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
// Also syncs to the engine-level ModelRegistry and API registry if available.
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

	// Register with the API registry so the engine can create clients for this
	// provider when it is selected at runtime.
	api := apiregistry.LookupAPI(config.BaseURL)
	apiregistry.RegisterFromExtension(api, func(baseURL, apiKey string, opts *llm.StreamOptions) types.LLMProvider {
		return apiregistry.NewLazyProvider(baseURL, apiKey, opts, func(bu, ak string, o *llm.StreamOptions) types.LLMProvider {
			return llm.NewClient(bu, ak)
		})
	}, name)

	return ctx.registry.RegisterProvider(name, config)
}

// UnregisterProvider removes a registered provider.
// Mirrors pi-mono's unregisterProvider(name).
func (ctx ExtensionContext) UnregisterProvider(name string) {
	if ctx.ModelRegistry != nil {
		ctx.ModelRegistry.UnregisterProvider(name)
	}
	apiregistry.UnregisterFromSource(name)
	ctx.registry.UnregisterProvider(name)
}

// AddAutocompleteProvider registers an autocomplete provider that supplies
// additional completion candidates when the user triggers autocomplete.
func (ctx ExtensionContext) AddAutocompleteProvider(provider AutocompleteProvider) {
	ctx.registry.AddAutocompleteProvider(provider)
}
