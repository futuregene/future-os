package events

import (
	"testing"
	"time"

	"github.com/huichen/xihu/pkg/types"
)

func TestNewEventBus(t *testing.T) {
	bus := NewEventBus()
	if bus == nil {
		t.Fatal("NewEventBus returned nil")
	}
	if bus.subscribers == nil {
		t.Fatal("subscribers map is nil")
	}
	if bus.closed {
		t.Fatal("bus is already closed")
	}
}

func TestEventBusSubscribe(t *testing.T) {
	bus := NewEventBus()
	ch := bus.Subscribe("sub1")
	if ch == nil {
		t.Fatal("Subscribe returned nil channel")
	}
	// Subscribe again with different ID
	ch2 := bus.Subscribe("sub2")
	if ch2 == nil {
		t.Fatal("second Subscribe returned nil")
	}
}

func TestEventBusSubscribeOnClosed(t *testing.T) {
	bus := NewEventBus()
	bus.Close()
	ch := bus.Subscribe("sub1")
	if ch != nil {
		t.Error("Subscribe on closed bus should return nil")
	}
}

func TestEventBusUnsubscribe(t *testing.T) {
	bus := NewEventBus()
	ch := bus.Subscribe("sub1")
	_ = ch

	bus.Unsubscribe("sub1")
	// After unsubscribing, the channel is closed
	// Verify by checking len of subscribers
	bus.mu.RLock()
	_, ok := bus.subscribers["sub1"]
	bus.mu.RUnlock()
	if ok {
		t.Error("subscriber should have been removed")
	}
}

func TestEventBusUnsubscribeNonExistent(t *testing.T) {
	bus := NewEventBus()
	// Should not panic
	bus.Unsubscribe("nonexistent")
}

func TestEventBusEmit(t *testing.T) {
	bus := NewEventBus()
	ch := bus.Subscribe("sub1")

	event := AgentEvent{
		Type: "test_event",
		Data: map[string]interface{}{"key": "value"},
	}

	go func() {
		bus.Emit(event)
	}()

	select {
	case received := <-ch:
		if received.Type != "test_event" {
			t.Errorf("type = %s, want test_event", received.Type)
		}
		if received.Data["key"] != "value" {
			t.Errorf("data key = %v", received.Data["key"])
		}
		if received.Timestamp.IsZero() {
			t.Error("timestamp should be set")
		}
	case <-time.After(time.Second):
		t.Fatal("timeout waiting for event")
	}
}

func TestEventBusEmitMultipleSubscribers(t *testing.T) {
	bus := NewEventBus()
	ch1 := bus.Subscribe("sub1")
	ch2 := bus.Subscribe("sub2")

	bus.Emit(AgentEvent{Type: "broadcast"})

	// Both should receive
	select {
	case evt := <-ch1:
		if evt.Type != "broadcast" {
			t.Error("ch1 got wrong event")
		}
	default:
		t.Error("ch1 did not receive event")
	}

	select {
	case evt := <-ch2:
		if evt.Type != "broadcast" {
			t.Error("ch2 got wrong event")
		}
	default:
		t.Error("ch2 did not receive event")
	}
}

func TestEventBusEmitNonBlocking(t *testing.T) {
	bus := NewEventBus()
	// Channel with buffer 0 won't work; subscribe gives buffer of 64
	ch := bus.Subscribe("sub1")

	// Fill up the buffer (64 events)
	for i := 0; i < 64; i++ {
		bus.Emit(AgentEvent{Type: "fill"})
	}

	// This one should be dropped (non-blocking)
	bus.Emit(AgentEvent{Type: "dropped"})

	// Drain first 64 events
	for i := 0; i < 64; i++ {
		<-ch
	}

	// The dropped event should not be there (channel empty)
	select {
	case <-ch:
		t.Error("should not have received dropped event")
	default:
		// Expected
	}
}

func TestEventBusEmitOnClosed(t *testing.T) {
	bus := NewEventBus()
	bus.Close()
	// Should not panic
	bus.Emit(AgentEvent{Type: "test"})
}

func TestEventBusClose(t *testing.T) {
	bus := NewEventBus()
	ch := bus.Subscribe("sub1")
	bus.Close()

	// Channel should be closed
	select {
	case _, ok := <-ch:
		if ok {
			t.Error("channel should be closed")
		}
	default:
	}

	// Double close should not panic
	bus.Close()
}

