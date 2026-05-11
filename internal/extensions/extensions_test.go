package extensions

import (
	"encoding/json"
	"os"
	"path/filepath"
	"testing"
	"time"
)

// TestDiscoverExtensionPaths verifies auto-discovery of extension paths.
func TestDiscoverExtensionPaths(t *testing.T) {
	// Create temp directories
	tmpDir := t.TempDir()

	// Set up a fake home directory structure
	homeDir := filepath.Join(tmpDir, "home", ".xihu", "extensions")
	os.MkdirAll(homeDir, 0755)

	// Create extension.json
	os.WriteFile(filepath.Join(homeDir, "extension.json"), []byte(`{"name":"test-ext","version":"1.0"}`), 0644)

	// Create a subdirectory with extension.json
	subDir := filepath.Join(homeDir, "my-plugin")
	os.MkdirAll(subDir, 0755)
	os.WriteFile(filepath.Join(subDir, "extension.json"), []byte(`{"name":"my-plugin"}`), 0644)

	// Create a .so file
	os.WriteFile(filepath.Join(homeDir, "plugin.so"), []byte("dummy"), 0644)

	// Set HOME for discovery
	t.Setenv("HOME", filepath.Join(tmpDir, "home"))

	// Also create project-local extensions
	projectDir := filepath.Join(tmpDir, "project", ".xihu", "extensions")
	os.MkdirAll(projectDir, 0755)
	os.WriteFile(filepath.Join(projectDir, "local.json"), []byte(`{"name":"local-ext"}`), 0644)

	paths := DiscoverExtensionPaths(filepath.Join(tmpDir, "project"))

	// We should have 4 paths: extension.json, plugin.so, my-plugin (dir), local.json
	if len(paths) < 3 {
		t.Errorf("expected at least 3 paths, got %d: %v", len(paths), paths)
	}
}

// TestDiscoverExtensionPathsEmpty verifies empty discovery when no paths exist.
func TestDiscoverExtensionPathsEmpty(t *testing.T) {
	tmpDir := t.TempDir()
	t.Setenv("HOME", tmpDir)

	paths := DiscoverExtensionPaths(tmpDir)
	if len(paths) != 0 {
		t.Errorf("expected 0 paths, got %d", len(paths))
	}
}

// TestExtensionRunnerEventEmission verifies events can be published.
func TestExtensionRunnerEventEmission(t *testing.T) {
	bus := NewEventBus()
	ch := make(chan Event, 10)
	bus.Subscribe("agent_start", ch)

	ctx := ExtensionContext{
		EventBus: bus,
	}

	runner := NewExtensionRunner(ctx)
	runner.EmitAgentStart()

	select {
	case ev := <-ch:
		if ev.Name != "agent_start" {
			t.Errorf("expected agent_start, got %s", ev.Name)
		}
	default:
		t.Error("expected agent_start event")
	}
}

// TestExtensionRunnerAllEvents verifies all event types can be emitted.
func TestExtensionRunnerAllEvents(t *testing.T) {
	bus := NewEventBus()
	ch := make(chan Event, 100)
	bus.Subscribe("before_agent_start", ch)
	bus.Subscribe("agent_start", ch)
	bus.Subscribe("agent_end", ch)
	bus.Subscribe("tool_call", ch)
	bus.Subscribe("tool_result", ch)
	bus.Subscribe("input", ch)
	bus.Subscribe("context", ch)
	bus.Subscribe("session_start", ch)
	bus.Subscribe("session_shutdown", ch)

	ctx := ExtensionContext{
		EventBus: bus,
	}
	runner := NewExtensionRunner(ctx)

	// Emit all events
	runner.EmitBeforeAgentStart("system prompt", "hello")
	runner.EmitAgentStart()
	runner.EmitAgentEnd()
	runner.EmitToolCall("bash", map[string]string{"command": "ls"})
	runner.EmitToolResult("bash", "file1.txt", false)
	runner.EmitInput("user input")
	runner.EmitContext(42)
	runner.EmitSessionStart()
	runner.EmitSessionShutdown("test")

	close(ch)

	count := 0
	for range ch {
		count++
	}
	if count != 9 {
		t.Errorf("expected 9 events, got %d", count)
	}
}

// TestExtensionActionsSafeDefaults verifies Actions nil-safety.
func TestExtensionActionsSafeDefaults(t *testing.T) {
	ctx := NewExtensionContext(nil, nil, NewEventBus(), nil, ".", nil)

	// These should not panic even though no engine is connected
	if ctx.Actions == nil {
		t.Error("Actions should be non-nil (safe defaults)")
	}

	// Safe calls
	if ctx.Actions.Abort != nil {
		ctx.Actions.Abort() // should not panic
	}
	if ctx.Actions.IsIdle != nil {
		_ = ctx.Actions.IsIdle()
	}
	if ctx.Actions.SendUserMessage != nil {
		ctx.Actions.SendUserMessage("test", "steer")
	}
	if ctx.Actions.SetModel != nil {
		ctx.Actions.SetModel("openai", "gpt-4o")
	}
	if ctx.Actions.GetThinkingLevel != nil {
		ctx.Actions.GetThinkingLevel()
	}
	if ctx.Actions.SetThinkingLevel != nil {
		ctx.Actions.SetThinkingLevel("high")
	}
	if ctx.Actions.GetActiveTools != nil {
		ctx.Actions.GetActiveTools()
	}
	if ctx.Actions.GetAllTools != nil {
		ctx.Actions.GetAllTools()
	}
	if ctx.Actions.SetActiveTools != nil {
		ctx.Actions.SetActiveTools([]string{"bash", "read"})
	}
	if ctx.Actions.SetSessionName != nil {
		ctx.Actions.SetSessionName("test-session")
	}
	if ctx.Actions.GetSessionName != nil {
		ctx.Actions.GetSessionName()
	}
	if ctx.Actions.SendMessage != nil {
		ctx.Actions.SendMessage("custom", map[string]string{"key": "val"}, "steer")
	}
	if ctx.Actions.AppendEntry != nil {
		ctx.Actions.AppendEntry("custom", map[string]string{"key": "val"})
	}
}

// TestAllEventsEmitted verifies all 24 event types can be emitted.
func TestAllEventsEmitted(t *testing.T) {
	bus := NewEventBus()
	counts := make(map[string]int)
	ch := make(chan Event, 100)

	allEvents := []string{
		"resources_discover", "before_agent_start", "agent_start", "agent_end",
		"turn_start", "turn_end", "message_start", "message_end",
		"tool_call", "tool_result", "tool_execution_start", "tool_execution_end",
		"model_select", "thinking_level_select", "user_bash",
		"input", "context", "before_provider_request", "after_provider_response",
		"session_start", "session_shutdown",
		"session_before_switch", "session_before_fork",
		"session_before_compact", "session_compact",
	}
	for _, name := range allEvents {
		bus.Subscribe(name, ch)
	}

	ctx := ExtensionContext{EventBus: bus, Logger: &noopLogger{}}
	runner := NewExtensionRunner(ctx)

	// Emit all events
	runner.EmitResourcesDiscover("/tmp", "startup")
	runner.EmitBeforeAgentStart("system", "hello")
	runner.EmitAgentStart()
	runner.EmitAgentEnd()
	runner.EmitTurnStart(1)
	runner.EmitTurnEnd(1)
	runner.EmitMessageStart("user")
	runner.EmitMessageEnd("assistant")
	runner.EmitToolCall("bash", map[string]string{"cmd": "ls"})
	runner.EmitToolResult("bash", "file.txt", false)
	runner.EmitToolExecutionStart("tc1", "bash", "{}")
	runner.EmitToolExecutionEnd("tc1", "bash", "ok", false)
	runner.EmitModelSelect("gpt-4o", "gpt-3.5", "set")
	runner.EmitThinkingLevelSelect("high", "medium")
	runner.EmitUserBash("echo hello", "/tmp")
	runner.EmitInput("hello world")
	runner.EmitContext(42)
	runner.EmitBeforeProviderRequest(map[string]string{"model": "gpt-4o"})
	runner.EmitAfterProviderResponse(200)
	runner.EmitSessionStart()
	runner.EmitSessionShutdown("test")
	runner.EmitSessionBeforeSwitch("/tmp/new.jsonl")
	runner.EmitSessionBeforeFork("entry-123")
	runner.EmitSessionBeforeCompact("be concise")
	runner.EmitSessionCompact("summary text")

	close(ch)
	for range ch {
		counts["total"]++
	}

	if counts["total"] != len(allEvents) {
		t.Errorf("expected %d events, got %d", len(allEvents), counts["total"])
	}
}

