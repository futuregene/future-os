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

	// handlers is the typed event handler registry.
	// ExtensionContext.On() delegates to this.
	handlers *HandlerRegistry
}

// NewExtensionRunner creates a new ExtensionRunner with the given context and
// an optional pre-configured logger. If logger is nil, a no-op logger is used.
func NewExtensionRunner(ctx ExtensionContext) *ExtensionRunner {
	logger := ctx.Logger
	if logger == nil {
		logger = &noopLogger{}
	}
	h := NewHandlerRegistry()
	ctx.handlers = h
	return &ExtensionRunner{
		Context:  ctx,
		Logger:   logger,
		handlers: h,
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

// EmitResourcesDiscover emits the resources_discover event.
func (r *ExtensionRunner) EmitResourcesDiscover(cwd string, reason string) {
	r.Context.EventBus.Publish(Event{Name: "resources_discover", Data: map[string]interface{}{
		"cwd":    cwd,
		"reason": reason,
	}})
}

// EmitBeforeAgentStart emits the before_agent_start event and collects handler results.
func (r *ExtensionRunner) EmitBeforeAgentStart(systemPrompt string, userMessage string) (string, string, []string) {
	r.Context.EventBus.Publish(Event{Name: "before_agent_start", Data: map[string]interface{}{
		"systemPrompt": systemPrompt,
		"userMessage":  userMessage,
	}})
	if r.handlers != nil {
		result := r.handlers.InvokeBeforeAgentStart(BeforeAgentStartEvent{
			SystemPrompt: systemPrompt,
			UserMessage:  userMessage,
		})
		if result != nil {
			return result.SystemPrompt, result.Message, nil
		}
	}
	return systemPrompt, userMessage, nil
}

// EmitAgentStart emits the agent_start event.
func (r *ExtensionRunner) EmitAgentStart() {
	r.Context.EventBus.Publish(Event{Name: "agent_start", Data: nil})
}

// EmitAgentEnd emits the agent_end event.
func (r *ExtensionRunner) EmitAgentEnd() {
	r.Context.EventBus.Publish(Event{Name: "agent_end", Data: nil})
}

// EmitTurnStart emits the turn_start event.
func (r *ExtensionRunner) EmitTurnStart(turnIndex int) {
	r.Context.EventBus.Publish(Event{Name: "turn_start", Data: map[string]interface{}{
		"turnIndex": turnIndex,
	}})
}

// EmitTurnEnd emits the turn_end event.
func (r *ExtensionRunner) EmitTurnEnd(turnIndex int) {
	r.Context.EventBus.Publish(Event{Name: "turn_end", Data: map[string]interface{}{
		"turnIndex": turnIndex,
	}})
}

// EmitMessageStart emits the message_start event.
func (r *ExtensionRunner) EmitMessageStart(role string) {
	r.Context.EventBus.Publish(Event{Name: "message_start", Data: map[string]interface{}{
		"role": role,
	}})
}

// EmitMessageEnd emits the message_end event.
func (r *ExtensionRunner) EmitMessageEnd(role string) {
	r.Context.EventBus.Publish(Event{Name: "message_end", Data: map[string]interface{}{
		"role": role,
	}})
}

// EmitToolCall emits the tool_call event and checks for blocks.
func (r *ExtensionRunner) EmitToolCall(toolName string, args interface{}) *ToolCallResult {
	r.Context.EventBus.Publish(Event{Name: "tool_call", Data: map[string]interface{}{
		"tool": toolName,
		"args": args,
	}})
	if r.handlers != nil {
		return r.handlers.InvokeToolCall(ToolCallEvent{ToolName: toolName, Args: args})
	}
	return nil
}

// EmitToolResult emits the tool_result event and collects handler modifications.
func (r *ExtensionRunner) EmitToolResult(toolName string, result string, isError bool) (string, bool) {
	r.Context.EventBus.Publish(Event{Name: "tool_result", Data: map[string]interface{}{
		"tool":    toolName,
		"result":  result,
		"isError": isError,
	}})
	if r.handlers != nil {
		hr := r.handlers.InvokeToolResult(ToolResultEvent{ToolName: toolName, Content: result, IsError: isError})
		if hr != nil {
			return hr.Content, hr.IsError
		}
	}
	return result, isError
}

// EmitInput emits the input event and collects handler transforms.
func (r *ExtensionRunner) EmitInput(text string) (string, InputResultAction) {
	r.Context.EventBus.Publish(Event{Name: "input", Data: map[string]interface{}{
		"text": text,
	}})
	if r.handlers != nil {
		hr := r.handlers.InvokeInput(InputEvent{Text: text, Source: "interactive"})
		if hr != nil {
			return hr.Text, hr.Action
		}
	}
	return text, InputContinue
}

// EmitUserBash emits the user_bash event and collects handler results.
func (r *ExtensionRunner) EmitUserBash(command string, cwd string) *UserBashResult {
	r.Context.EventBus.Publish(Event{Name: "user_bash", Data: map[string]interface{}{
		"command": command,
		"cwd":     cwd,
	}})
	if r.handlers != nil {
		return r.handlers.InvokeUserBash(UserBashEvent{Command: command, CWD: cwd})
	}
	return nil
}

// EmitModelSelect emits the model_select event via both EventBus and handlers.
func (r *ExtensionRunner) EmitModelSelect(model string, previousModel string, source string) {
	r.Context.EventBus.Publish(Event{Name: "model_select", Data: map[string]interface{}{
		"model":         model,
		"previousModel": previousModel,
		"source":        source,
	}})
	if r.handlers != nil {
		r.handlers.InvokeModelSelect(ModelSelectEvent{Model: model, PreviousModel: previousModel, Source: source})
	}
}

// EmitThinkingLevelSelect emits the thinking_level_select event.
func (r *ExtensionRunner) EmitThinkingLevelSelect(level string, previousLevel string) {
	r.Context.EventBus.Publish(Event{Name: "thinking_level_select", Data: map[string]interface{}{
		"level":         level,
		"previousLevel": previousLevel,
	}})
	if r.handlers != nil {
		r.handlers.InvokeThinkingLevelSelect(ThinkingLevelSelectEvent{Level: level, PreviousLevel: previousLevel})
	}
}

// EmitSessionBeforeSwitch emits and checks for cancellation.
func (r *ExtensionRunner) EmitSessionBeforeSwitch(targetSessionFile string) *SessionBeforeSwitchResult {
	r.Context.EventBus.Publish(Event{Name: "session_before_switch", Data: map[string]interface{}{
		"targetSessionFile": targetSessionFile,
	}})
	if r.handlers != nil {
		return r.handlers.InvokeSessionBeforeSwitch(SessionBeforeSwitchEvent{TargetSessionFile: targetSessionFile})
	}
	return nil
}

// EmitSessionBeforeFork emits and checks for cancellation.
func (r *ExtensionRunner) EmitSessionBeforeFork(entryID string) *SessionBeforeForkResult {
	r.Context.EventBus.Publish(Event{Name: "session_before_fork", Data: map[string]interface{}{
		"entryId": entryID,
	}})
	if r.handlers != nil {
		return r.handlers.InvokeSessionBeforeFork(SessionBeforeForkEvent{EntryID: entryID})
	}
	return nil
}

// EmitSessionBeforeCompact emits and checks for cancellation.
func (r *ExtensionRunner) EmitSessionBeforeCompact(customInstructions string) *SessionBeforeCompactResult {
	r.Context.EventBus.Publish(Event{Name: "session_before_compact", Data: map[string]interface{}{
		"customInstructions": customInstructions,
	}})
	if r.handlers != nil {
		return r.handlers.InvokeSessionBeforeCompact(SessionBeforeCompactEvent{CustomInstructions: customInstructions})
	}
	return nil
}

// EmitSessionShutdown emits the session_shutdown event.
func (r *ExtensionRunner) EmitSessionShutdown(reason string) {
	r.Context.EventBus.Publish(Event{Name: "session_shutdown", Data: map[string]interface{}{
		"reason": reason,
	}})
	if r.handlers != nil {
		r.handlers.InvokeSessionShutdown(SessionShutdownEvent{Reason: reason})
	}
}

// EmitSessionStart emits the session_start event.
func (r *ExtensionRunner) EmitSessionStart() {
	r.Context.EventBus.Publish(Event{Name: "session_start", Data: nil})
}

// EmitContext emits the context event before each LLM call.
func (r *ExtensionRunner) EmitContext(messageCount int) {
	r.Context.EventBus.Publish(Event{Name: "context", Data: map[string]interface{}{
		"messageCount": messageCount,
	}})
	if r.handlers != nil {
		r.handlers.InvokeContext(ContextEvent{MessageCount: messageCount})
	}
}

// EmitBeforeProviderRequest emits the before_provider_request event.
func (r *ExtensionRunner) EmitBeforeProviderRequest(payload interface{}) interface{} {
	r.Context.EventBus.Publish(Event{Name: "before_provider_request", Data: map[string]interface{}{
		"payload": payload,
	}})
	if r.handlers != nil {
		result := r.handlers.InvokeBeforeProviderRequest(BeforeProviderRequestEvent{Payload: payload})
		if result != nil {
			return result.Payload
		}
	}
	return payload
}

// EmitAfterProviderResponse emits the after_provider_response event.
func (r *ExtensionRunner) EmitAfterProviderResponse(status int) {
	r.Context.EventBus.Publish(Event{Name: "after_provider_response", Data: map[string]interface{}{
		"status": status,
	}})
}

// EmitToolExecutionStart emits the tool_execution_start event.
func (r *ExtensionRunner) EmitToolExecutionStart(toolCallID string, toolName string, args interface{}) {
	r.Context.EventBus.Publish(Event{Name: "tool_execution_start", Data: map[string]interface{}{
		"toolCallId": toolCallID,
		"toolName":   toolName,
		"args":       args,
	}})
}

// EmitToolExecutionEnd emits the tool_execution_end event.
func (r *ExtensionRunner) EmitToolExecutionEnd(toolCallID string, toolName string, result string, isError bool) {
	r.Context.EventBus.Publish(Event{Name: "tool_execution_end", Data: map[string]interface{}{
		"toolCallId": toolCallID,
		"toolName":   toolName,
		"result":     result,
		"isError":    isError,
	}})
}

// EmitSessionCompact emits the session_compact event.
func (r *ExtensionRunner) EmitSessionCompact(summary string) {
	r.Context.EventBus.Publish(Event{Name: "session_compact", Data: map[string]interface{}{
		"summary": summary,
	}})
}

// EmitMessageUpdate emits the message_update event (token-by-token streaming).
func (r *ExtensionRunner) EmitMessageUpdate(role string) {
	r.Context.EventBus.Publish(Event{Name: "message_update", Data: map[string]interface{}{
		"role": role,
	}})
}

// ---------------------------------------------------------------------------
// Management methods — mirrors pi-mono ExtensionRunner management
// ---------------------------------------------------------------------------

// GetAllRegisteredTools returns all tools registered by extensions.
func (r *ExtensionRunner) GetAllRegisteredTools() []ToolInfo {
	tools := GetAllTools()
	infos := make([]ToolInfo, 0, len(tools))
	for _, t := range tools {
		infos = append(infos, ToolInfo{
			Name:        t.Def.Function.Name,
			Description: t.Def.Function.Description,
		})
	}
	return infos
}

// HasHandlers checks if any extension handles an event type.
func (r *ExtensionRunner) HasHandlers(eventName string) bool {
	// Check via EventBus subscription count
	// This is a heuristic — EventBus doesn't expose count directly
	return true // Extensions may subscribe at runtime; always emit
}

// Invalidate marks the extension runtime as stale after session switch.
func (r *ExtensionRunner) Invalidate(message string) {
	r.Context.Logger.Warn("extension runtime invalidated: %s", message)
	r.Context.EventBus.Publish(Event{Name: "runtime_invalidated", Data: map[string]interface{}{
		"message": message,
	}})
}

// Shutdown requests graceful shutdown of extensions.
func (r *ExtensionRunner) Shutdown() {
	r.EmitSessionShutdown("quit")
}

// OnError registers an extension error listener. Returns unsubscribe function.
func (r *ExtensionRunner) OnError(listener func(err ExtensionDiagnostic)) func() {
	ch := make(chan Event, 64)
	r.Context.EventBus.Subscribe("extension_error", ch)
	go func() {
		for ev := range ch {
			if diag, ok := ev.Data.(ExtensionDiagnostic); ok {
				listener(diag)
			}
		}
	}()
	return func() {
		r.Context.EventBus.Unsubscribe("extension_error", ch)
		close(ch)
	}
}

// EmitExtensionError publishes an extension error event.
func (r *ExtensionRunner) EmitExtensionError(extName string, err error) {
	r.Context.EventBus.Publish(Event{Name: "extension_error", Data: ExtensionDiagnostic{
		Type:    "error",
		Message: err.Error(),
		Path:    extName,
	}})
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
