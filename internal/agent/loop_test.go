package agent

import (
	"context"
	"encoding/json"
	"strings"
	"sync"
	"testing"
	"time"

	"github.com/huichen/xihu/pkg/types"
)

// mockProvider implements types.LLMProvider for testing.
type mockProvider struct {
	mu       sync.Mutex
	events   []types.StreamEvent // events to emit per call
	callCount int
	// perCallEvents allows different event sequences per call
	perCallEvents [][]types.StreamEvent
	err           error
}

func (m *mockProvider) StreamChat(model string, messages []types.Message, tools []types.ToolDef, systemPrompt string) (<-chan types.StreamEvent, error) {
	m.mu.Lock()
	callIdx := m.callCount
	m.callCount++
	m.mu.Unlock()

	if m.err != nil {
		return nil, m.err
	}

	var evts []types.StreamEvent
	if callIdx < len(m.perCallEvents) {
		evts = m.perCallEvents[callIdx]
	} else {
		evts = m.events
	}

	ch := make(chan types.StreamEvent, len(evts)+1)
	for _, e := range evts {
		ch <- e
	}
	close(ch)
	return ch, nil
}

// simpleTextEvents returns events that produce a single text response.
func simpleTextEvents(text string) []types.StreamEvent {
	return []types.StreamEvent{
		{Type: "text_delta", Text: text},
		{Type: "stop"},
	}
}

// simpleToolCallEvents returns events that produce a tool call then stop.
func simpleToolCallEvents(toolName, toolID, args string) []types.StreamEvent {
	return []types.StreamEvent{
		{Type: "tool_call", ToolCall: &types.ToolCall{
			ID:   toolID,
			Type: "function",
			Function: types.ToolCallFn{
				Name:      toolName,
				Arguments: json.RawMessage(args),
			},
		}},
		{Type: "stop"},
	}
}

// textThenToolEvents: first call returns text, second returns a tool call, third returns stop.
// This is used for follow-up testing.
func textThenToolThenStop() [][]types.StreamEvent {
	return [][]types.StreamEvent{
		simpleTextEvents("Hello!"),
		simpleToolCallEvents("test_tool", "call_1", `{"key":"value"}`),
		simpleTextEvents("Done."),
	}
}

func TestNewLoop(t *testing.T) {
	mp := &mockProvider{}
	l := NewLoop(mp, "test-model")

	if l.Provider != mp {
		t.Error("Provider not set")
	}
	if l.Model != "test-model" {
		t.Errorf("Model = %q, want %q", l.Model, "test-model")
	}
	if l.Config.MaxTurns != DefaultMaxTurns {
		t.Errorf("MaxTurns = %d, want %d", l.Config.MaxTurns, DefaultMaxTurns)
	}
	if l.SteeringQueue == nil {
		t.Error("SteeringQueue should be initialized")
	}
	if l.SteeringQueue.Len() != 0 {
		t.Error("SteeringQueue should be empty")
	}
	if l.FollowUpQueue == nil {
		t.Error("FollowUpQueue should be initialized")
	}
}

func TestRunStreamingWithMessages_SimpleText(t *testing.T) {
	mp := &mockProvider{
		events: simpleTextEvents("Hello, world!"),
	}
	l := NewLoop(mp, "test-model")

	messages := []types.AgentMessage{
		newUserAgentMessage("Hi"),
	}
	ctx := context.Background()
	var collectedText string
	result, msgs, err := l.RunStreamingWithMessages(ctx, messages, func(text string) {
		collectedText += text
	})

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result != "Hello, world!" {
		t.Errorf("result = %q, want %q", result, "Hello, world!")
	}
	if collectedText != "Hello, world!" {
		t.Errorf("collected text = %q, want %q", collectedText, "Hello, world!")
	}
	// Should have user + assistant messages
	if len(msgs) != 2 {
		t.Errorf("got %d messages, want 2", len(msgs))
	}
	if len(msgs) == 2 && msgs[1].Role != "assistant" {
		t.Errorf("last message role = %q, want assistant", msgs[1].Role)
	}
}