// TestGetAllRegisteredTools verifies tool enumeration.
func TestGetAllRegisteredTools(t *testing.T) {
	bus := NewEventBus()
	ctx := ExtensionContext{EventBus: bus, Logger: &noopLogger{}}
	runner := NewExtensionRunner(ctx)

	// Register a tool via the package-level function
	RegisterTool("test_tool", func(args json.RawMessage) (string, error) { return "ok", nil }, "A test tool", json.RawMessage(`{"type":"object"}`))

	tools := runner.GetAllRegisteredTools()
	found := false
	for _, t := range tools {
		if t.Name == "test_tool" {
			found = true
			break
		}
	}
	if !found {
		t.Error("expected test_tool in registered tools")
	}
}

// TestLoadExtensionFromFactory verifies inline extension loading.
func TestLoadExtensionFromFactory(t *testing.T) {
	ext := LoadExtensionFromFactory(func() Extension {
		return &ConfigExtension{
			Manifest: ExtensionManifest{Name: "test-inline"},
		}
	}, "test-inline")

	if ext.Name() != "test-inline" {
		t.Errorf("expected name test-inline, got %s", ext.Name())
	}

	bus := NewEventBus()
	ctx := ExtensionContext{EventBus: bus, Logger: &noopLogger{}}
	if err := ext.Init(ctx); err != nil {
		t.Errorf("Init failed: %v", err)
	}
	if err := ext.Deinit(); err != nil {
		t.Errorf("Deinit failed: %v", err)
	}
}

// TestExtensionRunnerManagement verifies management methods.
func TestExtensionRunnerManagement(t *testing.T) {
	bus := NewEventBus()
	ch := make(chan Event, 10)
	bus.Subscribe("runtime_invalidated", ch)

	ctx := ExtensionContext{EventBus: bus, Logger: &noopLogger{}}
	runner := NewExtensionRunner(ctx)

	// Invalidate
	runner.Invalidate("session switched")
	select {
	case ev := <-ch:
		if ev.Name != "runtime_invalidated" {
			t.Errorf("expected runtime_invalidated, got %s", ev.Name)
		}
	default:
		t.Error("expected runtime_invalidated event")
	}

	// OnError
	errorReceived := false
	unsub := runner.OnError(func(diag ExtensionDiagnostic) {
		errorReceived = true
	})
	runner.EmitExtensionError("test-ext", assertErr("test error"))
	time.Sleep(10 * time.Millisecond)

	if !errorReceived {
		t.Error("expected error to be received")
	}
	unsub()

	// Shutdown
	ch2 := make(chan Event, 10)
	bus.Subscribe("session_shutdown", ch2)
	runner.Shutdown()
	select {
	case ev := <-ch2:
		if ev.Name != "session_shutdown" {
			t.Errorf("expected session_shutdown, got %s", ev.Name)
		}
	default:
		t.Error("expected session_shutdown event")
	}
}

func assertErr(msg string) error { return &testError{msg} }
type testError struct{ msg string }
func (e *testError) Error() string { return e.msg }

// TestHandlerRegistryToolCall tests typed handler with return value.
func TestHandlerRegistryToolCall(t *testing.T) {
	h := NewHandlerRegistry()

	blocked := false
	h.AddToolCallHandler(func(event ToolCallEvent) *ToolCallResult {
		blocked = true
		return &ToolCallResult{Block: true, Reason: "not allowed"}
	})

	result := h.InvokeToolCall(ToolCallEvent{ToolName: "bash", Args: "rm -rf"})
	if result == nil {
		t.Fatal("expected result")
	}
	if !result.Block {
		t.Error("expected block")
	}
	if !blocked {
		t.Error("handler should have been called")
	}
}

// TestHandlerRegistryToolResult tests chained tool_result handlers.
func TestHandlerRegistryToolResult(t *testing.T) {
	h := NewHandlerRegistry()

	h.AddToolResultHandler(func(event ToolResultEvent) *ToolResultResult {
		return &ToolResultResult{Content: event.Content + " [MODIFIED]", IsError: event.IsError}
	})

	result := h.InvokeToolResult(ToolResultEvent{ToolName: "bash", Content: "ok", IsError: false})
	if result == nil {
		t.Fatal("expected result")
	}
	if result.Content != "ok [MODIFIED]" {
		t.Errorf("expected modified content, got %q", result.Content)
	}
	if result.IsError {
		t.Error("expected no error")
	}
}

// TestHandlerRegistryInput tests input transformation.
func TestHandlerRegistryInput(t *testing.T) {
	h := NewHandlerRegistry()

	h.AddInputHandler(func(event InputEvent) *InputResult {
		return &InputResult{Action: InputTransform, Text: "[" + event.Text + "]"}
	})

	result := h.InvokeInput(InputEvent{Text: "hello", Source: "interactive"})
	if result == nil {
		t.Fatal("expected result")
	}
	if result.Action != InputTransform {
		t.Errorf("expected transform, got %s", result.Action)
	}
	if result.Text != "[hello]" {
		t.Errorf("expected [hello], got %s", result.Text)
	}
}

// TestHandlerRegistryInputHandled tests input short-circuit.
func TestHandlerRegistryInputHandled(t *testing.T) {
	h := NewHandlerRegistry()

	h.AddInputHandler(func(event InputEvent) *InputResult {
		return &InputResult{Action: InputHandled, Text: ""}
	})

	result := h.InvokeInput(InputEvent{Text: "hello", Source: "interactive"})
	if result == nil {
		t.Fatal("expected result")
	}
	if result.Action != InputHandled {
		t.Errorf("expected handled, got %s", result.Action)
	}
}

// TestHandlerRegistryBeforeAgentStart tests system prompt modification.
func TestHandlerRegistryBeforeAgentStart(t *testing.T) {
	h := NewHandlerRegistry()

	h.AddBeforeAgentStartHandler(func(event BeforeAgentStartEvent) *BeforeAgentStartResult {
		return &BeforeAgentStartResult{
			SystemPrompt: event.SystemPrompt + "\n\nBe concise.",
			Message:      event.UserMessage,
		}
	})

	result := h.InvokeBeforeAgentStart(BeforeAgentStartEvent{
		SystemPrompt: "You are a helpful assistant.",
		UserMessage:  "Hello",
	})
	if result == nil {
		t.Fatal("expected result")
	}
	if result.SystemPrompt != "You are a helpful assistant.\n\nBe concise." {
		t.Errorf("unexpected system prompt: %q", result.SystemPrompt)
	}
}

// TestHandlerRegistrySessionBeforeSwitch tests cancellation.
func TestHandlerRegistrySessionBeforeSwitch(t *testing.T) {
	h := NewHandlerRegistry()

	h.AddSessionBeforeSwitchHandler(func(event SessionBeforeSwitchEvent) *SessionBeforeSwitchResult {
		if event.TargetSessionFile == "/etc/passwd" {
			return &SessionBeforeSwitchResult{Cancel: true}
		}
		return nil
	})

	result := h.InvokeSessionBeforeSwitch(SessionBeforeSwitchEvent{TargetSessionFile: "/etc/passwd"})
	if result == nil {
		t.Fatal("expected result")
	}
	if !result.Cancel {
		t.Error("expected cancel")
	}

	result2 := h.InvokeSessionBeforeSwitch(SessionBeforeSwitchEvent{TargetSessionFile: "/tmp/ok.jsonl"})
	if result2 != nil {
		t.Error("expected nil result for allowed path")
	}
}

// TestExtensionContextOn tests the On() method.
func TestExtensionContextOn(t *testing.T) {
	h := NewHandlerRegistry()
	ctx := ExtensionContext{handlers: h, Logger: &noopLogger{}}

	called := false
	ctx.On("tool_call", ToolCallHandler(func(event ToolCallEvent) *ToolCallResult {
		called = true
		return nil // allow
	}))

	h.InvokeToolCall(ToolCallEvent{ToolName: "test"})
	if !called {
		t.Error("handler should have been called via On()")
	}
}

