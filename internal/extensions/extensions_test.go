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
