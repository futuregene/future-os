// Package events provides the AgentEvent system with a pub/sub EventBus
// for fine-grained streaming events during agent execution.
//
// Event types (matching TS pi):
//
//	agent_start, agent_end       — agent lifecycle
//	turn_start, turn_end         — turn boundaries
//	message_start, message_end   — message boundaries
//	text_start, text_delta, text_end           — streaming text
//	thinking_start, thinking_delta, thinking_end — streaming thinking
//	toolcall_start, toolcall_delta, toolcall_end — streaming tool calls
//	tool_start, tool_end         — tool execution
//	usage                        — token usage
//	error                        — error event
package events

import (
	"fmt"
	"sync"
	"time"

	"github.com/huichen/xihu/pkg/types"
)

// AgentEvent is a fine-grained event emitted during agent execution.
type AgentEvent struct {
	Type      string                 `json:"type"`
	Data      map[string]interface{} `json:"data"`
	Timestamp time.Time              `json:"timestamp"`
}

// EventBus is a simple pub/sub event bus for agent events.
// Supports two listener patterns:
//   1. Channel-based (Subscribe/Unsubscribe): returns a channel, subscriber drains it
//   2. Callback-based (OnEvent/OffEvent): registers a callback, invoked synchronously on Emit
// Slow consumers in channel-based mode may drop events if buffer is full.
// Callback-based listeners are always invoked synchronously (no drop).
type EventBus struct {
	subscribers      map[string]chan AgentEvent
	callbackListeners map[string]callbackEntry            // indexed by listener ID
	starListeners     map[string]func(AgentEvent)         // "*" wildcard listeners
	nextID            int                                 // auto-increment ID for callback listeners
	mu                sync.RWMutex
	closed            bool
}

// callbackEntry holds a single callback listener with its event type filter.
type callbackEntry struct {
	eventType string
	fn        func(AgentEvent)
}

// NewEventBus creates a new EventBus ready for use.
func NewEventBus() *EventBus {
	return &EventBus{
		subscribers: make(map[string]chan AgentEvent),
	}
}

// Subscribe adds a subscriber and returns a receive-only channel.
// The channel has a buffer of 64 events. Returns nil if the bus is closed.
func (b *EventBus) Subscribe(id string) <-chan AgentEvent {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.closed {
		return nil
	}
	ch := make(chan AgentEvent, 64)
	b.subscribers[id] = ch
	return ch
}

// Unsubscribe removes a subscriber and closes its channel.
func (b *EventBus) Unsubscribe(id string) {
	b.mu.Lock()
	defer b.mu.Unlock()
	if ch, ok := b.subscribers[id]; ok {
		delete(b.subscribers, id)
		close(ch)
	}
}

// OnEvent registers a callback-based listener for a specific event type.
// Use "*" as eventType to listen to all events (wildcard).
// Returns a listener ID for later removal via OffEvent.
// TS pi-mono: agent.on('event_type', callback) pattern.
func (b *EventBus) OnEvent(eventType string, fn func(AgentEvent)) string {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.closed {
		return ""
	}

	b.nextID++
	id := fmt.Sprintf("listener_%d", b.nextID)

	if b.callbackListeners == nil {
		b.callbackListeners = make(map[string]callbackEntry)
	}
	if b.starListeners == nil {
		b.starListeners = make(map[string]func(AgentEvent))
	}

	if eventType == "*" {
		b.starListeners[id] = fn
	} else {
		b.callbackListeners[id] = callbackEntry{eventType: eventType, fn: fn}
	}
	return id
}

// OffEvent removes a callback-based listener by ID.
// The ID is the opaque string returned by OnEvent.
// If id is empty or not found, this is a no-op.
func (b *EventBus) OffEvent(id string) {
	if id == "" {
		return
	}
	b.mu.Lock()
	defer b.mu.Unlock()
	delete(b.callbackListeners, id)
	delete(b.starListeners, id)
}

// Emit sends an event to all subscribers. Non-blocking: if a subscriber's
// buffer is full, the event is silently dropped for that subscriber.
// Callback-based listeners are invoked synchronously (never dropped).
func (b *EventBus) Emit(event AgentEvent) {
	b.mu.RLock()
	// Snapshot listeners under lock to avoid holding lock during callbacks.
	closed := b.closed
	var chans []chan AgentEvent
	for _, ch := range b.subscribers {
		chans = append(chans, ch)
	}
	var cbs []func(AgentEvent)
	for _, entry := range b.callbackListeners {
		if entry.eventType == event.Type {
			cbs = append(cbs, entry.fn)
		}
	}
	for _, fn := range b.starListeners {
		cbs = append(cbs, fn)
	}
	b.mu.RUnlock()

	if closed {
		return
	}

	event.Timestamp = time.Now()

	// Channel subscribers (non-blocking, may drop)
	for _, ch := range chans {
		select {
		case ch <- event:
		default:
			// drop for slow consumer
		}
	}

	// Callback listeners (synchronous, never dropped)
	for _, fn := range cbs {
		fn(event)
	}
}