func TestAgentStart(t *testing.T) {
	evt := AgentStart("sess-1", "gpt-4o", "/tmp")
	if evt.Type != "agent_start" {
		t.Errorf("type = %s, want agent_start", evt.Type)
	}
	if evt.Data["session_id"] != "sess-1" {
		t.Errorf("session_id = %v", evt.Data["session_id"])
	}
	if evt.Data["model"] != "gpt-4o" {
		t.Errorf("model = %v", evt.Data["model"])
	}
	if evt.Data["cwd"] != "/tmp" {
		t.Errorf("cwd = %v", evt.Data["cwd"])
	}
}

func TestAgentEnd(t *testing.T) {
	t.Run("with usage", func(t *testing.T) {
		usage := &types.Usage{PromptTokens: 100, CompletionTokens: 50, TotalTokens: 150}
		evt := AgentEnd("completed", usage)
		if evt.Type != "agent_end" {
			t.Errorf("type = %s", evt.Type)
		}
		if evt.Data["reason"] != "completed" {
			t.Errorf("reason = %v", evt.Data["reason"])
		}
		usageMap := evt.Data["usage"].(map[string]int)
		if usageMap["input_tokens"] != 100 {
			t.Errorf("input_tokens = %d", usageMap["input_tokens"])
		}
	})

	t.Run("without usage", func(t *testing.T) {
		evt := AgentEnd("timeout", nil)
		if evt.Data["usage"] != nil {
			t.Error("usage should be nil")
		}
	})
}

func TestTurnStart(t *testing.T) {
	evt := TurnStart(3)
	if evt.Type != "turn_start" {
		t.Errorf("type = %s", evt.Type)
	}
	if evt.Data["turn"] != 3 {
		t.Errorf("turn = %v", evt.Data["turn"])
	}
}

func TestTurnEnd(t *testing.T) {
	evt := TurnEnd(5)
	if evt.Type != "turn_end" {
		t.Errorf("type = %s", evt.Type)
	}
	if evt.Data["turn"] != 5 {
		t.Errorf("turn = %v", evt.Data["turn"])
	}
}

func TestMessageStart(t *testing.T) {
	evt := MessageStart("assistant")
	if evt.Type != "message_start" {
		t.Errorf("type = %s", evt.Type)
	}
	if evt.Data["role"] != "assistant" {
		t.Errorf("role = %v", evt.Data["role"])
	}
}

func TestMessageEnd(t *testing.T) {
	evt := MessageEnd("assistant")
	if evt.Type != "message_end" {
		t.Errorf("type = %s", evt.Type)
	}
}

func TestTextStart(t *testing.T) {
	evt := TextStart()
	if evt.Type != "text_start" {
		t.Errorf("type = %s", evt.Type)
	}
}

func TestTextDelta(t *testing.T) {
	evt := TextDelta("hello")
	if evt.Type != "text_delta" {
		t.Errorf("type = %s", evt.Type)
	}
	if evt.Data["text"] != "hello" {
		t.Errorf("text = %v", evt.Data["text"])
	}
}

func TestTextEnd(t *testing.T) {
	evt := TextEnd()
	if evt.Type != "text_end" {
		t.Errorf("type = %s", evt.Type)
	}
}

func TestThinkingStart(t *testing.T) {
	evt := ThinkingStart()
	if evt.Type != "thinking_start" {
		t.Errorf("type = %s", evt.Type)
	}
}

func TestThinkingDelta(t *testing.T) {
	evt := ThinkingDelta("thinking...")
	if evt.Type != "thinking_delta" {
		t.Errorf("type = %s", evt.Type)
	}
	if evt.Data["text"] != "thinking..." {
		t.Errorf("text = %v", evt.Data["text"])
	}
}

func TestThinkingEnd(t *testing.T) {
	evt := ThinkingEnd()
	if evt.Type != "thinking_end" {
		t.Errorf("type = %s", evt.Type)
	}
}

func TestToolCallStart(t *testing.T) {
	evt := ToolCallStart("bash", "call_1")
	if evt.Type != "toolcall_start" {
		t.Errorf("type = %s", evt.Type)
	}
	if evt.Data["tool_name"] != "bash" {
		t.Errorf("tool_name = %v", evt.Data["tool_name"])
	}
	if evt.Data["tool_id"] != "call_1" {
		t.Errorf("tool_id = %v", evt.Data["tool_id"])
	}
}

