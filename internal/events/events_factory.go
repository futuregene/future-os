package events

import (
	"github.com/huichen/xihu/pkg/types"
)

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