func TestRunStreamingWithMessages_ToolCall(t *testing.T) {
	called := false
	tool := types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "test_tool",
				Description: "A test tool",
				Parameters:  json.RawMessage(`{"type":"object"}`),
			},
		},
		Handler: func(args json.RawMessage) (string, error) {
			called = true
			return "tool result", nil
		},
	}

	mp := &mockProvider{
		perCallEvents: [][]types.StreamEvent{
			simpleToolCallEvents("test_tool", "call_1", `{"key":"value"}`),
			simpleTextEvents("Final answer."),
		},
	}
	l := NewLoop(mp, "test-model")
	l.Tools = []types.AgentTool{tool}

	messages := []types.AgentMessage{
		newUserAgentMessage("Do something"),
	}
	ctx := context.Background()
	result, msgs, err := l.RunStreamingWithMessages(ctx, messages, nil)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !called {
		t.Error("tool handler was not called")
	}
	if result != "Final answer." {
		t.Errorf("result = %q, want %q", result, "Final answer.")
	}
	// user → assistant(tool call) → tool → assistant(text)
	if len(msgs) != 4 {
		t.Errorf("got %d messages, want 4", len(msgs))
	}
}

func TestRunStreamingWithMessages_AbortSignal(t *testing.T) {
	mp := &mockProvider{
		events: simpleTextEvents("Hello!"),
	}
	l := NewLoop(mp, "test-model")

	ctx, cancel := context.WithCancel(context.Background())
	cancel() // cancel immediately

	messages := []types.AgentMessage{
		newUserAgentMessage("Hi"),
	}
	_, _, err := l.RunStreamingWithMessages(ctx, messages, nil)

	// After the interrupt refactor, a cancelled context with no steering
	// messages returns nil error (clean interrupt exit), not an error.
	if err != nil {
		t.Fatalf("expected nil from clean interrupt, got: %v", err)
	}
}

func test_RunStreamingWithMessages_SteeringQueue(t *testing.T) {
	mp := &mockProvider{
		perCallEvents: [][]types.StreamEvent{
			// First call: return text, no tool calls
			simpleTextEvents("First response."),
			// Second call (after steering injected): return more text
			simpleTextEvents("Steered response."),
		},
	}
	l := NewLoop(mp, "test-model")

	// Queue a steering message before starting
	l.SteeringQueue.Enqueue("Steer: do more")

	messages := []types.AgentMessage{
		newUserAgentMessage("Hi"),
	}
	ctx := context.Background()
	result, _, err := l.RunStreamingWithMessages(ctx, messages, nil)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result != "Steered response." {
		t.Errorf("result = %q, want %q", result, "Steered response.")
	}
}

func test_RunStreamingWithMessages_FollowUp(t *testing.T) {
	mp := &mockProvider{
		perCallEvents: textThenToolThenStop(),
	}
	tool := types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "test_tool",
				Description: "A test tool",
				Parameters:  json.RawMessage(`{"type":"object"}`),
			},
		},
		Handler: func(args json.RawMessage) (string, error) {
			return "tool done", nil
		},
	}
	l := NewLoop(mp, "test-model")
	l.Tools = []types.AgentTool{tool}

	messages := []types.AgentMessage{
		newUserAgentMessage("Do something"),
	}
	ctx := context.Background()
	result, _, err := l.RunStreamingWithMessages(ctx, messages, nil)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result != "Done." {
		t.Errorf("result = %q, want %q", result, "Done.")
	}
}

