package agent

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"os"
	"strings"
	"sync"
	"time"
	"unicode/utf8"

	"github.com/huichen/xihu/internal/events"
	"github.com/huichen/xihu/pkg/types"
)

// ANSI terminal colors for verbose output.
const (
	cReset   = "\033[0m"
	cBold    = "\033[1m"
	cRed     = "\033[31m"
	cGreen   = "\033[32m"
	cYellow  = "\033[33m"
	cBlue    = "\033[34m"
	cMagenta = "\033[35m"
)

// DefaultMaxTurns is the default limit of agent turns per run
const DefaultMaxTurns = 50

// Loop runs the agent loop: call LLM → execute tools → repeat until done
type Loop struct {
	Provider     types.LLMProvider
	Model        string
	SystemPrompt string
	Tools        []types.AgentTool
	Config       types.AgentConfig

	// Verbose enables tool execution logging to stderr.
	Verbose bool

	// EventBus is an optional event bus for fine-grained streaming events.
	// If nil, no events are emitted.
	EventBus *events.EventBus

	// SessionID is used for event metadata.
	SessionID string

	// SteeringQueue is a buffered channel of steering messages (injected before each turn).
	SteeringQueue chan string

	// FollowUpQueue is a buffered channel of follow-up messages (injected after agent finishes).
	FollowUpQueue chan string

	// SteeringMode controls steering behavior: "all" (default) or "one-at-a-time".
	SteeringMode string

	// FollowUpMode controls follow-up behavior: "all" (default) or "one-at-a-time".
	FollowUpMode string

	// ParallelTools enables concurrent execution of multiple tool calls.
	ParallelTools bool
}

// NewLoop creates a new agent loop with defaults
func NewLoop(provider types.LLMProvider, model string) *Loop {
	return &Loop{
		Provider:      provider,
		Model:         model,
		SteeringQueue: make(chan string, 64),
		FollowUpQueue: make(chan string, 64),
		SteeringMode:  "all",
		FollowUpMode:  "all",
		Config: types.AgentConfig{
			MaxTurns: DefaultMaxTurns,
		},
	}
}

