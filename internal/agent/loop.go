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

	"github.com/huichen/xihu/internal/compaction"
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

	// LastCompactionResult holds the result of the most recent compaction (set by TransformContext).
	LastCompactionResult *compaction.CompactionResult

	// ParallelTools enables concurrent execution of multiple tool calls.
	ParallelTools bool

	// interruptFn is set during streaming to allow external interruption.
	interruptFn context.CancelFunc
	mu          sync.Mutex
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

	// Save parent context for restarts on interrupt
	baseCtx := ctx

	// Create cancellable context for interrupt support
	ctx, cancel := context.WithCancel(baseCtx)
	defer cancel()

	l.mu.Lock()
	l.interruptFn = cancel
	l.mu.Unlock()
	defer func() {
		l.mu.Lock()
		l.interruptFn = nil
		l.mu.Unlock()
	}()

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
	var lastStopReason string // Anthropic stop_reason preserved across turns
	retryAttempt := 0
	for turn := 0; turn < maxTurns; turn++ {
		// Drain steering queue FIRST (before checking context cancellation).
		// This ensures interrupt messages are not lost when context is cancelled.
		steeringBefore := len(l.SteeringQueue)
		messages = l.drainSteering(messages)

		// Check cancellation: only exit if no steering was just drained.
		// If steering was drained, continue the turn to process the interrupt message.
		if err := ctx.Err(); err != nil {
			if steeringBefore == 0 {
				// Pure interrupt (no message) → exit cleanly
				if l.EventBus != nil {
					l.EventBus.Emit(events.AgentEnd("interrupted", nil))
				}
				return "", messages, nil
			}
			// Steering message was queued → create fresh context and continue
			ctx = l.setupTurnContext(baseCtx)
		}

		// Emit turn_start
		if l.EventBus != nil {
			l.EventBus.Emit(events.TurnStart(turn))
		}

		// Apply context transform if configured (e.g., compaction)
		workMessages := messages
		if l.Config.TransformContext != nil {
			beforeLen := len(workMessages)
			if l.EventBus != nil {
				l.EventBus.Emit(events.CompactionStart("auto"))
			}
			workMessages = l.Config.TransformContext(messages, "")
			if l.EventBus != nil {
				if len(workMessages) < beforeLen {
					tokensBefore := 0
					summary := ""
					if l.LastCompactionResult != nil {
						tokensBefore = l.LastCompactionResult.TokensBefore
						summary = l.LastCompactionResult.Summary
						l.LastCompactionResult = nil // reset after use
					}
					l.EventBus.Emit(events.CompactionEnd(tokensBefore, summary, false, "auto"))
				}
			}
		}

		// Emit message_start for assistant
		if l.EventBus != nil {
			l.EventBus.Emit(events.MessageStart("assistant"))
		}

		// Wire interrupt context to LLM client for HTTP request cancellation
		if cl, ok := l.Provider.(interface{ SetActiveCtx(context.Context) }); ok {
			cl.SetActiveCtx(ctx)
		}

		eventsCh, err := l.Provider.StreamChat(l.Model, workMessages, toolDefs, l.SystemPrompt)
		if err != nil {
			if l.EventBus != nil {
				l.EventBus.Emit(events.ErrorEvent(err.Error()))
			}
			lastError = err
			// Check if retryable
			if l.Config.MaxRetries > 0 && retryAttempt < l.Config.MaxRetries {
				retryAttempt++
				delayMs := 2000 * (1 << (retryAttempt - 1)) // exponential backoff: 2s, 4s, 8s...
				if l.EventBus != nil {
					l.EventBus.Emit(events.AutoRetryStart(retryAttempt, l.Config.MaxRetries, delayMs))
				}
				// TS pi-mono: actual countdown delay with cancellation support
				select {
				case <-time.After(time.Duration(delayMs) * time.Millisecond):
					// delay elapsed, proceed with retry
				case <-ctx.Done():
					// user aborted during retry countdown (TS pi-mono: abortRetry)
					if l.EventBus != nil {
						l.EventBus.Emit(events.AutoRetryEnd(false, retryAttempt, "Retry cancelled"))
					}
					return "", messages, nil
				}
				continue // retry on next turn
			}
			// Emit retry end failure if retries were attempted but exhausted (TS pi-mono)
			if retryAttempt > 0 && l.EventBus != nil {
				l.EventBus.Emit(events.AutoRetryEnd(false, retryAttempt, err.Error()))
			}
			// Emit agent_end
			if l.EventBus != nil {
				l.EventBus.Emit(events.AgentEnd("error", nil))
			}
			return "", messages, fmt.Errorf("turn %d: %w", turn, err)
		}
		// Successful stream call after retry -> emit retry end
		if retryAttempt > 0 && l.EventBus != nil {
			l.EventBus.Emit(events.AutoRetryEnd())
			retryAttempt = 0
		}

		var fullText string
		var reasoningText string
		var toolCalls []types.ToolCall
		var totalUsage types.Usage
		var outputStarted bool
		lastStopReason = "" // reset per turn

		// Bridge LLM stream events to EventBus
		for event := range eventsCh {
			switch event.Type {
		case "thinking_start":
			if l.Verbose {
				fmt.Fprintf(os.Stderr, "\n%s[thinking]%s ", cMagenta, cReset)
			}
			if l.EventBus != nil {
				l.EventBus.Emit(events.ThinkingStart())
			}
		case "thinking_delta":
			reasoningText += event.Text
			if l.Verbose {
				fmt.Fprint(os.Stderr, event.Text)
			}
			if l.EventBus != nil {
				l.EventBus.Emit(events.ThinkingDelta(event.Text))
			}
		case "thinking_end":
			if l.Verbose {
				fmt.Fprintln(os.Stderr)
			}
			if l.EventBus != nil {
				l.EventBus.Emit(events.ThinkingEnd())
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
			case "toolcall_start":
				if l.EventBus != nil {
					l.EventBus.Emit(events.ToolCallStart(event.ToolName, event.ToolID))
				}
			case "toolcall_delta":
				if l.EventBus != nil {
					l.EventBus.Emit(events.ToolCallDelta(event.Text))
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
							event.Usage.CacheReadTokens,
							event.Usage.CacheWriteTokens,
						))
					}
				}
				if event.StopReason != "" {
					lastStopReason = event.StopReason
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
			// If steering messages are pending, the user interrupted during
			// streaming. Drain steering and restart with a fresh context.
			if len(l.SteeringQueue) > 0 {
				messages = l.drainSteering(messages)
				lastError = nil
				ctx = l.setupTurnContext(baseCtx)
				continue
			}
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
				l.EventBus.Emit(events.AgentEnd("stop_condition", &totalUsage, lastStopReason))
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
				l.EventBus.Emit(events.AgentEnd("complete", &totalUsage, lastStopReason))
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
		l.EventBus.Emit(events.AgentEnd("max_turns", nil, lastStopReason))
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

				if l.EventBus != nil {
					l.EventBus.Emit(events.ToolStart(call.ID, call.Function.Name))
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

			if l.EventBus != nil {
				l.EventBus.Emit(events.ToolStart(tc.ID, tc.Function.Name))
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

// QueuedCounts returns separate steering and follow-up queue lengths.
func (l *Loop) QueuedCounts() (steering, followUp int) {
	return len(l.SteeringQueue), len(l.FollowUpQueue)
}

// PendingMessageCount returns the total pending messages in both queues.
func (l *Loop) PendingMessageCount() int {
	return len(l.SteeringQueue) + len(l.FollowUpQueue)
}

// DrainQueues drains all pending messages from both queues and returns them.
func (l *Loop) DrainQueues() []string {
	var msgs []string
	for {
		select {
		case msg := <-l.SteeringQueue:
			msgs = append(msgs, msg)
		default:
			goto doneSteering
		}
	}
doneSteering:
	for {
		select {
		case msg := <-l.FollowUpQueue:
			msgs = append(msgs, msg)
		default:
			return msgs
		}
	}
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
	select {
	case l.SteeringQueue <- message:
	default:
		// Queue full, drop
	}

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
	select {
	case l.SteeringQueue <- message:
	default:
	}
}

// FollowUp injects a follow-up message for after the agent finishes the current
// turn. TS pi-mono equivalent: Alt+Enter queues message for later delivery.
func (l *Loop) FollowUp(message string) {
	select {
	case l.FollowUpQueue <- message:
	default:
	}
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