func TestRunStreamingWithMessages_MaxTurnsExceeded(t *testing.T) {
	// Create a mock that always returns a tool call, causing infinite loop
	mp := &mockProvider{
		events: simpleToolCallEvents("test_tool", "call_1", `{}`),
	}
	tool := types.AgentTool{
		Def: types.ToolDef{
			Type: "function",
			Function: types.FunctionDef{
				Name:        "test_tool",
				Description: "A test tool",
				Parameters:  json.RawMessage(`{"type":"object"}`),
			},
		},
		Handler: func(args json.RawMessage) (string, error) {
			return "ok", nil
		},
	}
	l := NewLoop(mp, "test-model")
	l.Config.MaxTurns = 3
	l.Tools = []types.AgentTool{tool}

	messages := []types.AgentMessage{
		newUserAgentMessage("Loop forever"),
	}
	ctx := context.Background()
	_, _, err := l.RunStreamingWithMessages(ctx, messages, nil)

	if err == nil {
		t.Fatal("expected max turns error, got nil")
	}
	if !strings.Contains(err.Error(), "exceeded max turns") {
		t.Errorf("error should mention max turns, got: %v", err)
	}
}

func TestRunStreamingWithMessages_LastMessageAssistant(t *testing.T) {
	mp := &mockProvider{}
	l := NewLoop(mp, "test-model")

	// Last message is from assistant — should be rejected
	messages := []types.AgentMessage{
		newUserAgentMessage("Hi"),
		{Role: "assistant", Content: []types.ContentBlock{types.TextBlock{Text: "Hello"}}},
	}
	ctx := context.Background()
	_, _, err := l.RunStreamingWithMessages(ctx, messages, nil)

	if err == nil {
		t.Fatal("expected error, got nil")
	}
	if !strings.Contains(err.Error(), "last message must not be from assistant") {
		t.Errorf("unexpected error: %v", err)
	}
}

func TestRunStreamingWithMessages_EmptyMessages(t *testing.T) {
	mp := &mockProvider{
		events: simpleTextEvents("Response without context."),
	}
	l := NewLoop(mp, "test-model")

	ctx := context.Background()
	result, _, err := l.RunStreamingWithMessages(ctx, nil, nil)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result != "Response without context." {
		t.Errorf("result = %q", result)
	}
}

func TestRunStreamingWithMessages_ContextTransform(t *testing.T) {
	transformCalled := false
	mp := &mockProvider{
		events: simpleTextEvents("Transformed."),
	}
	l := NewLoop(mp, "test-model")
	l.Config.TransformContext = func(msgs []types.Message, _ string) []types.Message {
		transformCalled = true
		// Prefix a system message
		sys := newSystemMessage("Context was transformed")
		return append([]types.Message{sys}, msgs...)
	}

	messages := []types.AgentMessage{
		newUserAgentMessage("Hi"),
	}
	ctx := context.Background()
	_, _, err := l.RunStreamingWithMessages(ctx, messages, nil)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !transformCalled {
		t.Error("TransformContext was not called")
	}
}

func TestRunStreamingWithMessages_StopCondition(t *testing.T) {
	mp := &mockProvider{
		events: simpleTextEvents("STOP"),
	}
	l := NewLoop(mp, "test-model")
	l.Config.StopCondition = func(msgs []types.Message, lastResponse string) bool {
		return lastResponse == "STOP"
	}

	messages := []types.AgentMessage{
		newUserAgentMessage("Hi"),
	}
	ctx := context.Background()
	result, _, err := l.RunStreamingWithMessages(ctx, messages, nil)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result != "STOP" {
		t.Errorf("result = %q, want STOP", result)
	}
}

func TestRunStreamingWithMessages_ProviderError(t *testing.T) {
	mp := &mockProvider{
		err: context.DeadlineExceeded,
	}
	l := NewLoop(mp, "test-model")

	messages := []types.AgentMessage{
		newUserAgentMessage("Hi"),
	}
	ctx := context.Background()
	_, _, err := l.RunStreamingWithMessages(ctx, messages, nil)

	if err == nil {
		t.Fatal("expected error from provider, got nil")
	}
}

func TestRunStreaming_NewSession(t *testing.T) {
	mp := &mockProvider{
		events: simpleTextEvents("Response"),
	}
	l := NewLoop(mp, "test-model")
	l.SystemPrompt = "You are helpful."

	ctx := context.Background()
	var text string
	result, err := l.RunStreaming(ctx, "Hello", func(t string) {
		text += t
	})

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result != "Response" {
		t.Errorf("result = %q", result)
	}
	if text != "Response" {
		t.Errorf("onText collected = %q", text)
	}
}