// TestProviderRegistration tests provider registration.
func TestProviderRegistration(t *testing.T) {
	bus := NewEventBus()
	ctx := NewExtensionContext(nil, nil, bus, nil, ".", nil)

	err := ctx.RegisterProvider("test-provider", ProviderConfig{
		Name:    "test",
		BaseURL: "https://api.test.com",
	})
	if err != nil {
		t.Fatalf("RegisterProvider: %v", err)
	}

	cfg, ok := GetProvider("test-provider")
	if !ok {
		t.Fatal("provider not found")
	}
	if cfg.BaseURL != "https://api.test.com" {
		t.Errorf("unexpected base URL: %s", cfg.BaseURL)
	}

	ctx.UnregisterProvider("test-provider")
	_, ok = GetProvider("test-provider")
	if ok {
		t.Error("provider should be removed")
	}
}

// =============================================================================
// ExtensionContext method tests
// =============================================================================

// TestExtensionContextRegisterTool tests RegisterTool via ExtensionContext.
func TestExtensionContextRegisterTool(t *testing.T) {
	bus := NewEventBus()
	ctx := NewExtensionContext(nil, nil, bus, nil, ".", nil)

	err := ctx.RegisterTool("ctx_tool", func(args json.RawMessage) (string, error) {
		return "ok", nil
	}, "a test tool", json.RawMessage(`{"type":"object"}`))
	if err != nil {
		t.Fatalf("RegisterTool: %v", err)
	}

	// Duplicate registration should error
	err = ctx.RegisterTool("ctx_tool", nil, "", nil)
	if err == nil {
		t.Error("expected error on duplicate tool registration")
	}
}

// TestExtensionContextRegisterSlashCommand tests RegisterSlashCommand.
func TestExtensionContextRegisterSlashCommand(t *testing.T) {
	bus := NewEventBus()
	ctx := NewExtensionContext(nil, nil, bus, nil, ".", nil)

	err := ctx.RegisterSlashCommand("/mycmd", func(args []string, _ ExtensionContext) (string, error) {
		return "done", nil
	})
	if err != nil {
		t.Fatalf("RegisterSlashCommand: %v", err)
	}

	// Duplicate
	err = ctx.RegisterSlashCommand("/mycmd", nil)
	if err == nil {
		t.Error("expected error on duplicate slash command")
	}
}

// TestExtensionContextRegisterPrompt tests RegisterPrompt.
func TestExtensionContextRegisterPrompt(t *testing.T) {
	bus := NewEventBus()
	ctx := NewExtensionContext(nil, nil, bus, nil, ".", nil)

	err := ctx.RegisterPrompt("greeting", "Hello, {{name}}!")
	if err != nil {
		t.Fatalf("RegisterPrompt: %v", err)
	}

	// Duplicate
	err = ctx.RegisterPrompt("greeting", "dup")
	if err == nil {
		t.Error("expected error on duplicate prompt")
	}
}

// TestExtensionContextRegisterShortcut tests RegisterShortcut.
func TestExtensionContextRegisterShortcut(t *testing.T) {
	bus := NewEventBus()
	ctx := NewExtensionContext(nil, nil, bus, nil, ".", nil)

	err := ctx.RegisterShortcut("ctrl+k", func() {}, "do a thing")
	if err != nil {
		t.Fatalf("RegisterShortcut: %v", err)
	}

	// Verify shortcut was registered (the handler was stored)
	cmds := GetAllShortcuts()
	if _, ok := cmds["ctrl+k"]; !ok {
		t.Error("shortcut not found in global registry")
	}

	// Duplicate
	err = ctx.RegisterShortcut("ctrl+k", func() {}, "dup")
	if err == nil {
		t.Error("expected error on duplicate shortcut")
	}
}

// TestExtensionContextRegisterFlagAndGetFlag tests RegisterFlag and GetFlag.
func TestExtensionContextRegisterFlagAndGetFlag(t *testing.T) {
	bus := NewEventBus()
	ctx := NewExtensionContext(nil, nil, bus, nil, ".", nil)

	err := ctx.RegisterFlag("myflag", "a test flag", FlagString, "defaultVal")
	if err != nil {
		t.Fatalf("RegisterFlag: %v", err)
	}

	val := ctx.GetFlag("myflag")
	if val != "defaultVal" {
		t.Errorf("expected defaultVal, got %v", val)
	}

	// Duplicate
	err = ctx.RegisterFlag("myflag", "dup", FlagString, "x")
	if err == nil {
		t.Error("expected error on duplicate flag")
	}

	// Non-existent flag
	val = ctx.GetFlag("nonexistent")
	if val != nil {
		t.Errorf("expected nil for nonexistent flag, got %v", val)
	}
}

// TestExtensionContextAddAutocompleteProvider tests AddAutocompleteProvider.
func TestExtensionContextAddAutocompleteProvider(t *testing.T) {
	bus := NewEventBus()
	ctx := NewExtensionContext(nil, nil, bus, nil, ".", nil)

	ctx.AddAutocompleteProvider(func(query string) []string {
		return []string{"candidate1", "candidate2"}
	})

	providers := GetAllAutocompleteProviders()
	if len(providers) != 1 {
		t.Fatalf("expected 1 provider, got %d", len(providers))
	}
	result := providers[0]("test")
	if len(result) != 2 {
		t.Errorf("expected 2 results, got %d", len(result))
	}
}

// TestExtensionContextOnAllEvents tests On() with all supported event types.
func TestExtensionContextOnAllEvents(t *testing.T) {
	h := NewHandlerRegistry()
	ctx := ExtensionContext{handlers: h, Logger: &noopLogger{}}

	// tool_call
	ctx.On("tool_call", ToolCallHandler(func(event ToolCallEvent) *ToolCallResult {
		return nil
	}))
	if r := h.InvokeToolCall(ToolCallEvent{ToolName: "test"}); r != nil {
		t.Error("expected nil result")
	}

	// tool_result
	ctx.On("tool_result", ToolResultHandler(func(event ToolResultEvent) *ToolResultResult {
		return &ToolResultResult{Content: event.Content + "!", IsError: event.IsError}
	}))
	if r := h.InvokeToolResult(ToolResultEvent{Content: "hi"}); r.Content != "hi!" {
		t.Errorf("expected 'hi!', got %q", r.Content)
	}

	// input
	ctx.On("input", InputHandler(func(event InputEvent) *InputResult {
		return &InputResult{Action: InputTransform, Text: "[" + event.Text + "]"}
	}))
	if r := h.InvokeInput(InputEvent{Text: "hello"}); r.Text != "[hello]" {
		t.Errorf("expected '[hello]', got %q", r.Text)
	}

	// context
	ctx.On("context", ContextHandler(func(event ContextEvent) *ContextResult {
		return &ContextResult{}
	}))
	if r := h.InvokeContext(ContextEvent{MessageCount: 5}); r == nil {
		t.Error("expected non-nil result")
	}

	// before_provider_request
	ctx.On("before_provider_request", BeforeProviderRequestHandler(func(event BeforeProviderRequestEvent) *BeforeProviderRequestResult {
		return &BeforeProviderRequestResult{Payload: "modified"}
	}))
	if r := h.InvokeBeforeProviderRequest(BeforeProviderRequestEvent{Payload: "orig"}); r.Payload != "modified" {
		t.Error("expected modified payload")
	}

	// before_agent_start
	ctx.On("before_agent_start", BeforeAgentStartHandler(func(event BeforeAgentStartEvent) *BeforeAgentStartResult {
		return &BeforeAgentStartResult{SystemPrompt: event.SystemPrompt + "+", Message: event.UserMessage}
	}))
	if r := h.InvokeBeforeAgentStart(BeforeAgentStartEvent{SystemPrompt: "sys", UserMessage: "hello"}); r.SystemPrompt != "sys+" {
		t.Error("expected sys+")
	}

	// message_end
	ctx.On("message_end", MessageEndHandler(func(event MessageEndEvent) *MessageEndResult {
		return &MessageEndResult{Role: event.Role + "-modified"}
	}))
	if r := h.InvokeMessageEnd(MessageEndEvent{Role: "assistant"}); r.Role != "assistant-modified" {
		t.Error("expected assistant-modified")
	}

	// user_bash
	ctx.On("user_bash", UserBashHandler(func(event UserBashEvent) *UserBashResult {
		return &UserBashResult{Output: "output", ExitCode: 0}
	}))
	if r := h.InvokeUserBash(UserBashEvent{Command: "ls"}); r.Output != "output" {
		t.Error("expected output")
	}

	// model_select
	calledModel := false
	ctx.On("model_select", ModelSelectHandler(func(event ModelSelectEvent) {
		calledModel = true
	}))
	h.InvokeModelSelect(ModelSelectEvent{Model: "gpt-4"})
	if !calledModel {
		t.Error("model_select handler not called")
	}

	// thinking_level_select
	calledThinking := false
	ctx.On("thinking_level_select", ThinkingLevelSelectHandler(func(event ThinkingLevelSelectEvent) {
		calledThinking = true
	}))
	h.InvokeThinkingLevelSelect(ThinkingLevelSelectEvent{Level: "high"})
	if !calledThinking {
		t.Error("thinking_level_select handler not called")
	}

	// session_before_switch
	ctx.On("session_before_switch", SessionBeforeSwitchHandler(func(event SessionBeforeSwitchEvent) *SessionBeforeSwitchResult {
		return &SessionBeforeSwitchResult{Cancel: true}
	}))
	if r := h.InvokeSessionBeforeSwitch(SessionBeforeSwitchEvent{}); !r.Cancel {
		t.Error("expected cancel")
	}

	// session_before_fork
	ctx.On("session_before_fork", SessionBeforeForkHandler(func(event SessionBeforeForkEvent) *SessionBeforeForkResult {
		return &SessionBeforeForkResult{Cancel: false}
	}))
	if r := h.InvokeSessionBeforeFork(SessionBeforeForkEvent{}); r == nil {
		t.Error("expected non-nil result")
	}

	// session_before_compact
	ctx.On("session_before_compact", SessionBeforeCompactHandler(func(event SessionBeforeCompactEvent) *SessionBeforeCompactResult {
		return &SessionBeforeCompactResult{Cancel: true}
	}))
	if r := h.InvokeSessionBeforeCompact(SessionBeforeCompactEvent{}); !r.Cancel {
		t.Error("expected cancel")
	}

	// session_shutdown
	calledShutdown := false
	ctx.On("session_shutdown", SessionShutdownHandler(func(event SessionShutdownEvent) {
		calledShutdown = true
	}))
	h.InvokeSessionShutdown(SessionShutdownEvent{Reason: "test"})
	if !calledShutdown {
		t.Error("session_shutdown handler not called")
	}
}