// RunStreamingWithMessages runs the agent loop with pre-existing messages,
// returning final text and all messages.
func (l *Loop) RunStreamingWithMessages(ctx context.Context, messages []types.Message, onText func(string)) (string, []types.Message, error) {
	// Validate: last message must not be from assistant (the model responds next)
	if len(messages) > 0 && messages[len(messages)-1].Role == "assistant" {
		return "", messages, fmt.Errorf("agent: last message must not be from assistant (the model responds next)")
	}

	// Emit agent_start event
	if l.EventBus != nil {
		l.EventBus.Emit(events.AgentStart(l.SessionID, l.Model, ""))
	}

	toolDefs := make([]types.ToolDef, 0, len(l.Tools))
	for _, t := range l.Tools {
		toolDefs = append(toolDefs, t.Def)
	}

	maxTurns := l.Config.MaxTurns
	if maxTurns <= 0 {
		maxTurns = DefaultMaxTurns
	}

	var lastError error
	for turn := 0; turn < maxTurns; turn++ {
		// Check cancellation / abort signal before each turn
		if err := ctx.Err(); err != nil {
			if l.EventBus != nil {
				l.EventBus.Emit(events.AgentEnd("cancelled", nil))
			}
			return "", messages, fmt.Errorf("agent: context cancelled at turn %d: %w", turn, err)
		}

		// Emit turn_start
		if l.EventBus != nil {
			l.EventBus.Emit(events.TurnStart(turn))
		}

		// Drain steering queue and inject as user messages
		messages = l.drainSteering(messages)

		// Apply context transform if configured
		workMessages := messages
		if l.Config.TransformContext != nil {
			workMessages = l.Config.TransformContext(messages, "")
		}

		// Emit message_start for assistant
		if l.EventBus != nil {
			l.EventBus.Emit(events.MessageStart("assistant"))
		}

		eventsCh, err := l.Provider.StreamChat(l.Model, workMessages, toolDefs, l.SystemPrompt)
		if err != nil {
			if l.EventBus != nil {
				l.EventBus.Emit(events.ErrorEvent(err.Error()))
			}
			lastError = err
			// Check if retryable
			if l.Config.MaxRetries > 0 {
				continue // retry on next turn
			}
			// Emit agent_end
			if l.EventBus != nil {
				l.EventBus.Emit(events.AgentEnd("error", nil))
			}
			return "", messages, fmt.Errorf("turn %d: %w", turn, err)
		}

		var fullText string
		var reasoningText string
		var toolCalls []types.ToolCall
		var totalUsage types.Usage
		var outputStarted bool

		// Bridge LLM stream events to EventBus
		for event := range eventsCh {
			switch event.Type {
		case "thinking_start":
			if l.Verbose {
				fmt.Fprintf(os.Stderr, "\n%s[thinking]%s ", cMagenta, cReset)
			}
		case "thinking_delta":
			reasoningText += event.Text
			if l.Verbose {
				fmt.Fprint(os.Stderr, event.Text)
			}
		case "thinking_end":
			if l.Verbose {
				fmt.Fprintln(os.Stderr)
			}
		case "text", "text_delta":
			fullText += event.Text
			if l.Verbose && !outputStarted {
				outputStarted = true
				fmt.Fprintln(os.Stderr)
			}
				if onText != nil {
					onText(event.Text)
				}
				if l.EventBus != nil {
					l.EventBus.Emit(events.TextDelta(event.Text))
				}
			case "tool_call", "toolcall_end":
				if event.ToolCall != nil {
					toolCalls = append(toolCalls, *event.ToolCall)
					if l.EventBus != nil {
						l.EventBus.Emit(events.ToolCallEnd(
							event.ToolCall.Function.Name,
							event.ToolCall.ID,
							string(event.ToolCall.Function.Arguments),
						))
					}
				}
			case "usage":
				if event.Usage != nil {
					totalUsage = *event.Usage
					if l.EventBus != nil {
						l.EventBus.Emit(events.UsageEvent(
							event.Usage.PromptTokens,
							event.Usage.CompletionTokens,
							0,
						))
					}
				}
			case "stop":
				// done
		case "error":
			lastError = fmt.Errorf("stream error: %s", event.Text)
			if l.EventBus != nil {
				l.EventBus.Emit(events.ErrorEvent(event.Text))
			}
			}
		}

		// Check for stream errors before processing results
		if lastError != nil {
			if l.EventBus != nil {
				l.EventBus.Emit(events.AgentEnd("error", nil))
			}
			return "", messages, lastError
		}

		// Emit message_end
		if l.EventBus != nil {
			l.EventBus.Emit(events.MessageEnd("assistant"))
		}

		content, _ := json.Marshal([]types.TextContent{{Type: "text", Text: fullText}})
		assistantMsg := types.Message{
			Role:             "assistant",
			Content:          content,
			ToolCalls:        toolCalls,
			ReasoningContent: reasoningText,
		}
		messages = append(messages, assistantMsg)

		// Check stop condition after this turn
		if l.Config.StopCondition != nil && l.Config.StopCondition(messages, fullText) {
			if l.EventBus != nil {
				l.EventBus.Emit(events.AgentEnd("stop_condition", &totalUsage))
			}
			return fullText, messages, nil
		}

		// If no tool calls, check if follow-up queue has pending messages before returning
		if len(toolCalls) == 0 {
			if len(l.FollowUpQueue) > 0 {
				messages = l.drainFollowUp(messages)
				if l.EventBus != nil {
					l.EventBus.Emit(events.TurnEnd(turn))
				}
				continue // follow-up messages waiting; continue the loop
			}
			if l.EventBus != nil {
				l.EventBus.Emit(events.AgentEnd("complete", &totalUsage))
			}
			return fullText, messages, nil
		}

		// Execute tool calls (parallel or sequential)
		l.executeTools(ctx, turn, toolCalls, &messages)

		if l.EventBus != nil {
			l.EventBus.Emit(events.TurnEnd(turn))
		}

		// Reset error for next turn
		lastError = nil
	}

	if l.EventBus != nil {
		l.EventBus.Emit(events.AgentEnd("max_turns", nil))
	}

	if lastError != nil {
		return "", messages, fmt.Errorf("exceeded max turns (%d) after retry errors: %w", maxTurns, lastError)
	}
	return "", messages, fmt.Errorf("exceeded max turns (%d)", maxTurns)
}

// RunStreaming runs the agent loop with streaming output (new session)
func (l *Loop) RunStreaming(ctx context.Context, userPrompt string, onText func(string)) (string, error) {
	messages := []types.Message{
		newSystemMessage(l.SystemPrompt),
		newUserMessage(userPrompt),
	}
	result, _, err := l.RunStreamingWithMessages(ctx, messages, onText)
	return result, err
}

