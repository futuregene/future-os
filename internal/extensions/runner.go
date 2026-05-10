package extensions

import (
	"fmt"
	"sort"
	"strings"
)

// ---------------------------------------------------------------------------
// ExtensionRunner — manages the lifecycle of loaded extensions
// ---------------------------------------------------------------------------

// ExtensionDiagnostic describes an issue found during extension loading/init.
type ExtensionDiagnostic struct {
	Type    string // "error" or "warning"
	Message string
	Path    string
}

// ExtensionRunner manages the lifecycle of extensions: loading, initializing,
// and deinitializing.
type ExtensionRunner struct {
	Extensions []Extension
	Context    ExtensionContext
	Logger     Logger

	// ordered tracks the initialization order for reverse-order shutdown.
	initialized []Extension

	// initErrors tracks init failures for diagnostic reporting.
	initErrors []ExtensionDiagnostic

	// loadErrors tracks load failures for diagnostic reporting.
	loadErrors []ExtensionDiagnostic
}

// NewExtensionRunner creates a new ExtensionRunner with the given context and
// an optional pre-configured logger. If logger is nil, a no-op logger is used.
func NewExtensionRunner(ctx ExtensionContext) *ExtensionRunner {
	logger := ctx.Logger
	if logger == nil {
		logger = &noopLogger{}
	}
	return &ExtensionRunner{
		Context: ctx,
		Logger:  logger,
	}
}

// Load finds and loads extensions from the given paths using LoadExtensions.
// Loaded extensions are stored but NOT initialized yet (call InitAll).
func (r *ExtensionRunner) Load(paths []string) error {
	exts, err := LoadExtensions(paths, r.Logger)
	if err != nil {
		r.loadErrors = append(r.loadErrors, ExtensionDiagnostic{
			Type: "error", Message: err.Error(), Path: strings.Join(paths, ", "),
		})
		return fmt.Errorf("load extensions: %w", err)
	}
	r.Extensions = append(r.Extensions, exts...)
	return nil
}

// Add adds an already-constructed Extension to the runner.
func (r *ExtensionRunner) Add(ext Extension) {
	r.Extensions = append(r.Extensions, ext)
}

// InitAll initializes all loaded extensions in order. Extensions that fail
// initialization are skipped (logged and removed from the active set).
// Returns the first error encountered, but continues initializing remaining
// extensions.
func (r *ExtensionRunner) InitAll() error {
	var firstErr error

	for _, ext := range r.Extensions {
		if err := r.initOne(ext); err != nil {
			if firstErr == nil {
				firstErr = err
			}
		}
	}

	return firstErr
}

// initOne initializes a single extension. On failure, logs and skips.
func (r *ExtensionRunner) initOne(ext Extension) error {
	r.Logger.Info("initializing extension %q...", ext.Name())

	if err := ext.Init(r.Context); err != nil {
		r.Logger.Error("extension %q failed to initialize: %v", ext.Name(), err)
		r.initErrors = append(r.initErrors, ExtensionDiagnostic{
			Type: "error", Message: err.Error(), Path: ext.Name(),
		})
		return fmt.Errorf("extension %q: init: %w", ext.Name(), err)
	}

	r.initialized = append(r.initialized, ext)
	r.Logger.Info("extension %q initialized successfully", ext.Name())
	return nil
}

// GetExtensionDiagnostics returns all extension diagnostics (load + init errors).
func (r *ExtensionRunner) GetExtensionDiagnostics() []ExtensionDiagnostic {
	all := make([]ExtensionDiagnostic, 0, len(r.loadErrors)+len(r.initErrors))
	all = append(all, r.loadErrors...)
	all = append(all, r.initErrors...)
	return all
}

// AddLoadError records a load-time diagnostic.
func (r *ExtensionRunner) AddLoadError(msg, path string) {
	r.loadErrors = append(r.loadErrors, ExtensionDiagnostic{
		Type: "error", Message: msg, Path: path,
	})
}

// DeinitAll deinitializes all successfully initialized extensions in reverse
// order. Errors are logged but do not stop the shutdown process.
func (r *ExtensionRunner) DeinitAll() {
	// Deinit in reverse order
	for i := len(r.initialized) - 1; i >= 0; i-- {
		ext := r.initialized[i]
		r.Logger.Info("deinitializing extension %q...", ext.Name())
		if err := ext.Deinit(); err != nil {
			r.Logger.Error("extension %q deinit error: %v", ext.Name(), err)
		} else {
			r.Logger.Info("extension %q deinitialized", ext.Name())
		}
	}
	r.initialized = nil
}