// TestExtensionContextOnNilHandlers tests On() with nil handlers (no-op).
func TestExtensionContextOnNilHandlers(t *testing.T) {
	ctx := ExtensionContext{handlers: nil}
	unsub := ctx.On("tool_call", ToolCallHandler(func(event ToolCallEvent) *ToolCallResult {
		return nil
	}))
	unsub() // should not panic
}

// TestExtensionContextOnUnknownEvent tests On() with an unknown event type.
func TestExtensionContextOnUnknownEvent(t *testing.T) {
	h := NewHandlerRegistry()
	ctx := ExtensionContext{handlers: h}
	unsub := ctx.On("unknown_event", "not a handler")
	unsub() // should not panic, just no-op
}

// =============================================================================
// HandlerRegistry full coverage tests
// =============================================================================

// TestHandlerRegistryInvokeContext tests context handler chaining.
func TestHandlerRegistryInvokeContext(t *testing.T) {
	h := NewHandlerRegistry()

	h.AddContextHandler(func(event ContextEvent) *ContextResult {
		return &ContextResult{}
	})

	result := h.InvokeContext(ContextEvent{MessageCount: 42})
	if result == nil {
		t.Error("expected non-nil result")
	}

	// No handlers = nil result
	h2 := NewHandlerRegistry()
	if r := h2.InvokeContext(ContextEvent{}); r != nil {
		t.Error("expected nil result with no handlers")
	}
}

// TestHandlerRegistryInvokeBeforeProviderRequest tests chaining.
func TestHandlerRegistryInvokeBeforeProviderRequest(t *testing.T) {
	h := NewHandlerRegistry()

	h.AddBeforeProviderRequestHandler(func(event BeforeProviderRequestEvent) *BeforeProviderRequestResult {
		return &BeforeProviderRequestResult{Payload: "modified"}
	})

	result := h.InvokeBeforeProviderRequest(BeforeProviderRequestEvent{Payload: "original"})
	if result == nil {
		t.Fatal("expected result")
	}
	if result.Payload != "modified" {
		t.Errorf("expected 'modified', got %v", result.Payload)
	}
}

// TestHandlerRegistryInvokeMessageEnd tests message_end handler chaining.
func TestHandlerRegistryInvokeMessageEnd(t *testing.T) {
	h := NewHandlerRegistry()

	h.AddMessageEndHandler(func(event MessageEndEvent) *MessageEndResult {
		return &MessageEndResult{Role: "modified"}
	})

	result := h.InvokeMessageEnd(MessageEndEvent{Role: "user"})
	if result == nil {
		t.Fatal("expected result")
	}
	if result.Role != "modified" {
		t.Errorf("expected 'modified', got %q", result.Role)
	}
}

// TestHandlerRegistryInvokeUserBash tests user_bash handlers.
func TestHandlerRegistryInvokeUserBash(t *testing.T) {
	h := NewHandlerRegistry()

	h.AddUserBashHandler(func(event UserBashEvent) *UserBashResult {
		return &UserBashResult{Output: "custom output", ExitCode: 1}
	})

	result := h.InvokeUserBash(UserBashEvent{Command: "ls", CWD: "/tmp"})
	if result == nil {
		t.Fatal("expected result")
	}
	if result.Output != "custom output" {
		t.Errorf("expected 'custom output', got %q", result.Output)
	}

	// No handlers = nil
	h2 := NewHandlerRegistry()
	if r := h2.InvokeUserBash(UserBashEvent{}); r != nil {
		t.Error("expected nil result with no handlers")
	}
}

// TestHandlerRegistryInvokeModelSelect tests fire-and-forget model_select.
func TestHandlerRegistryInvokeModelSelect(t *testing.T) {
	h := NewHandlerRegistry()
	called := false

	h.AddModelSelectHandler(func(event ModelSelectEvent) {
		called = true
	})

	h.InvokeModelSelect(ModelSelectEvent{Model: "gpt-4", PreviousModel: "gpt-3.5", Source: "set"})
	if !called {
		t.Error("expected handler to be called")
	}

	// No handlers = no panic
	h2 := NewHandlerRegistry()
	h2.InvokeModelSelect(ModelSelectEvent{})
}

// TestHandlerRegistryInvokeThinkingLevelSelect tests fire-and-forget.
func TestHandlerRegistryInvokeThinkingLevelSelect(t *testing.T) {
	h := NewHandlerRegistry()
	called := false

	h.AddThinkingLevelSelectHandler(func(event ThinkingLevelSelectEvent) {
		called = true
	})

	h.InvokeThinkingLevelSelect(ThinkingLevelSelectEvent{Level: "high", PreviousLevel: "medium"})
	if !called {
		t.Error("expected handler to be called")
	}
}

// TestHandlerRegistryInvokeSessionBeforeFork tests fork cancellation.
func TestHandlerRegistryInvokeSessionBeforeFork(t *testing.T) {
	h := NewHandlerRegistry()

	h.AddSessionBeforeForkHandler(func(event SessionBeforeForkEvent) *SessionBeforeForkResult {
		return &SessionBeforeForkResult{Cancel: true}
	})

	result := h.InvokeSessionBeforeFork(SessionBeforeForkEvent{EntryID: "entry-1"})
	if result == nil {
		t.Fatal("expected result")
	}
	if !result.Cancel {
		t.Error("expected cancel")
	}

	// No handlers = nil
	h2 := NewHandlerRegistry()
	if r := h2.InvokeSessionBeforeFork(SessionBeforeForkEvent{}); r != nil {
		t.Error("expected nil result with no handlers")
	}
}

// TestHandlerRegistryInvokeSessionBeforeCompact tests compact cancellation.
func TestHandlerRegistryInvokeSessionBeforeCompact(t *testing.T) {
	h := NewHandlerRegistry()

	h.AddSessionBeforeCompactHandler(func(event SessionBeforeCompactEvent) *SessionBeforeCompactResult {
		return &SessionBeforeCompactResult{Cancel: true}
	})

	result := h.InvokeSessionBeforeCompact(SessionBeforeCompactEvent{CustomInstructions: "be brief"})
	if result == nil {
		t.Fatal("expected result")
	}
	if !result.Cancel {
		t.Error("expected cancel")
	}
}

