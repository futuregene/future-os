package extensions

import (
	"os"
	"path/filepath"
	"testing"
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
	runner.EmitSessionShutdown()

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
}