// executeTools runs tool calls either sequentially or in parallel.
func (l *Loop) executeTools(ctx context.Context, turn int, toolCalls []types.ToolCall, messages *[]types.Message) {
	if l.ParallelTools && len(toolCalls) > 1 {
		toolResults := make([]struct {
			result    string
			err       error
			toolName  string
			duration  time.Duration
		}, len(toolCalls))

		var wg sync.WaitGroup
		for i, tc := range toolCalls {
			wg.Add(1)
			go func(idx int, call types.ToolCall) {
				defer wg.Done()

				if ctx.Err() != nil {
					toolResults[idx].err = fmt.Errorf("context cancelled during tool execution at turn %d: %w", turn, ctx.Err())
					return
				}

				start := time.Now()
				r, err := l.executeTool(call)
				toolResults[idx].result = r
				toolResults[idx].err = err
				toolResults[idx].toolName = call.Function.Name
				toolResults[idx].duration = time.Since(start)
			}(i, tc)
		}
		wg.Wait()

		for i, tc := range toolCalls {
			if l.Verbose {
				toolLog(tc.Function.Name, tc.Function.Arguments, toolResults[i].err, toolResults[i].duration)
			}
			emitToolEnd(l.EventBus, toolResults[i].toolName, toolResults[i].result, toolResults[i].err, toolResults[i].duration)
			toolMsg := newToolResult(tc.ID, toolResults[i].result, toolResults[i].err)
			*messages = append(*messages, toolMsg)
		}
	} else {
		for _, tc := range toolCalls {
			if err := ctx.Err(); err != nil {
				if l.Verbose {
				fmt.Fprintf(os.Stderr, "\n[tool] %s: context cancelled\n", tc.Function.Name)
			}
				break
			}

			start := time.Now()
			result, err := l.executeTool(tc)
			duration := time.Since(start)

			if l.Verbose {
				toolLog(tc.Function.Name, tc.Function.Arguments, err, duration)
			}
			emitToolEnd(l.EventBus, tc.Function.Name, result, err, duration)
			toolMsg := newToolResult(tc.ID, result, err)
			*messages = append(*messages, toolMsg)
		}
	}
}

// emitToolEnd emits a tool_end event if the event bus is set.
func emitToolEnd(bus *events.EventBus, name, result string, execErr error, duration time.Duration) {
	if bus == nil {
		return
	}
	errStr := ""
	if execErr != nil {
		errStr = execErr.Error()
	}
	bus.Emit(events.ToolEnd(name, result, errStr, duration.Milliseconds()))
}

// drainSteering drains all pending messages from the steering queue and
// returns the message list with steering messages appended as user messages.
func (l *Loop) drainSteering(messages []types.Message) []types.Message {
	drained := 0
	for {
		select {
		case msg := <-l.SteeringQueue:
			messages = append(messages, newUserMessage(msg))
			drained++
			if l.SteeringMode == "one-at-a-time" {
				if drained > 0 {
					log.Printf("[steering] injected %d steering message(s)", drained)
				}
				return messages
			}
		default:
			if drained > 0 {
				log.Printf("[steering] injected %d steering message(s)", drained)
			}
			return messages
		}
	}
}

// drainFollowUp drains messages from the follow-up queue.
func (l *Loop) drainFollowUp(messages []types.Message) []types.Message {
	drained := 0
	for {
		select {
		case msg := <-l.FollowUpQueue:
			messages = append(messages, newUserMessage(msg))
			drained++
			if l.FollowUpMode == "one-at-a-time" {
				if drained > 0 {
					log.Printf("[followup] injected %d follow-up message(s)", drained)
				}
				return messages
			}
		default:
			if drained > 0 {
				log.Printf("[followup] injected %d follow-up message(s)", drained)
			}
			return messages
		}
	}
}

// ClearQueues drains all pending messages from both queues without injecting them.
func (l *Loop) ClearQueues() {
	for {
		select {
		case <-l.SteeringQueue:
		default:
			goto doneSteering
		}
	}
doneSteering:
	for {
		select {
		case <-l.FollowUpQueue:
		default:
			return
		}
	}
}

// PendingMessageCount returns the total pending messages in both queues.
func (l *Loop) PendingMessageCount() int {
	return len(l.SteeringQueue) + len(l.FollowUpQueue)
}

func (l *Loop) executeTool(tc types.ToolCall) (string, error) {
	for _, t := range l.Tools {
		if t.Def.Function.Name == tc.Function.Name {
			return t.Handler(tc.Function.Arguments)
		}
	}
	return "", fmt.Errorf("tool %s not found", tc.Function.Name)
}

func newSystemMessage(content string) types.Message {
	tc := types.TextContent{Type: "text", Text: content}
	b, _ := json.Marshal([]types.TextContent{tc})
	return types.Message{Role: "system", Content: b}
}

func newUserMessage(content string) types.Message {
	tc := types.TextContent{Type: "text", Text: content}
	b, _ := json.Marshal([]types.TextContent{tc})
	return types.Message{Role: "user", Content: b}
}

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
