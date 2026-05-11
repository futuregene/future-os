package agent

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"os"
	"strings"
	"time"
	"unicode/utf8"

	"github.com/huichen/xihu/pkg/types"
)

// drainSteering drains all pending messages from the steering queue and
// returns the message list with steering messages appended as user messages.
func (l *Loop) drainSteering(messages []types.AgentMessage) []types.AgentMessage {
	msgs := l.SteeringQueue.Drain()
	for _, msg := range msgs {
		messages = append(messages, newUserAgentMessage(msg))
	}
	if len(msgs) > 0 {
		log.Printf("[steering] injected %d steering message(s)", len(msgs))
	}
	return messages
}

// drainFollowUp drains messages from the follow-up queue.
func (l *Loop) drainFollowUp(messages []types.AgentMessage) []types.AgentMessage {
	msgs := l.FollowUpQueue.Drain()
	for _, msg := range msgs {
		messages = append(messages, newUserAgentMessage(msg))
	}
	if len(msgs) > 0 {
		log.Printf("[followup] injected %d follow-up message(s)", len(msgs))
	}
	return messages
}

// ClearQueues drains all pending messages from both queues without injecting them.
func (l *Loop) ClearQueues() {
	l.SteeringQueue.Clear()
	l.FollowUpQueue.Clear()
}

// QueuedCounts returns separate steering and follow-up queue lengths.
func (l *Loop) QueuedCounts() (steering, followUp int) {
	return l.SteeringQueue.Len(), l.FollowUpQueue.Len()
}

// PendingMessageCount returns the total pending messages in both queues.
func (l *Loop) PendingMessageCount() int {
	return l.SteeringQueue.Len() + l.FollowUpQueue.Len()
}

// DrainQueues drains all pending messages from both queues and returns them.
func (l *Loop) DrainQueues() []string {
	var msgs []string
	msgs = append(msgs, l.SteeringQueue.Drain()...)
	msgs = append(msgs, l.FollowUpQueue.Drain()...)
	return msgs
}

func (l *Loop) executeTool(tc types.ToolCall) (string, error) {
	for _, t := range l.Tools {
		if t.Def.Function.Name == tc.Function.Name {
			return t.Handler(tc.Function.Arguments)
		}
	}
	return "", fmt.Errorf("tool %s not found", tc.Function.Name)
}

func newSystemAgentMessage(content string) types.AgentMessage {
	return types.AgentMessage{Role: "system", Content: []types.ContentBlock{types.TextBlock{Text: content}}}
}

func newUserAgentMessage(content string) types.AgentMessage {
	return types.AgentMessage{Role: "user", Content: []types.ContentBlock{types.TextBlock{Text: content}}}
}

func newToolAgentResult(callID, result string, err error) types.AgentMessage {
	text := result
	if err != nil {
		text = fmt.Sprintf("Error: %s", err.Error())
	}
	return types.AgentMessage{
		Role:       "tool",
		Content:    []types.ContentBlock{types.TextBlock{Text: text}},
		ToolCallID: callID,
	}
}

// newSystemMessage is kept for backward compatibility with code that still uses Message.
func newSystemMessage(content string) types.Message {
	tc := types.TextContent{Type: "text", Text: content}
	b, _ := json.Marshal([]types.TextContent{tc})
	return types.Message{Role: "system", Content: b}
}

// newUserMessage is kept for backward compatibility with code that still uses Message.
func newUserMessage(content string) types.Message {
	tc := types.TextContent{Type: "text", Text: content}
	b, _ := json.Marshal([]types.TextContent{tc})
	return types.Message{Role: "user", Content: b}
}

// newToolResult is kept for backward compatibility with code that still uses Message.
func newToolResult(callID, result string, err error) types.Message {
	text := result
	if err != nil {
		text = fmt.Sprintf("Error: %s", err.Error())
	}
	tc := types.TextContent{Type: "text", Text: text}
	b, _ := json.Marshal([]types.TextContent{tc})
	return types.Message{
		Role:       "tool",
		Content:    b,
		ToolCallID: callID,
	}
}

func toolLog(name string, args json.RawMessage, err error, d time.Duration) {
	tag := "[tool]"
	color := cGreen
	if err != nil {
		color = cRed
	}
	// Detect skill reads
	if name == "read" && args != nil {
		var p struct{ FilePath string `json:"file_path"` }
		if json.Unmarshal(args, &p) == nil && strings.Contains(p.FilePath, "SKILL.md") {
			tag = "[skill]"
			color = cBlue
		}
	}
	if err != nil {
		fmt.Fprintf(os.Stderr, "\n%s%s%s %s %-12s %6s  %s\n", color, tag, cReset, "✗", name, d.Round(time.Millisecond), err)
	} else {
		fmt.Fprintf(os.Stderr, "\n%s%s%s %s %-12s %6s\n", color, tag, cReset, "✓", name, d.Round(time.Millisecond))
	}
}

// Interrupt cancels the current streaming run and queues a steering message.
// This implements the TS pi interrupt pattern: push to SteeringQueue, then abort
// the current LLM stream. The agent loop will pick up the steering message
// at the start of the next turn after the current tool calls complete.
//
// DEPRECATED: Use Steer(text) for "inject without abort" or Abort() for "abort without message".
// Interrupt is kept for backward compatibility (steer + abort).
func (l *Loop) Interrupt(message string) {
	l.mu.Lock()
	defer l.mu.Unlock()

	// Queue the steering message
	l.SteeringQueue.Enqueue(message)

	// Cancel the current context if streaming
	if l.interruptFn != nil {
		l.interruptFn()
		l.interruptFn = nil
	}
}

// Steer injects a steering message into the agent loop without aborting
// the current LLM stream. The message will be picked up at the start
// of the next turn. TS pi-mono equivalent: Enter during streaming.
func (l *Loop) Steer(message string) {
	l.SteeringQueue.Enqueue(message)
}

// FollowUp injects a follow-up message for after the agent finishes the current
// turn. TS pi-mono equivalent: Alt+Enter queues message for later delivery.
func (l *Loop) FollowUp(message string) {
	l.FollowUpQueue.Enqueue(message)
}

// Abort cancels the current LLM stream without queuing a message.
// Useful for Escape key during streaming (pure abort, no new message).
func (l *Loop) Abort() {
	l.mu.Lock()
	defer l.mu.Unlock()

	if l.interruptFn != nil {
		l.interruptFn()
		l.interruptFn = nil
	}
}

// setupTurnContext creates a fresh cancellable context from baseCtx
// and registers its cancel function as the interrupt target.
func (l *Loop) setupTurnContext(baseCtx context.Context) context.Context {
	ctx, cancel := context.WithCancel(baseCtx)
	l.mu.Lock()
	l.interruptFn = cancel
	l.mu.Unlock()
	return ctx
}

func runeTruncate(s string, n int) string {
	r := []rune(s)
	if len(r) <= n {
		return s
	}
	return string(r[:n]) + "..."
}

func runeTruncateBytes(s string, maxBytes int) string {
	if len(s) <= maxBytes {
		return s
	}
	// Walk runes and stop when we exceed maxBytes
	pos := 0
	for pos < len(s) {
		_, size := utf8.DecodeRuneInString(s[pos:])
		if pos+size > maxBytes {
			break
		}
		pos += size
	}
	if pos == 0 {
		return "..."
	}
	return s[:pos] + "..."
}
