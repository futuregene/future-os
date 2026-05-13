package agent

import (
	"context"
	"fmt"
	"os"
	"sync"
	"time"

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

	// SteeringQueue is a buffered queue of steering messages (injected before each turn).
	SteeringQueue *PendingMessageQueue

	// FollowUpQueue is a buffered queue of follow-up messages (injected after agent finishes).
	FollowUpQueue *PendingMessageQueue

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
		SteeringQueue: NewPendingMessageQueue(64, "all"),
		FollowUpQueue: NewPendingMessageQueue(64, "all"),
		Config: types.AgentConfig{
			MaxTurns: DefaultMaxTurns,
		},
	}
}

// RunStreamingWithMessages runs the agent loop with pre-existing messages,
// returning final text and all messages. Uses AgentMessage internally, converting
// to []types.Message via ConvertToLLM() before each LLM call.
// onText is called for each text chunk. onEvent is called for all stream events
// (including tool calls), allowing the caller to forward them to SSE subscribers.
func (l *Loop) RunStreamingWithMessages(
	ctx context.Context,
	messages []types.AgentMessage,
	onText func(string),
	onEvent func(event types.StreamEvent),
) (string, []types.AgentMessage, error) {
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
		steeringBefore := l.SteeringQueue.Len()
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

		// Apply context transform if configured (e.g., compaction).
		// Convert to LLM Message format for TransformContext, then convert back.
		workMessages := messages
		if l.Config.TransformContext != nil {
			beforeLen := len(workMessages)
			llmWork := types.ConvertToLLM(messages)
			llmWork = l.Config.TransformContext(llmWork, "")
			workMessages = types.ConvertFromLLM(llmWork)
			if len(workMessages) < beforeLen {
				// Compaction actually happened — emit events
				if l.EventBus != nil {
					l.EventBus.Emit(events.CompactionStart("auto"))
				}
				tokensBefore := 0
				summary := ""
				if l.LastCompactionResult != nil {
					tokensBefore = l.LastCompactionResult.TokensBefore
					summary = l.LastCompactionResult.Summary
					l.LastCompactionResult = nil // reset after use
				}
				if l.EventBus != nil {
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

		// Convert agent messages to LLM wire format before the API call.
		llmMessages := types.ConvertToLLM(workMessages)
		eventsCh, err := l.Provider.StreamChat(l.Model, llmMessages, toolDefs, l.SystemPrompt)
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
				if onEvent != nil {
					onEvent(event)
				}
			case "toolcall_delta":
				if l.EventBus != nil {
					l.EventBus.Emit(events.ToolCallDelta(event.Text))
				}
				if onEvent != nil {
					onEvent(event)
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
					if onEvent != nil {
						onEvent(event)
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
			if l.SteeringQueue.Len() > 0 {
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

		assistantMsg := types.AgentMessage{
			Role:     "assistant",
			Content:  []types.ContentBlock{types.TextBlock{Text: fullText}},
			Thinking: reasoningText,
		}
		// Convert LLM tool calls to agent tool calls
		for _, tc := range toolCalls {
			assistantMsg.ToolCalls = append(assistantMsg.ToolCalls, types.AgentToolCall{
				ID:   tc.ID,
				Name: tc.Function.Name,
				Args: tc.Function.Arguments,
			})
		}
		messages = append(messages, assistantMsg)

		// Check stop condition after this turn.
		// Convert to LLM Message format for the callback (backward compat).
		if l.Config.StopCondition != nil && l.Config.StopCondition(types.ConvertToLLM(messages), fullText) {
			if l.EventBus != nil {
				l.EventBus.Emit(events.AgentEnd("stop_condition", &totalUsage, lastStopReason))
			}
			return fullText, messages, nil
		}

		// If no tool calls, check if follow-up queue has pending messages before returning
		if len(toolCalls) == 0 {
			if l.FollowUpQueue.Len() > 0 {
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
	messages := []types.AgentMessage{
		newSystemAgentMessage(l.SystemPrompt),
		newUserAgentMessage(userPrompt),
	}
	result, _, err := l.RunStreamingWithMessages(ctx, messages, onText, nil)
	return result, err
}
