// Package extensions provides a plugin architecture for cobalt.
// Extensions can register tools, slash commands, and prompt templates,
// and are loaded dynamically from Go plugins (.so) or JSON configs.
package extensions

import (
	"encoding/json"

	"github.com/huichen/cobalt/internal/session"
	"github.com/huichen/cobalt/internal/settings"
)

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
// ExtensionContext — the environment passed to extensions at Init time
// ---------------------------------------------------------------------------

// ExtensionContext provides extensions with access to cobalt internals and
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

	// registry is the shared extension registry (set internally).
	registry *Registry
}

// NewExtensionContext creates a new ExtensionContext with the given components.
// The registry is created internally and shared across all extensions.
func NewExtensionContext(sm *session.Manager, s *settings.Settings, bus *EventBus, logger Logger, cwd string) ExtensionContext {
	return ExtensionContext{
		SessionManager: sm,
		Settings:       s,
		EventBus:        bus,
		Logger:          logger,
		CWD:             cwd,
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