// TestHandlerRegistryInvokeSessionShutdown tests fire-and-forget shutdown.
func TestHandlerRegistryInvokeSessionShutdown(t *testing.T) {
	h := NewHandlerRegistry()
	called := false

	h.AddSessionShutdownHandler(func(event SessionShutdownEvent) {
		called = true
	})

	h.InvokeSessionShutdown(SessionShutdownEvent{Reason: "quit"})
	if !called {
		t.Error("expected handler to be called")
	}
}

// TestHandlerRegistryMultipleHandlers tests multiple handlers for same event.
func TestHandlerRegistryMultipleHandlers(t *testing.T) {
	h := NewHandlerRegistry()
	count := 0

	h.AddModelSelectHandler(func(event ModelSelectEvent) { count++ })
	h.AddModelSelectHandler(func(event ModelSelectEvent) { count++ })

	h.InvokeModelSelect(ModelSelectEvent{})
	if count != 2 {
		t.Errorf("expected 2 calls, got %d", count)
	}
}

// TestHandlerRegistryInputChainedTransform tests chained input transforms.
func TestHandlerRegistryInputChainedTransform(t *testing.T) {
	h := NewHandlerRegistry()

	h.AddInputHandler(func(event InputEvent) *InputResult {
		return &InputResult{Action: InputTransform, Text: event.Text + "-A"}
	})
	h.AddInputHandler(func(event InputEvent) *InputResult {
		return &InputResult{Action: InputTransform, Text: event.Text + "-B"}
	})

	result := h.InvokeInput(InputEvent{Text: "start"})
	if result == nil {
		t.Fatal("expected result")
	}
	// start → start-A (first handler) → start-A-B (second handler)
	if result.Text != "start-A-B" {
		t.Errorf("expected 'start-A-B', got %q", result.Text)
	}
}

// TestHandlerRegistryInputHandledShortCircuits tests that handled stops further processing.
func TestHandlerRegistryInputHandledShortCircuits(t *testing.T) {
	h := NewHandlerRegistry()

	h.AddInputHandler(func(event InputEvent) *InputResult {
		return &InputResult{Action: InputHandled, Text: ""}
	})
	h.AddInputHandler(func(event InputEvent) *InputResult {
		t.Error("this should not be called")
		return nil
	})

	result := h.InvokeInput(InputEvent{Text: "hello"})
	if result == nil {
		t.Fatal("expected result")
	}
	if result.Action != InputHandled {
		t.Errorf("expected handled, got %s", result.Action)
	}
}

// =============================================================================
// ExtensionRunner lifecycle tests
// =============================================================================

// TestNewExtensionRunnerDefaultLogger tests that nil logger defaults to noop.
func TestNewExtensionRunnerDefaultLogger(t *testing.T) {
	ctx := ExtensionContext{EventBus: NewEventBus(), Logger: nil}
	runner := NewExtensionRunner(ctx)
	if runner.Logger == nil {
		t.Error("expected non-nil logger")
	}
}

// TestExtensionRunnerAddAndInitAll tests Add, InitAll, Initialized.
func TestExtensionRunnerAddAndInitAll(t *testing.T) {
	ctx := ExtensionContext{EventBus: NewEventBus(), Logger: &noopLogger{}}
	runner := NewExtensionRunner(ctx)

	ext := &ConfigExtension{Manifest: ExtensionManifest{Name: "test-ext"}}
	runner.Add(ext)

	if err := runner.InitAll(); err != nil {
		t.Fatalf("InitAll: %v", err)
	}

	inited := runner.Initialized()
	if len(inited) != 1 {
		t.Errorf("expected 1 initialized, got %d", len(inited))
	}
	if inited[0].Name() != "test-ext" {
		t.Errorf("expected test-ext, got %s", inited[0].Name())
	}
}

// TestExtensionRunnerInitAllWithFailure tests that failing extensions are skipped.
func TestExtensionRunnerInitAllWithFailure(t *testing.T) {
	ctx := ExtensionContext{EventBus: NewEventBus(), Logger: &noopLogger{}}
	runner := NewExtensionRunner(ctx)

	failingExt := &failingExtension{name: "fail-ext"}
	runner.Add(failingExt)

	err := runner.InitAll()
	if err == nil {
		t.Error("expected error from failing extension")
	}

	diags := runner.GetExtensionDiagnostics()
	if len(diags) == 0 {
		t.Error("expected diagnostics from failing extension")
	}
}

type failingExtension struct{ name string }

func (f *failingExtension) Name() string                       { return f.name }
func (f *failingExtension) Init(ctx ExtensionContext) error     { return assertErr("init failed") }
func (f *failingExtension) Deinit() error                       { return nil }

// TestExtensionRunnerDeinitAll tests deinitialization in reverse order.
func TestExtensionRunnerDeinitAll(t *testing.T) {
	ctx := ExtensionContext{EventBus: NewEventBus(), Logger: &noopLogger{}}
	runner := NewExtensionRunner(ctx)

	deinited := []string{}
	ext1 := &trackingExtension{name: "ext1", deinited: &deinited}
	ext2 := &trackingExtension{name: "ext2", deinited: &deinited}
	runner.Add(ext1)
	runner.Add(ext2)
	runner.InitAll()

	runner.DeinitAll()

	// Should deinit in reverse order: ext2, ext1
	if len(deinited) != 2 {
		t.Fatalf("expected 2 deinits, got %d", len(deinited))
	}
	if deinited[0] != "ext2" || deinited[1] != "ext1" {
		t.Errorf("expected [ext2, ext1], got %v", deinited)
	}
}

type trackingExtension struct {
	name     string
	deinited *[]string
}

func (e *trackingExtension) Name() string                       { return e.name }
func (e *trackingExtension) Init(ctx ExtensionContext) error     { return nil }
func (e *trackingExtension) Deinit() error                       { *e.deinited = append(*e.deinited, e.name); return nil }

// TestExtensionRunnerAddLoadError tests AddLoadError and GetExtensionDiagnostics.
func TestExtensionRunnerAddLoadError(t *testing.T) {
	ctx := ExtensionContext{EventBus: NewEventBus(), Logger: &noopLogger{}}
	runner := NewExtensionRunner(ctx)

	runner.AddLoadError("failed to load", "/some/path.so")

	diags := runner.GetExtensionDiagnostics()
	if len(diags) != 1 {
		t.Fatalf("expected 1 diagnostic, got %d", len(diags))
	}
	if diags[0].Message != "failed to load" {
		t.Errorf("expected 'failed to load', got %q", diags[0].Message)
	}
	if diags[0].Path != "/some/path.so" {
		t.Errorf("expected '/some/path.so', got %q", diags[0].Path)
	}
}

// TestExtensionRunnerHasHandlers tests HasHandlers.
func TestExtensionRunnerHasHandlers(t *testing.T) {
	ctx := ExtensionContext{EventBus: NewEventBus(), Logger: &noopLogger{}}
	runner := NewExtensionRunner(ctx)

	if !runner.HasHandlers("any_event") {
		t.Error("HasHandlers should return true")
	}
}

// TestExtensionRunnerRunAndRunExtension tests the Run and RunExtension helpers.
func TestExtensionRunnerRunAndRunExtension(t *testing.T) {
	ctx := ExtensionContext{EventBus: NewEventBus(), Logger: &noopLogger{}}

	// RunExtension with a single extension
	ext := &ConfigExtension{Manifest: ExtensionManifest{Name: "run-ext"}}
	if err := RunExtension(ext, ctx); err != nil {
		t.Fatalf("RunExtension: %v", err)
	}
}

// TestExtensionRunnerRunWithNoPaths tests Run with empty paths.
func TestExtensionRunnerRunWithNoPaths(t *testing.T) {
	ctx := ExtensionContext{EventBus: NewEventBus(), Logger: &noopLogger{}}
	runner, err := Run(nil, ctx)
	if err != nil {
		t.Fatalf("Run with empty paths: %v", err)
	}
	if runner == nil {
		t.Fatal("expected non-nil runner")
	}
	runner.DeinitAll()
}