// Close shuts down the event bus and closes all subscriber channels.
// Subsequent calls to Emit or Subscribe are no-ops.
func (b *EventBus) Close() {
	b.mu.Lock()
	defer b.mu.Unlock()
	if b.closed {
		return
	}
	b.closed = true
	for _, ch := range b.subscribers {
		close(ch)
	}
	b.subscribers = nil
	b.callbackListeners = nil
	b.starListeners = nil
}

// ---------------------------------------------------------------------------
// Convenience constructors for common event types
// ---------------------------------------------------------------------------

// AgentStart creates an agent_start event.
func AgentStart(sessionID, model, cwd string) AgentEvent {
	return AgentEvent{
		Type: "agent_start",
		Data: map[string]interface{}{
			"session_id": sessionID,
			"model":      model,
			"cwd":        cwd,
		},
	}
}

// AgentEnd creates an agent_end event.
func AgentEnd(reason string, usage *types.Usage, stopReason ...string) AgentEvent {
	data := map[string]interface{}{"reason": reason}
	if usage != nil {
		data["usage"] = map[string]int{
			"input_tokens":       usage.PromptTokens,
			"output_tokens":      usage.CompletionTokens,
			"cache_read_tokens":  usage.CacheReadTokens,
			"cache_write_tokens": usage.CacheWriteTokens,
			"total_tokens":       usage.TotalTokens,
		}
	}
	if len(stopReason) > 0 && stopReason[0] != "" {
		data["stop_reason"] = stopReason[0]
	}
	return AgentEvent{Type: "agent_end", Data: data}
}

// TurnStart creates a turn_start event.
func TurnStart(turn int) AgentEvent {
	return AgentEvent{
		Type: "turn_start",
		Data: map[string]interface{}{"turn": turn},
	}
}

// TurnEnd creates a turn_end event.
func TurnEnd(turn int) AgentEvent {
	return AgentEvent{
		Type: "turn_end",
		Data: map[string]interface{}{"turn": turn},
	}
}

// MessageStart creates a message_start event.
func MessageStart(role string) AgentEvent {
	return AgentEvent{
		Type: "message_start",
		Data: map[string]interface{}{"role": role},
	}
}

// MessageEnd creates a message_end event.
func MessageEnd(role string) AgentEvent {
	return AgentEvent{
		Type: "message_end",
		Data: map[string]interface{}{"role": role},
	}
}

// TextStart creates a text_start event.
func TextStart() AgentEvent {
	return AgentEvent{Type: "text_start", Data: map[string]interface{}{}}
}

// TextDelta creates a text_delta event.
func TextDelta(text string) AgentEvent {
	return AgentEvent{
		Type: "text_delta",
		Data: map[string]interface{}{"text": text},
	}
}

// TextEnd creates a text_end event.
func TextEnd() AgentEvent {
	return AgentEvent{Type: "text_end", Data: map[string]interface{}{}}
}

// ThinkingStart creates a thinking_start event.
func ThinkingStart() AgentEvent {
	return AgentEvent{Type: "thinking_start", Data: map[string]interface{}{}}
}

// ThinkingDelta creates a thinking_delta event.
func ThinkingDelta(text string) AgentEvent {
	return AgentEvent{
		Type: "thinking_delta",
		Data: map[string]interface{}{"text": text},
	}
}

// ThinkingEnd creates a thinking_end event.
func ThinkingEnd() AgentEvent {
	return AgentEvent{Type: "thinking_end", Data: map[string]interface{}{}}
}

// ToolCallStart creates a toolcall_start event.
func ToolCallStart(name, id string) AgentEvent {
	return AgentEvent{
		Type: "toolcall_start",
		Data: map[string]interface{}{
			"tool_name": name,
			"tool_id":   id,
		},
	}
}

// ToolCallDelta creates a toolcall_delta event.
func ToolCallDelta(text string) AgentEvent {
	return AgentEvent{
		Type: "toolcall_delta",
		Data: map[string]interface{}{"text": text},
	}
}

// ToolCallEnd creates a toolcall_end event.
func ToolCallEnd(name, id, args string) AgentEvent {
	return AgentEvent{
		Type: "toolcall_end",
		Data: map[string]interface{}{
			"tool_name": name,
			"tool_id":   id,
			"args":      args,
		},
	}
}

// ToolStart creates a tool_start event.
func ToolStart(id, name string) AgentEvent {
	return AgentEvent{
		Type: "tool_start",
		Data: map[string]interface{}{"tool_call_id": id, "tool_name": name},
	}
}