func TestToolCallDelta(t *testing.T) {
	evt := ToolCallDelta("arg_chunk")
	if evt.Type != "toolcall_delta" {
		t.Errorf("type = %s", evt.Type)
	}
	if evt.Data["text"] != "arg_chunk" {
		t.Errorf("text = %v", evt.Data["text"])
	}
}

func TestToolCallEnd(t *testing.T) {
	evt := ToolCallEnd("bash", "call_1", `{"command":"ls"}`)
	if evt.Type != "toolcall_end" {
		t.Errorf("type = %s", evt.Type)
	}
	if evt.Data["tool_name"] != "bash" {
		t.Errorf("tool_name = %v", evt.Data["tool_name"])
	}
	if evt.Data["args"] != `{"command":"ls"}` {
		t.Errorf("args = %v", evt.Data["args"])
	}
}

func TestToolStart(t *testing.T) {
	evt := ToolStart("bash")
	if evt.Type != "tool_start" {
		t.Errorf("type = %s", evt.Type)
	}
	if evt.Data["tool_name"] != "bash" {
		t.Errorf("tool_name = %v", evt.Data["tool_name"])
	}
}

func TestToolEnd(t *testing.T) {
	evt := ToolEnd("bash", "hello", "", 150)
	if evt.Type != "tool_end" {
		t.Errorf("type = %s", evt.Type)
	}
	if evt.Data["tool_name"] != "bash" {
		t.Errorf("tool_name = %v", evt.Data["tool_name"])
	}
	if evt.Data["result"] != "hello" {
		t.Errorf("result = %v", evt.Data["result"])
	}
	if evt.Data["duration"] != int64(150) {
		t.Errorf("duration = %v", evt.Data["duration"])
	}
}

func TestToolEndWithError(t *testing.T) {
	evt := ToolEnd("bash", "", "command not found", 50)
	if evt.Data["error"] != "command not found" {
		t.Errorf("error = %v", evt.Data["error"])
	}
}

func TestUsageEvent(t *testing.T) {
	evt := UsageEvent(100, 50, 10)
	if evt.Type != "usage" {
		t.Errorf("type = %s", evt.Type)
	}
	if evt.Data["input_tokens"] != 100 {
		t.Errorf("input_tokens = %v", evt.Data["input_tokens"])
	}
	if evt.Data["output_tokens"] != 50 {
		t.Errorf("output_tokens = %v", evt.Data["output_tokens"])
	}
	if evt.Data["cache_tokens"] != 10 {
		t.Errorf("cache_tokens = %v", evt.Data["cache_tokens"])
	}
}

func TestErrorEvent(t *testing.T) {
	evt := ErrorEvent("something went wrong")
	if evt.Type != "error" {
		t.Errorf("type = %s", evt.Type)
	}
	if evt.Data["message"] != "something went wrong" {
		t.Errorf("message = %v", evt.Data["message"])
	}
}

func TestEmitStreamingEvents(t *testing.T) {
	bus := NewEventBus()
	ch := bus.Subscribe("sub1")

	stream := make(chan types.StreamEvent, 10)
	stream <- types.StreamEvent{Type: "text_start"}
	stream <- types.StreamEvent{Type: "text_delta", Text: "hello"}
	stream <- types.StreamEvent{Type: "text_end"}
	close(stream)

	go EmitStreamingEvents(stream, bus)

	// Collect events until channel closes
	var received []AgentEvent
	done := make(chan struct{})
	go func() {
		for evt := range ch {
			received = append(received, evt)
		}
		close(done)
	}()

	// Wait for EmitStreamingEvents to finish, then close bus
	time.Sleep(100 * time.Millisecond)
	bus.Close()
	<-done

	if len(received) == 0 {
		t.Error("expected at least 1 event")
	} else {
		t.Logf("received %d events", len(received))
	}
}

func TestEmitStreamingEventsEmptyStream(t *testing.T) {
	bus := NewEventBus()
	stream := make(chan types.StreamEvent)
	close(stream)

	go EmitStreamingEvents(stream, bus)
	// Should not panic or block
}

func TestEmitStreamingEventsStopEvent(t *testing.T) {
	bus := NewEventBus()
	ch := bus.Subscribe("sub1")
	stream := make(chan types.StreamEvent, 1)
	stream <- types.StreamEvent{Type: "stop"}
	close(stream)

	go EmitStreamingEvents(stream, bus)

	// stop event should not produce any AgentEvent
	time.Sleep(50 * time.Millisecond)
	select {
	case <-ch:
		t.Error("stop event should not emit anything")
	default:
	}
}