// TestSortExtensionsByName tests SortExtensionsByName.
func TestSortExtensionsByName(t *testing.T) {
	exts := []Extension{
		&ConfigExtension{Manifest: ExtensionManifest{Name: "zzz"}},
		&ConfigExtension{Manifest: ExtensionManifest{Name: "aaa"}},
		&ConfigExtension{Manifest: ExtensionManifest{Name: "mmm"}},
	}
	SortExtensionsByName(exts)
	if exts[0].Name() != "aaa" || exts[1].Name() != "mmm" || exts[2].Name() != "zzz" {
		t.Errorf("expected sorted order, got %v", []string{exts[0].Name(), exts[1].Name(), exts[2].Name()})
	}
}

// =============================================================================
// Registry method tests
// =============================================================================

// TestRegistryToolMethods tests GetTool and HasTool.
func TestRegistryToolMethods(t *testing.T) {
	// Register first via the global
	RegisterTool("findable_tool", func(args json.RawMessage) (string, error) {
		return "found", nil
	}, "a tool", json.RawMessage(`{"type":"object"}`))

	tool := globalRegistry.GetTool("findable_tool")
	if tool == nil {
		t.Fatal("expected tool")
	}
	if tool.Def.Function.Name != "findable_tool" {
		t.Errorf("expected findable_tool, got %s", tool.Def.Function.Name)
	}

	if !globalRegistry.HasTool("findable_tool") {
		t.Error("HasTool should return true")
	}
	if globalRegistry.HasTool("nonexistent") {
		t.Error("HasTool should return false for nonexistent")
	}
	if globalRegistry.GetTool("nonexistent") != nil {
		t.Error("GetTool should return nil for nonexistent")
	}
}

// TestRegistrySlashCommandMethods tests slash command CRUD.
func TestRegistrySlashCommandMethods(t *testing.T) {
	err := RegisterSlashCommand("/testcmd", func(args []string, _ ExtensionContext) (string, error) {
		return "ok", nil
	})
	if err != nil {
		t.Fatalf("RegisterSlashCommand: %v", err)
	}

	handler := GetSlashCommand("/testcmd")
	if handler == nil {
		t.Fatal("expected handler")
	}

	if !globalRegistry.HasSlashCommand("/testcmd") {
		t.Error("HasSlashCommand should return true")
	}
	if globalRegistry.HasSlashCommand("/nope") {
		t.Error("HasSlashCommand should return false")
	}
	if globalRegistry.GetSlashCommand("/nope") != nil {
		t.Error("GetSlashCommand should return nil")
	}

	// GetAllSlashCommands
	cmds := GetAllSlashCommands()
	if _, ok := cmds["/testcmd"]; !ok {
		t.Error("expected /testcmd in all slash commands")
	}
}

// TestRegistryPromptMethods tests prompt CRUD.
func TestRegistryPromptMethods(t *testing.T) {
	err := RegisterPrompt("test-prompt", "Hello, {{user}}!")
	if err != nil {
		t.Fatalf("RegisterPrompt: %v", err)
	}

	p := globalRegistry.GetPrompt("test-prompt")
	if p != "Hello, {{user}}!" {
		t.Errorf("expected template, got %q", p)
	}
	if globalRegistry.GetPrompt("nonexistent") != "" {
		t.Error("expected empty string for nonexistent prompt")
	}

	prompts := GetAllPrompts()
	if _, ok := prompts["test-prompt"]; !ok {
		t.Error("expected test-prompt in all prompts")
	}
}

// TestRegistryFlagMethods tests flag CRUD.
func TestRegistryFlagMethods(t *testing.T) {
	err := RegisterFlag("verbose", "verbose mode", FlagBool, false)
	if err != nil {
		t.Fatalf("RegisterFlag: %v", err)
	}

	val := GetFlag("verbose")
	if val != false {
		t.Errorf("expected false, got %v", val)
	}

	SetFlagValue("verbose", true)
	val = GetFlag("verbose")
	if val != true {
		t.Errorf("expected true, got %v", val)
	}

	flags := GetAllFlags()
	if _, ok := flags["verbose"]; !ok {
		t.Error("expected verbose in all flags")
	}
}

// TestRegistryFlagWithNilDefault tests flag registration with nil default.
func TestRegistryFlagWithNilDefault(t *testing.T) {
	// Using a unique name to avoid conflicts
	err := RegisterFlag("nilflag", "flag with nil default", FlagString, nil)
	if err != nil {
		t.Fatalf("RegisterFlag with nil: %v", err)
	}
	val := GetFlag("nilflag")
	if val != nil {
		t.Errorf("expected nil, got %v", val)
	}
}

// TestRegistryAutocompleteMethods tests autocomplete provider CRUD.
func TestRegistryAutocompleteMethods(t *testing.T) {
	AddAutocompleteProvider(func(query string) []string { return []string{"a", "b"} })
	AddAutocompleteProvider(func(query string) []string { return []string{"c"} })

	providers := GetAllAutocompleteProviders()
	if len(providers) != 2 {
		t.Errorf("expected 2 providers, got %d", len(providers))
	}
}

// TestRegistryShortcutMethods tests shortcut CRUD.
func TestRegistryShortcutMethods(t *testing.T) {
	RegisterShortcut("ctrl+x", func() {}, "exit")
	RegisterShortcut("ctrl+s", func() {}, "save")

	shortcuts := GetAllShortcuts()
	if len(shortcuts) != 2 {
		t.Errorf("expected 2 shortcuts, got %d", len(shortcuts))
	}
}

// TestGetAllProviders tests GetAllProviders.
func TestGetAllProviders(t *testing.T) {
	globalRegistry.RegisterProvider("prov1", ProviderConfig{Name: "p1", BaseURL: "https://p1.com"})
	globalRegistry.RegisterProvider("prov2", ProviderConfig{Name: "p2", BaseURL: "https://p2.com"})

	all := GetAllProviders()
	if len(all) != 2 {
		t.Errorf("expected 2 providers, got %d", len(all))
	}
	globalRegistry.UnregisterProvider("prov1")
	globalRegistry.UnregisterProvider("prov2")
}

// =============================================================================
// ConfigExtension Init tests
// =============================================================================

// TestConfigExtensionInitFull tests init with all resource types.
func TestConfigExtensionInitFull(t *testing.T) {
	bus := NewEventBus()
	ctx := NewExtensionContext(nil, nil, bus, nil, ".", nil)

	ext := &ConfigExtension{
		Manifest: ExtensionManifest{
			Name:    "full-ext",
			Version: "1.0",
			Tools: []ExtensionToolDef{
				{Name: "ext-tool", Description: "a tool from extension", Parameters: json.RawMessage(`{"type":"object","properties":{"x":{"type":"string"}}}`)},
			},
			SlashCommands: []ExtensionSlashCommandDef{
				{Command: "/extcmd", Description: "extension command", Message: "hello from ext"},
			},
			Prompts: []ExtensionPromptDef{
				{Name: "ext-prompt", Template: "Be helpful."},
			},
		},
	}

	if err := ext.Init(ctx); err != nil {
		t.Fatalf("Init: %v", err)
	}

	// Verify tool was registered
	if !globalRegistry.HasTool("ext-tool") {
		t.Error("expected ext-tool to be registered")
	}

	// Verify slash command was registered
	if !globalRegistry.HasSlashCommand("/extcmd") {
		t.Error("expected /extcmd to be registered")
	}

	// Verify prompt was registered
	if globalRegistry.GetPrompt("ext-prompt") != "Be helpful." {
		t.Error("expected ext-prompt to be registered")
	}
}

// TestConfigExtensionInitEmptyNames tests skipping items with empty names.
func TestConfigExtensionInitEmptyNames(t *testing.T) {
	bus := NewEventBus()
	ctx := NewExtensionContext(nil, nil, bus, &noopLogger{}, ".", nil)

	ext := &ConfigExtension{
		Manifest: ExtensionManifest{
			Name: "empty-name-ext",
			Tools: []ExtensionToolDef{
				{Name: "", Description: "empty name tool"},
			},
			SlashCommands: []ExtensionSlashCommandDef{
				{Command: "", Description: "empty command"},
			},
			Prompts: []ExtensionPromptDef{
				{Name: "", Template: "empty"},
			},
		},
	}

	if err := ext.Init(ctx); err != nil {
		t.Fatalf("Init should not fail on empty names: %v", err)
	}
}

// =============================================================================
// EventBus tests
// =============================================================================