// ToolEnd creates a tool_end event.
func ToolEnd(name, result, toolErr string, durationMs int64) AgentEvent {
	return AgentEvent{
		Type: "tool_end",
		Data: map[string]interface{}{
			"tool_name":  name,
			"result":     result,
			"error":      toolErr,
			"duration":   durationMs,
		},
	}
}

// UsageEvent creates a usage event.
func UsageEvent(inputTokens, outputTokens, cacheReadTokens, cacheWriteTokens int) AgentEvent {
	return AgentEvent{
		Type: "usage",
		Data: map[string]interface{}{
			"input_tokens":        inputTokens,
			"output_tokens":       outputTokens,
			"cache_read_tokens":   cacheReadTokens,
			"cache_write_tokens":  cacheWriteTokens,
		},
	}
}

// ErrorEvent creates an error event.
func ErrorEvent(message string) AgentEvent {
	return AgentEvent{
		Type: "error",
		Data: map[string]interface{}{"message": message},
	}
}

// CompactionStart creates a compaction_start event.
// reason is "auto" (token threshold) or "manual" (/compact command).
func CompactionStart(reason string) AgentEvent {
	return AgentEvent{
		Type: "compaction_start",
		Data: map[string]interface{}{
			"reason": reason,
		},
	}
}

// CompactionEnd creates a compaction_end event with optional summary data.
// Set aborted=true if compaction was cancelled.
func CompactionEnd(tokensBefore int, summary string, aborted bool, reason string) AgentEvent {
	return AgentEvent{
		Type: "compaction_end",
		Data: map[string]interface{}{
			"tokens_before": tokensBefore,
			"summary":       summary,
			"aborted":       aborted,
			"reason":        reason,
		},
	}
}

// AutoRetryStart creates an auto_retry_start event with attempt info.
func AutoRetryStart(attempt, maxAttempts, delayMs int) AgentEvent {
	return AgentEvent{
		Type: "auto_retry_start",
		Data: map[string]interface{}{
			"attempt":      attempt,
			"max_attempts": maxAttempts,
			"delay_ms":     delayMs,
		},
	}
}

// AutoRetryEnd creates an auto_retry_end event.
// Pass optional (success bool, attempt int, finalError string) for retry failure info (TS pi-mono).
func AutoRetryEnd(failureInfo ...interface{}) AgentEvent {
	e := AgentEvent{Type: "auto_retry_end", Data: map[string]interface{}{}}
	if len(failureInfo) >= 1 {
		if success, ok := failureInfo[0].(bool); ok {
			e.Data["success"] = success
		}
	}
	if len(failureInfo) >= 2 {
		if attempt, ok := failureInfo[1].(int); ok {
			e.Data["attempt"] = attempt
		}
	}
	if len(failureInfo) >= 3 {
		if finalError, ok := failureInfo[2].(string); ok {
			e.Data["final_error"] = finalError
		}
	}
	return e
}

// EmitStreamingEvents bridges LLM StreamEvent channel output into AgentEvent
// emissions on the given EventBus. This runs synchronously in the caller's
// goroutine (blocking until the stream channel closes).
func EmitStreamingEvents(stream <-chan types.StreamEvent, bus *EventBus) {
	for evt := range stream {
		switch evt.Type {
		case "text_start":
			bus.Emit(TextStart())
		case "text_delta":
			bus.Emit(TextDelta(evt.Text))
		case "text_end":
			bus.Emit(TextEnd())
		case "thinking_start":
			bus.Emit(ThinkingStart())
		case "thinking_delta":
			bus.Emit(ThinkingDelta(evt.Text))
		case "thinking_end":
			bus.Emit(ThinkingEnd())
		case "toolcall_start":
			bus.Emit(ToolCallStart(evt.ToolName, evt.ToolID))
		case "toolcall_delta":
			bus.Emit(ToolCallDelta(evt.Text))
		case "toolcall_end":
			if evt.ToolCall != nil {
				bus.Emit(ToolCallEnd(
					evt.ToolCall.Function.Name,
					evt.ToolCall.ID,
					string(evt.ToolCall.Function.Arguments),
				))
			}
		case "tool_call":
			// legacy: treat as complete tool call
			if evt.ToolCall != nil {
				bus.Emit(ToolCallEnd(
					evt.ToolCall.Function.Name,
					evt.ToolCall.ID,
					string(evt.ToolCall.Function.Arguments),
				))
			}
		case "usage":
			if evt.Usage != nil {
				bus.Emit(UsageEvent(
					evt.Usage.PromptTokens,
					evt.Usage.CompletionTokens,
					evt.Usage.CacheReadTokens,
					evt.Usage.CacheWriteTokens,
				))
			}
		case "error":
			bus.Emit(ErrorEvent(evt.Text))
		// "stop" is handled by the caller as it indicates end of stream
		}
	}
}
