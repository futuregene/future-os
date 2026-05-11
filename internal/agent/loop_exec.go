package agent

import (
	"context"
	"fmt"
	"os"
	"sync"
	"time"

	"github.com/huichen/xihu/internal/events"
	"github.com/huichen/xihu/pkg/types"
)

// executeTools runs tool calls either sequentially or in parallel.
func (l *Loop) executeTools(ctx context.Context, turn int, toolCalls []types.ToolCall, messages *[]types.AgentMessage) {
	useParallel := l.ParallelTools
	if l.Config.ToolsExecutionMode != "" {
		useParallel = l.Config.ToolsExecutionMode == "parallel"
	}

	if useParallel && len(toolCalls) > 1 {
		l.executeToolsParallel(ctx, turn, toolCalls, messages)
	} else {
		l.executeToolsSequential(ctx, turn, toolCalls, messages)
	}
}

// executeToolsParallel runs all tool calls concurrently via goroutines.
func (l *Loop) executeToolsParallel(ctx context.Context, turn int, toolCalls []types.ToolCall, messages *[]types.AgentMessage) {
	toolResults := make([]struct {
		result   string
		err      error
		toolName string
		duration time.Duration
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

			l.executeOneTool(ctx, call, idx, &toolResults[idx].result, &toolResults[idx].err, &toolResults[idx].toolName, &toolResults[idx].duration)
		}(i, tc)
	}
	wg.Wait()

	for i, tc := range toolCalls {
		if l.Verbose {
			toolLog(tc.Function.Name, tc.Function.Arguments, toolResults[i].err, toolResults[i].duration)
		}
		emitToolEnd(l.EventBus, toolResults[i].toolName, toolResults[i].result, toolResults[i].err, toolResults[i].duration)
		toolMsg := newToolAgentResult(tc.ID, toolResults[i].result, toolResults[i].err)
		*messages = append(*messages, toolMsg)
	}
}

// executeToolsSequential runs tool calls one at a time.
func (l *Loop) executeToolsSequential(ctx context.Context, turn int, toolCalls []types.ToolCall, messages *[]types.AgentMessage) {
	for _, tc := range toolCalls {
		if err := ctx.Err(); err != nil {
			if l.Verbose {
				fmt.Fprintf(os.Stderr, "\n[tool] %s: context cancelled\n", tc.Function.Name)
			}
			break
		}

		var result string
		var execErr error
		var toolName string
		var duration time.Duration

		l.executeOneTool(ctx, tc, 0, &result, &execErr, &toolName, &duration)

		if l.Verbose {
			toolLog(tc.Function.Name, tc.Function.Arguments, execErr, duration)
		}
		emitToolEnd(l.EventBus, toolName, result, execErr, duration)
		toolMsg := newToolAgentResult(tc.ID, result, execErr)
		*messages = append(*messages, toolMsg)
	}
}

// executeOneTool runs a single tool call with the full 3-stage pipeline:
//   1. BeforeToolCall — intercept/skip execution
//   2. PrepareToolCall — transform arguments
//   3. Execute the tool
//   4. FinalizeToolCall — transform result/error
//   5. AfterToolCall — modify or mask results
func (l *Loop) executeOneTool(ctx context.Context, tc types.ToolCall, _ int, outResult *string, outErr *error, outName *string, outDuration *time.Duration) {
	*outName = tc.Function.Name

	// Stage 1: BeforeToolCall hook — allows extensions to intercept or skip execution.
	// TS pi-mono: beforeToolCall in the 3-stage pipeline.
	if l.Config.BeforeToolCall != nil {
		if override := l.Config.BeforeToolCall(tc.Function.Name, tc.ID, tc.Function.Arguments); override != nil && override.Result != "" {
			if override.IsError {
				*outErr = fmt.Errorf("%s", override.Result)
			} else {
				*outResult = override.Result
			}
			return
		}
	}

	// Stage 2: PrepareToolCall hook — transform arguments before execution.
	// TS pi-mono: prepareToolCall in the 3-stage pipeline.
	effectiveArgs := tc.Function.Arguments
	if l.Config.PrepareToolCall != nil {
		if modified := l.Config.PrepareToolCall(tc.Function.Name, tc.Function.Arguments); modified != nil {
			effectiveArgs = modified
		}
	}

	if l.EventBus != nil {
		l.EventBus.Emit(events.ToolStart(tc.ID, tc.Function.Name))
	}
	start := time.Now()

	// Stage 3: Execute the tool with (possibly modified) arguments.
	effectiveTC := tc
	effectiveTC.Function.Arguments = effectiveArgs
	result, err := l.executeTool(effectiveTC)
	*outDuration = time.Since(start)

	// Stage 4: FinalizeToolCall hook — transform result/error after execution.
	// TS pi-mono: finalizeToolCall in the 3-stage pipeline.
	if l.Config.FinalizeToolCall != nil {
		result, err = l.Config.FinalizeToolCall(tc.Function.Name, result, err)
	}

	// Stage 5: AfterToolCall hook — allows extensions to modify or mask results.
	// TS pi-mono: afterToolCall in the 3-stage pipeline.
	if l.Config.AfterToolCall != nil {
		if override := l.Config.AfterToolCall(tc.Function.Name, tc.ID, tc.Function.Arguments, result, err); override != nil {
			result = override.Result
			if override.IsError && err == nil {
				err = fmt.Errorf("%s", override.Result)
			} else if !override.IsError {
				err = nil
			}
		}
	}

	*outResult = result
	*outErr = err
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