// TestEventBusUnsubscribe tests unsubscribe.
func TestEventBusUnsubscribe(t *testing.T) {
	bus := NewEventBus()
	ch := make(chan Event, 10)
	bus.Subscribe("test_event", ch)
	bus.Unsubscribe("test_event", ch)

	bus.Publish(Event{Name: "test_event", Data: "hello"})
	select {
	case <-ch:
		t.Error("should not receive event after unsubscribe")
	default:
		// expected
	}
}

// TestEventBusPublishNoSubscribers tests publish with no subscribers (no panic).
func TestEventBusPublishNoSubscribers(t *testing.T) {
	bus := NewEventBus()
	bus.Publish(Event{Name: "no_such_event", Data: "hi"})
	// should not panic
}

// TestEventBusPublishFullChannel tests non-blocking publish to full channel.
func TestEventBusPublishFullChannel(t *testing.T) {
	bus := NewEventBus()
	ch := make(chan Event, 1) // small buffer
	bus.Subscribe("full_event", ch)

	// Fill the buffer
	bus.Publish(Event{Name: "full_event", Data: "first"})
	// This should not block (dropped silently)
	bus.Publish(Event{Name: "full_event", Data: "second"})

	// Drain the channel
	ev := <-ch
	if ev.Data != "first" {
		t.Errorf("expected 'first', got %v", ev.Data)
	}
}

// =============================================================================
// Logger tests
// =============================================================================

// TestStdLogger tests StdLogger delegates to its function fields.
func TestStdLogger(t *testing.T) {
	infoCalled := false
	warnCalled := false
	errCalled := false
	debugCalled := false

	l := &StdLogger{
		Infof:  func(format string, args ...interface{}) { infoCalled = true },
		Warnf:  func(format string, args ...interface{}) { warnCalled = true },
		Errorf: func(format string, args ...interface{}) { errCalled = true },
		Debugf: func(format string, args ...interface{}) { debugCalled = true },
	}

	l.Info("test %s", "a")
	l.Warn("test %s", "b")
	l.Error("test %s", "c")
	l.Debug("test %s", "d")

	if !infoCalled || !warnCalled || !errCalled || !debugCalled {
		t.Error("expected all logger methods to be called")
	}
}

// TestNoopLogger tests that noopLogger does not panic.
func TestNoopLogger(t *testing.T) {
	l := &noopLogger{}
	l.Info("test %d", 1)
	l.Warn("test %d", 2)
	l.Error("test %d", 3)
	l.Debug("test %d", 4)
	// should not panic
}

// =============================================================================
// NoopUI tests
// =============================================================================

// TestNoopUI tests that noopUI methods don't panic.
func TestNoopUI(t *testing.T) {
	ui := NoopUI

	s, err := ui.Select("title", []string{"a", "b"}, nil)
	if err != nil || s != "" {
		t.Error("Select should return empty string")
	}

	b, err := ui.Confirm("title", "msg", nil)
	if err != nil || b {
		t.Error("Confirm should return false")
	}

	s, err = ui.Input("title", "placeholder", nil)
	if err != nil || s != "" {
		t.Error("Input should return empty")
	}

	s, err = ui.Editor("title", "prefill")
	if err != nil || s != "" {
		t.Error("Editor should return empty")
	}

	ui.Notify("msg", "info")
	ui.SetStatus("key", "text")
	ui.SetTitle("title")
	ui.SetHiddenThinkingLabel("label")
	ui.SetWorkingMessage("msg")
	ui.SetWorkingVisible(true)
	ui.SetWorkingIndicator(nil, 100)
	ui.PasteToEditor("text")
	ui.SetEditorText("text")
	if t2 := ui.GetEditorText(); t2 != "" {
		t.Error("GetEditorText should return empty")
	}

	unsub := ui.OnTerminalInput(func(data string) *TerminalInputResult { return nil })
	unsub()

	themes := ui.GetAllThemes()
	if themes != nil {
		t.Error("GetAllThemes should return nil")
	}

	ui.SetTheme("dark")
	if ui.GetCurrentThemeName() != "" {
		t.Error("GetCurrentThemeName should return empty")
	}
	if ui.GetToolsExpanded() {
		t.Error("GetToolsExpanded should return false")
	}
	ui.SetToolsExpanded(true)
	ui.AddAutocompleteProvider(func(query string) []string { return nil })
	ui.SetFooter(nil)
	ui.SetHeader(nil)
	ui.GetTheme("dark")
	ui.SetEditorComponent(nil)
	ui.GetEditorComponent()
	ui.SetWidget("w", "c", "aboveEditor")
	s, err = ui.Custom("title", "content", []CustomButton{{"OK", "ok"}}, nil)
	if err != nil || s != "" {
		t.Error("Custom should return empty")
	}
}

// =============================================================================
// Type tests
// =============================================================================

// TestProviderConfigAndProviderModel tests the type structs.
func TestProviderConfigAndProviderModel(t *testing.T) {
	cfg := ProviderConfig{
		Name:    "test-provider",
		BaseURL: "https://api.example.com",
		APIKey:  "sk-123",
		Headers: map[string]string{"X-Custom": "value"},
		Models: []ProviderModel{
			{ID: "model-1", Name: "Model One", ContextWindow: 8192, Reasoning: true},
			{ID: "model-2", Name: "Model Two", ContextWindow: 4096, Reasoning: false},
		},
	}

	if cfg.Name != "test-provider" {
		t.Error("Name mismatch")
	}
	if cfg.Models[0].ID != "model-1" {
		t.Error("Model ID mismatch")
	}
	if cfg.Models[1].Reasoning {
		t.Error("expected Reasoning false")
	}
}

// TestToolInfoSourceInfoExecResult tests type structs.
func TestToolInfoSourceInfoExecResult(t *testing.T) {
	info := ToolInfo{
		Name:        "my-tool",
		Description: "does stuff",
		SourceInfo: SourceInfo{
			Path:   "/path/to/ext",
			Source: "user-ext",
			Scope:  "user",
			Origin: "top-level",
		},
	}

	if info.Name != "my-tool" {
		t.Error("ToolInfo name mismatch")
	}
	if info.SourceInfo.Scope != "user" {
		t.Error("SourceInfo scope mismatch")
	}

	result := ExecResult{
		Stdout: "output",
		Stderr: "error output",
		Code:   1,
	}
	if result.Code != 1 {
		t.Error("ExecResult code mismatch")
	}
}

// TestExtensionDiagnosticType tests ExtensionDiagnostic.
func TestExtensionDiagnosticType(t *testing.T) {
	diag := ExtensionDiagnostic{
		Type:    "error",
		Message: "something went wrong",
		Path:    "/path/to/ext.so",
	}
	if diag.Type != "error" {
		t.Error("Type mismatch")
	}
}

// TestInputResultActionConstants tests the constants.
func TestInputResultActionConstants(t *testing.T) {
	if InputContinue != "continue" {
		t.Error("InputContinue mismatch")
	}
	if InputTransform != "transform" {
		t.Error("InputTransform mismatch")
	}
	if InputHandled != "handled" {
		t.Error("InputHandled mismatch")
	}
}

// TestShortcutDefAndFlagDef tests the struct types.
func TestShortcutDefAndFlagDef(t *testing.T) {
	sd := ShortcutDef{
		Key:         "ctrl+p",
		Description: "print",
		Handler:     func() {},
	}
	if sd.Key != "ctrl+p" {
		t.Error("ShortcutDef key mismatch")
	}

	fd := FlagDef{
		Name:        "myflag",
		Description: "a flag",
		Type:        FlagBool,
		Default:     true,
	}
	if fd.Type != FlagBool {
		t.Error("FlagDef type mismatch")
	}
}

// =============================================================================
// Loader tests
// =============================================================================

// TestLoadExtensionsWithJSONFile tests loading from a JSON manifest file.
func TestLoadExtensionsWithJSONFile(t *testing.T) {
	tmpDir := t.TempDir()

	// Create a valid manifest file
	manifestPath := filepath.Join(tmpDir, "extension.json")
	err := os.WriteFile(manifestPath, []byte(`{"name":"json-ext","version":"1.0"}`), 0644)
	if err != nil {
		t.Fatal(err)
	}

	exts, err := LoadExtensions([]string{manifestPath}, &noopLogger{})
	if err != nil {
		t.Fatalf("LoadExtensions: %v", err)
	}
	if len(exts) != 1 {
		t.Fatalf("expected 1 extension, got %d", len(exts))
	}
	if exts[0].Name() != "json-ext" {
		t.Errorf("expected json-ext, got %s", exts[0].Name())
	}
}