// Run is a convenience method that loads extensions from paths, then
// initializes them all. Returns the runner for subsequent DeinitAll.
// If loading fails entirely, returns nil and the error.
func Run(paths []string, ctx ExtensionContext) (*ExtensionRunner, error) {
	runner := NewExtensionRunner(ctx)
	if err := runner.Load(paths); err != nil {
		return nil, err
	}
	if err := runner.InitAll(); err != nil {
		// Some extensions failed, but initialized ones are tracked.
		// Return the runner so the caller can still DeinitAll.
		return runner, err
	}
	return runner, nil
}

// RunExtension is a convenience method that adds a single extension and
// initializes it.
func RunExtension(ext Extension, ctx ExtensionContext) error {
	runner := NewExtensionRunner(ctx)
	runner.Add(ext)
	return runner.InitAll()
}

// Initialized returns the list of successfully initialized extensions.
func (r *ExtensionRunner) Initialized() []Extension {
	return r.initialized
}

// ---------------------------------------------------------------------------
// Event emission — mirrors pi-mono ExtensionRunner emit methods
// ---------------------------------------------------------------------------

// EmitBeforeAgentStart emits the before_agent_start event.
// Extensions can modify systemPrompt or inject customMessages.
// Returns the (possibly modified) system prompt and any custom messages to inject.
func (r *ExtensionRunner) EmitBeforeAgentStart(systemPrompt string, userMessage string) (modifiedPrompt string, customMessages []string) {
	modifiedPrompt = systemPrompt
	ev := Event{Name: "before_agent_start", Data: map[string]interface{}{
		"systemPrompt": systemPrompt,
		"userMessage":  userMessage,
	}}
	r.Context.EventBus.Publish(ev)
	return modifiedPrompt, nil
}

// EmitAgentStart emits the agent_start event.
func (r *ExtensionRunner) EmitAgentStart() {
	r.Context.EventBus.Publish(Event{Name: "agent_start", Data: nil})
}

// EmitAgentEnd emits the agent_end event.
func (r *ExtensionRunner) EmitAgentEnd() {
	r.Context.EventBus.Publish(Event{Name: "agent_end", Data: nil})
}

// EmitToolCall emits the tool_call event before a tool executes.
func (r *ExtensionRunner) EmitToolCall(toolName string, args interface{}) {
	r.Context.EventBus.Publish(Event{Name: "tool_call", Data: map[string]interface{}{
		"tool": toolName,
		"args": args,
	}})
}

// EmitToolResult emits the tool_result event after a tool executes.
func (r *ExtensionRunner) EmitToolResult(toolName string, result string, isError bool) {
	r.Context.EventBus.Publish(Event{Name: "tool_result", Data: map[string]interface{}{
		"tool":   toolName,
		"result": result,
		"isError": isError,
	}})
}

// EmitInput emits the input event when user submits a message.
// Extensions can transform the message; the first handler that returns
// non-empty text overrides.
func (r *ExtensionRunner) EmitInput(text string) string {
	ev := Event{Name: "input", Data: map[string]interface{}{
		"text": text,
	}}
	r.Context.EventBus.Publish(ev)
	return text
}

// EmitContext emits the context event before each LLM call.
// Extensions can read/modify context via the EventBus.
func (r *ExtensionRunner) EmitContext(messageCount int) {
	r.Context.EventBus.Publish(Event{Name: "context", Data: map[string]interface{}{
		"messageCount": messageCount,
	}})
}

// EmitSessionStart emits the session_start event.
func (r *ExtensionRunner) EmitSessionStart() {
	r.Context.EventBus.Publish(Event{Name: "session_start", Data: nil})
}

// EmitSessionShutdown emits the session_shutdown event.
func (r *ExtensionRunner) EmitSessionShutdown() {
	r.Context.EventBus.Publish(Event{Name: "session_shutdown", Data: nil})
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

// SortExtensionsByName sorts a slice of extensions by name.
func SortExtensionsByName(exts []Extension) {
	sort.Slice(exts, func(i, j int) bool {
		return exts[i].Name() < exts[j].Name()
	})
}

// ---------------------------------------------------------------------------
// noopLogger — silent logger for when none is provided
// ---------------------------------------------------------------------------

type noopLogger struct{}

func (n *noopLogger) Info(string, ...interface{})  {}
func (n *noopLogger) Warn(string, ...interface{})  {}
func (n *noopLogger) Error(string, ...interface{}) {}
func (n *noopLogger) Debug(string, ...interface{}) {}
