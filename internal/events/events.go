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