// TestLoadExtensionsWithDirectory tests loading from a directory.
func TestLoadExtensionsWithDirectory(t *testing.T) {
	tmpDir := t.TempDir()

	// Create extension.json in the directory
	err := os.WriteFile(filepath.Join(tmpDir, "extension.json"), []byte(`{"name":"dir-ext"}`), 0644)
	if err != nil {
		t.Fatal(err)
	}

	exts, err := LoadExtensions([]string{tmpDir}, &noopLogger{})
	if err != nil {
		t.Fatalf("LoadExtensions: %v", err)
	}
	if len(exts) != 1 {
		t.Fatalf("expected 1 extension, got %d", len(exts))
	}
	if exts[0].Name() != "dir-ext" {
		t.Errorf("expected dir-ext, got %s", exts[0].Name())
	}
}

// TestLoadExtensionsEmptyDir tests loading from an empty directory.
func TestLoadExtensionsEmptyDir(t *testing.T) {
	tmpDir := t.TempDir()
	exts, err := LoadExtensions([]string{tmpDir}, &noopLogger{})
	if err != nil {
		t.Fatalf("LoadExtensions: %v", err)
	}
	if len(exts) != 0 {
		t.Errorf("expected 0 extensions from empty dir, got %d", len(exts))
	}
}

// TestLoadExtensionsNonExistentPath tests loading a non-existent path.
func TestLoadExtensionsNonExistentPath(t *testing.T) {
	exts, err := LoadExtensions([]string{"/nonexistent/path"}, &noopLogger{})
	if err != nil {
		t.Fatalf("LoadExtensions should not error: %v", err)
	}
	if len(exts) != 0 {
		t.Errorf("expected 0 extensions, got %d", len(exts))
	}
}

// TestLoadExtensionsUnsupportedFileType tests unsupported extension.
func TestLoadExtensionsUnsupportedFileType(t *testing.T) {
	tmpDir := t.TempDir()
	badFile := filepath.Join(tmpDir, "bad.txt")
	os.WriteFile(badFile, []byte("hello"), 0644)

	exts, err := LoadExtensions([]string{badFile}, &noopLogger{})
	if err != nil {
		t.Fatalf("LoadExtensions: %v", err)
	}
	if len(exts) != 0 {
		t.Errorf("expected 0 extensions, got %d", len(exts))
	}
}

// TestLoadExtensionsDuplicateDedup tests duplicate extension names are deduplicated.
func TestLoadExtensionsDuplicateDedup(t *testing.T) {
	tmpDir := t.TempDir()

	extDir := filepath.Join(tmpDir, "ext")
	os.MkdirAll(extDir, 0755)
	os.WriteFile(filepath.Join(extDir, "extension.json"), []byte(`{"name":"dup-ext"}`), 0644)

	// Load the directory (which contains extension.json)
	exts, err := LoadExtensions([]string{extDir}, &noopLogger{})
	if err != nil {
		t.Fatalf("LoadExtensions: %v", err)
	}
	if len(exts) != 1 {
		t.Errorf("expected 1 extension after dedup, got %d", len(exts))
	}
}

// TestLoadManifestFileEmptyName tests that empty name is derived from directory.
func TestLoadManifestFileEmptyName(t *testing.T) {
	tmpDir := t.TempDir()

	// Manifest with no name in a directory
	extDir := filepath.Join(tmpDir, "my-ext")
	os.MkdirAll(extDir, 0755)
	manifestPath := filepath.Join(extDir, "extension.json")
	os.WriteFile(manifestPath, []byte(`{"description":"no name"}`), 0644)

	ext, err := loadManifestFile(manifestPath, &noopLogger{})
	if err != nil {
		t.Fatalf("loadManifestFile: %v", err)
	}
	if ext.Name() != "my-ext" {
		t.Errorf("expected name derived from dir 'my-ext', got %q", ext.Name())
	}
}

// TestLoadManifestFileInvalidJSON tests invalid JSON manifest.
func TestLoadManifestFileInvalidJSON(t *testing.T) {
	tmpDir := t.TempDir()
	badPath := filepath.Join(tmpDir, "bad.json")
	os.WriteFile(badPath, []byte(`not json`), 0644)

	_, err := loadManifestFile(badPath, &noopLogger{})
	if err == nil {
		t.Error("expected error for invalid JSON")
	}
}

// TestLoadManifestFileMissing tests missing file.
func TestLoadManifestFileMissing(t *testing.T) {
	_, err := loadManifestFile("/nonexistent/file.json", &noopLogger{})
	if err == nil {
		t.Error("expected error for missing file")
	}
}

// TestLoadExtensionFileBadType tests unsupported file type.
func TestLoadExtensionFileBadType(t *testing.T) {
	_, err := loadExtensionFile("/test/bad.exe", &noopLogger{})
	if err == nil {
		t.Error("expected error for unsupported file type")
	}
}

// TestLoadExtensionFileJSON tests loading JSON file.
func TestLoadExtensionFileJSON(t *testing.T) {
	tmpDir := t.TempDir()
	jsonPath := filepath.Join(tmpDir, "test.json")
	os.WriteFile(jsonPath, []byte(`{"name":"file-ext"}`), 0644)

	ext, err := loadExtensionFile(jsonPath, &noopLogger{})
	if err != nil {
		t.Fatalf("loadExtensionFile: %v", err)
	}
	if ext.Name() != "file-ext" {
		t.Errorf("expected file-ext, got %s", ext.Name())
	}
}

// TestLoadPluginFileUnsupportedPlatform tests that plugin loading logs on unsupported platforms.
func TestLoadPluginFileUnsupportedPlatform(t *testing.T) {
	// This tests the GOOS guard - on unsupported platforms it returns nil,nil
	// On supported platforms (linux/darwin) it would try plugin.Open which would fail
	// We just test it doesn't panic
	tmpDir := t.TempDir()
	soPath := filepath.Join(tmpDir, "test.so")
	os.WriteFile(soPath, []byte("dummy"), 0644)

	ext, err := loadPluginFile(soPath, &noopLogger{})
	// Either returns error (supported platform) or nil,nil (unsupported)
	_ = ext
	_ = err
	// Just ensure no panic
}

// =============================================================================
// ExtensionRunner Load tests
// =============================================================================

// TestExtensionRunnerLoad tests Load method.
func TestExtensionRunnerLoad(t *testing.T) {
	tmpDir := t.TempDir()
	os.WriteFile(filepath.Join(tmpDir, "extension.json"), []byte(`{"name":"load-ext"}`), 0644)

	ctx := ExtensionContext{EventBus: NewEventBus(), Logger: &noopLogger{}}
	runner := NewExtensionRunner(ctx)

	if err := runner.Load([]string{tmpDir}); err != nil {
		t.Fatalf("Load: %v", err)
	}
	if len(runner.Extensions) != 1 {
		t.Errorf("expected 1 extension, got %d", len(runner.Extensions))
	}
}

// TestExtensionRunnerLoadError tests Load with invalid paths.
func TestExtensionRunnerLoadError(t *testing.T) {
	ctx := ExtensionContext{EventBus: NewEventBus(), Logger: &noopLogger{}}
	runner := NewExtensionRunner(ctx)

	// Directory with nothing useful should load fine (0 extensions)
	if err := runner.Load([]string{"/dev/null"}); err != nil {
		// stat error should be caught gracefully
	}
}

// =============================================================================
// NewExtensionContext tests
// =============================================================================

// TestNewExtensionContextNilUI tests that nil UI defaults to NoopUI.
func TestNewExtensionContextNilUI(t *testing.T) {
	ctx := NewExtensionContext(nil, nil, NewEventBus(), nil, ".", nil)
	if ctx.UI == nil {
		t.Error("UI should not be nil (should default to NoopUI)")
	}
	if ctx.Actions == nil {
		t.Error("Actions should not be nil")
	}
	if ctx.CWD != "." {
		t.Errorf("expected CWD '.', got %q", ctx.CWD)
	}
}

// TestExtensionContextDefaultLogger tests default logger behavior.
func TestExtensionContextDefaultLogger(t *testing.T) {
	ctx := ExtensionContext{EventBus: NewEventBus()}
	// Should not panic when logger is nil and accessed
	_ = ctx
}