func TestRunStreamingWithMessages_ParallelTools(t *testing.T) {
	var mu sync.Mutex
	var callOrder []string
	makeTool := func(name string) types.AgentTool {
		return types.AgentTool{
			Def: types.ToolDef{
				Type: "function",
				Function: types.FunctionDef{
					Name:        name,
					Description: "tool " + name,
					Parameters:  json.RawMessage(`{"type":"object"}`),
				},
			},
			Handler: func(args json.RawMessage) (string, error) {
				mu.Lock()
				callOrder = append(callOrder, name)
				mu.Unlock()
				time.Sleep(10 * time.Millisecond) // ensure concurrency
				return name + "_result", nil
			},
		}
	}

	mp := &mockProvider{
		perCallEvents: [][]types.StreamEvent{
			{
				{Type: "tool_call", ToolCall: &types.ToolCall{
					ID: "tc1", Type: "function",
					Function: types.ToolCallFn{Name: "tool_a", Arguments: json.RawMessage(`{}`)},
				}},
				{Type: "tool_call", ToolCall: &types.ToolCall{
					ID: "tc2", Type: "function",
					Function: types.ToolCallFn{Name: "tool_b", Arguments: json.RawMessage(`{}`)},
				}},
				{Type: "stop"},
			},
			simpleTextEvents("Done"),
		},
	}
	l := NewLoop(mp, "test-model")
	l.ParallelTools = true
	l.Tools = []types.AgentTool{makeTool("tool_a"), makeTool("tool_b")}

	messages := []types.AgentMessage{
		newUserAgentMessage("Run both"),
	}
	ctx := context.Background()
	_, _, err := l.RunStreamingWithMessages(ctx, messages, nil)

	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(callOrder) != 2 {
		t.Errorf("expected 2 tool calls, got %d", len(callOrder))
	}
}

func TestDrainSteering(t *testing.T) {
	l := NewLoop(&mockProvider{}, "test")

	// Queue multiple messages
	l.SteeringQueue.Enqueue("msg1")
	l.SteeringQueue.Enqueue("msg2")

	msgs := []types.AgentMessage{
		newUserAgentMessage("original"),
	}
	result := l.drainSteering(msgs)

	if len(result) != 3 {
		t.Fatalf("expected 3 messages, got %d", len(result))
	}
	if result[0].Role != "user" {
		t.Errorf("msg 0 role = %q", result[0].Role)
	}
	// Verify the steering messages are user messages
	for i := 1; i < 3; i++ {
		if result[i].Role != "user" {
			t.Errorf("msg %d role = %q, want user", i, result[i].Role)
		}
	}

	// Channel should be drained
	if l.SteeringQueue.Len() != 0 {
		t.Error("steering queue should be empty")
	}
}

func TestExecuteTool_NotFound(t *testing.T) {
	l := NewLoop(&mockProvider{}, "test")
	_, err := l.executeTool(types.ToolCall{
		Function: types.ToolCallFn{Name: "nonexistent"},
	})
	if err == nil {
		t.Fatal("expected error for unknown tool")
	}
	if !strings.Contains(err.Error(), "not found") {
		t.Errorf("error = %q, want contains 'not found'", err.Error())
	}
}

func TestTruncate(t *testing.T) {
	tests := []struct {
		input    string
		n        int
		expected string
	}{
		{"hello", 10, "hello"},
		{"hello world", 5, "hello..."},
		{"", 5, ""},
		{"abc", 3, "abc"},
		{"abcd", 3, "abc..."},
	}
	for _, tt := range tests {
		got := runeTruncate(tt.input, tt.n)
		if got != tt.expected {
			t.Errorf("truncate(%q, %d) = %q, want %q", tt.input, tt.n, got, tt.expected)
		}
	}
}
