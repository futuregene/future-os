// Package tui provides the interactive terminal UI for xihu using Bubble Tea.
package tui

import (

	"context"
	"encoding/json"
	"fmt"
	"strings"
	"sync/atomic"
	"time"


	"github.com/huichen/xihu/internal/compaction"
	"github.com/huichen/xihu/internal/modelregistry"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/tui/components"
	"github.com/huichen/xihu/pkg/types"

	bashexec "github.com/huichen/xihu/internal/exec"

)

// ─── Message Types ─────────────────────────────────────────────────────────

// StreamTextMsg is a chunk of streamed text from the LLM.

func (m *AppModel) runAgent(text string, myID int32) {
	m.streaming = true
	m.pendingSteeringMsgs = nil
		m.pendingFollowUpMsgs = nil // clear pending messages when agent starts running
	m.startProgress()

	// Show connecting status (TS pi-mono: createWorkingLoader)
	modelName, provider := parseModelString(m.agent.Loop().Model)
	_ = provider
	m.footer.SetWorkingMessage("Connecting to " + modelName + "...")

	// Look up model cost and context window for footer stats (TS pi-mono: cost tracking)
	var modelCost types.Model
	contextWin := 0
	for _, mi := range modelregistry.BuiltinModels() {
		if mi.ID == modelName {
			modelCost = mi
			contextWin = mi.ContextWindow
			break
		}
	}

	// Accumulated stats (declared here so they can be used in initial StatusMsg before goroutine)
	var tokensIn, tokensOut, cacheR, cacheW int
	var contextTokens int // estimated current context size
	// Carry forward previous stats
	tokensIn = m.lastStatus.TokensIn
	tokensOut = m.lastStatus.TokensOut
	cacheR = m.lastStatus.TokensCacheR
	cacheW = m.lastStatus.TokensCacheW
	contextTokens = m.lastStatus.ContextTokens

	var messages []types.Message
	if m.session != nil && len(m.session.Entries) > 0 {
		leafID := session.EffectiveLeafID(m.session)
		messages = session.BuildContextFromLeaf(m.session.Entries, leafID)
	}
	userMsg := types.Message{
		Role:    "user",
		Content: jsonMarshalContent(text),
	}
	messages = append(messages, userMsg)

	// Save user message as session entry (TS pi-mono: appendMessage in _processAgentEvent)
	initialMsgCount := len(messages)
	if m.session != nil && m.sessMgr != nil {
		parentID := session.EffectiveLeafID(m.session)
		userEntry := session.MessageToEntry(userMsg, parentID)
		if err := m.sessMgr.AddEntry(m.session, userEntry); err == nil {
			m.footer.SetEntryCount(len(m.session.Entries))
		}
	}

	// Estimate initial context size (before first LLM call) for accurate context % at startup
	contextTokens = compaction.EstimateContextTokens(messages)

	// Show streaming indicator and initial context % via footer
	if m.program != nil {
		m.program.Send(StatusMsg{
			TokensIn:      m.lastStatus.TokensIn,
			TokensOut:     m.lastStatus.TokensOut,
			TokensCacheR:  m.lastStatus.TokensCacheR,
			TokensCacheW:  m.lastStatus.TokensCacheW,
			ContextTokens: contextTokens,
			ContextWin:    contextWin,
			Streaming:     true,
		})
	}

	// Subscribe to EventBus
	subID := fmt.Sprintf("tui-%d", time.Now().UnixNano())
	eventsCh := m.eventBus.Subscribe(subID)
	defer m.eventBus.Unsubscribe(subID)

	// Goroutine to forward EventBus events to Bubble Tea
	go func() {
		for evt := range eventsCh {
			// Drop events from a stale stream (interrupted / superseded)
			if atomic.LoadInt32(&m.streamID) != myID {
				continue
			}
			switch evt.Type {
			case "thinking_delta":
				if t, ok := evt.Data["text"].(string); ok && m.program != nil {
					m.program.Send(ThinkingMsg(t))
				}
			case "toolcall_start":
				id, _ := evt.Data["tool_id"].(string)
				name, _ := evt.Data["tool_name"].(string)
				if m.program != nil {
					m.program.Send(ToolCallStartMsg{ID: id, Name: name})
				}
			case "toolcall_delta":
				if t, ok := evt.Data["text"].(string); ok && m.program != nil {
					m.program.Send(ToolCallDeltaMsg{Text: t})
				}
			case "toolcall_end":
				id, _ := evt.Data["tool_id"].(string)
				name, _ := evt.Data["tool_name"].(string)
				args, _ := evt.Data["args"].(string)
				if m.program != nil {
					m.program.Send(ToolCallMsg{ID: id, Name: name, Arguments: args})
				}
			case "tool_start":
				if id, ok := evt.Data["tool_call_id"].(string); ok && m.program != nil {
					name, _ := evt.Data["tool_name"].(string)
					m.program.Send(ToolRunningMsg{ID: id, Name: name})
				}
			case "tool_end":
				name, _ := evt.Data["tool_name"].(string)
				result, _ := evt.Data["result"].(string)
				errStr, _ := evt.Data["error"].(string)
				durMs, _ := evt.Data["duration"].(int64)
				if m.program != nil {
					m.program.Send(ToolResultMsg{ID: name, Output: result, Error: errStr, DurationMs: durMs})
				}
			case "usage":
				if in, ok := evt.Data["input_tokens"].(int); ok {
					tokensIn += in
				}
				if out, ok := evt.Data["output_tokens"].(int); ok {
					tokensOut += out
				}
				if cr, ok := evt.Data["cache_read_tokens"].(int); ok {
					cacheR += cr
				}
				if cw, ok := evt.Data["cache_write_tokens"].(int); ok {
					cacheW += cw
				}
				// Update context size estimate from usage (tokens are accumulated per-call)
				contextTokens = tokensIn + tokensOut
				// Send intermediate status with cost and context% (TS pi-mono: live stats)
				if m.program != nil {
					totalCost := calcModelCost(tokensIn, tokensOut, cacheR, cacheW, modelCost)
					m.program.Send(StatusMsg{
						TokensIn:      tokensIn,
						TokensOut:     tokensOut,
						TokensCacheR:  cacheR,
						TokensCacheW:  cacheW,
						TotalCost:     totalCost,
						ContextTokens: contextTokens,
						ContextWin:    contextWin,
						Streaming:     true,
					})
				}
			case "auto_retry_start":
				attempt, _ := evt.Data["attempt"].(int)
				maxAttempts, _ := evt.Data["max_attempts"].(int)
				delayMs, _ := evt.Data["delay_ms"].(int)
				delaySec := (delayMs + 999) / 1000
				// Store countdown state for live ticking (TS pi-mono: CountdownTimer)
				m.retryTicking = true
				m.retryDelaySec = delaySec
				m.retryAttempt = attempt
				m.retryMaxAttempts = maxAttempts
				m.chat.AppendSystem(fmt.Sprintf("Retrying (%d/%d) in %ds... (Esc to cancel)", attempt, maxAttempts, delaySec))
				if m.program != nil && delaySec > 0 {
					go func() {
						time.Sleep(1 * time.Second)
						if m.program != nil && m.retryTicking {
							m.program.Send(RetryTickMsg(time.Now()))
						}
					}()
				}
			case "auto_retry_end":
				m.retryTicking = false
				if success, ok := evt.Data["success"].(bool); ok && !success {
					attempt, _ := evt.Data["attempt"].(int)
					finalError, _ := evt.Data["final_error"].(string)
					if finalError == "" {
						finalError = "Unknown error"
					}
					m.chat.AppendError(fmt.Sprintf("Retry failed after %d attempts: %s", attempt, finalError))
				}
			case "compaction_start":
				m.compacting = true
				m.compactionQueue = nil // reset queue on new compaction
				reason, _ := evt.Data["reason"].(string)
				if reason == "manual" {
					m.chat.AppendSystem("Compacting context... (Esc to cancel)")
				} else {
					m.chat.AppendSystem("Context overflow detected, Auto-compacting... (Esc to cancel)")
				}
				m.footer.SetWorkingMessage("Compacting...")
			case "compaction_end":
				m.compacting = false
				m.footer.SetWorkingMessage("Working...")
				aborted, _ := evt.Data["aborted"].(bool)
				reason, _ := evt.Data["reason"].(string)
				if aborted {
					if reason == "manual" {
						m.chat.AppendError("Compaction cancelled")
					} else {
						m.chat.AppendSystem("Auto-compaction cancelled")
					}
				} else {
					tokensBefore, _ := evt.Data["tokens_before"].(int)
					summary, _ := evt.Data["summary"].(string)
					if tokensBefore > 0 {
						// TS pi-mono: clear chat and rebuild from session after compaction.
						// rebuildChatFromSession walks the tree from the current leaf,
						// skipping entries that were compacted away, then we append a
						// fresh expandable compaction summary card at the end.
						m.rebuildChatFromSession()
						m.chat.AppendCompactionSummary(summary, tokensBefore)
					} else {
						m.chat.AppendSystem("Context compacted")
					}
				}
				// Flush queued messages (TS pi-mono: flushCompactionQueue)
				if len(m.compactionQueue) > 0 {
					queued := m.compactionQueue
					m.compactionQueue = nil
					for _, qm := range queued {
						m.program.Send(components.SubmitMsg(qm))
					}
									}
			case "agent_end":
				// agent_end may carry a nested "usage" map
				if usageRaw, ok := evt.Data["usage"]; ok {
					if usageMap, ok := usageRaw.(map[string]int); ok {
						if in, ok := usageMap["input_tokens"]; ok {
							tokensIn += in
						}
						if out, ok := usageMap["output_tokens"]; ok {
							tokensOut += out
						}
						if cr, ok := usageMap["cache_read_tokens"]; ok {
							cacheR += cr
						}
						if cw, ok := usageMap["cache_write_tokens"]; ok {
							cacheW += cw
						}
					}
				}
				// Propagate stop_reason for display (TS pi-mono: stopReason on last assistant message)
				if sr, ok := evt.Data["stop_reason"].(string); ok && sr != "" && sr != "stop" && sr != "toolUse" {
					reason := sr
					if sr == "length" {
						reason = "length"
					}
					if m.program != nil {
						m.program.Send(StopReasonMsg{Reason: reason})
					}
				}
				// Send final status
				if m.program != nil {
					totalCost := calcModelCost(tokensIn, tokensOut, cacheR, cacheW, modelCost)
					m.program.Send(StatusMsg{
						TokensIn:      tokensIn,
						TokensOut:     tokensOut,
						TokensCacheR:  cacheR,
						TokensCacheW:  cacheW,
						TotalCost:     totalCost,
						ContextTokens: contextTokens,
						ContextWin:    contextWin,
						Streaming:     false,
					})
				}
			}
		}
	}()

	ctx := context.Background()
	_, finalMessages, err := m.agent.Loop().RunStreamingWithMessages(ctx, types.ConvertFromLLM(messages), func(chunk string) {
		if m.program != nil && atomic.LoadInt32(&m.streamID) == myID {
			m.program.Send(StreamTextMsg(chunk))
		}
	})

	// Save assistant and tool messages as session entries (TS pi-mono: appendMessage for each event.message)
	if m.session != nil && m.sessMgr != nil && len(finalMessages) > initialMsgCount {
		newMessages := finalMessages[initialMsgCount:] // assistant messages + tool results
		for _, msg := range newMessages {
			parentID := session.EffectiveLeafID(m.session)
			entry := session.MessageToEntry(types.ConvertToLLM([]types.AgentMessage{msg})[0], parentID)
			if err := m.sessMgr.AddEntry(m.session, entry); err == nil {
				// LeafID is auto-updated by AddEntry
			}
		}
		m.footer.SetEntryCount(len(m.session.Entries))
	}

	if err != nil && m.program != nil {
		m.program.Send(AgentErrorMsg{Error: err})
	} else if m.program != nil {
		m.program.Send(AgentDoneMsg{})
	}
	m.streaming = false
}

func jsonMarshalContent(text string) json.RawMessage {
	b, _ := json.Marshal([]types.TextContent{{Type: "text", Text: text}})
	return json.RawMessage(b)
}

// runBashDirect executes a bash command directly (! prefix), bypassing the LLM.
// excludeFromCtx is true for !! prefix.
func (m *AppModel) runBashDirect(command string, excludeFromCtx bool) {
	cwd := ""
	if m.session != nil {
		cwd = m.session.CWD
	}

	// Show the bash entry in chat viewport immediately (dim if excluded from context)
	m.chat.AddBashExecution(command, excludeFromCtx)

	m.bashCancelCh = make(chan struct{})
	cancelCh := m.bashCancelCh
	defer func() { m.bashCancelCh = nil }()

	result, err := bashexec.ExecuteBash(bashexec.BashExecutorOptions{
		Command:     command,
		CWD:         cwd,
		AbortSignal: cancelCh,
	})

	// Record bash result in session (TS pi-mono: bash results stored in session)
	if m.session != nil && m.sessMgr != nil {
		output := ""
		exitCode := -1
		cancelled := false
		if err != nil {
			output = fmt.Sprintf("Error: %v", err)
		} else {
			output = result.Output
			exitCode = result.ExitCode
			cancelled = result.Cancelled
		}
		bashData, _ := json.Marshal(map[string]interface{}{
			"command":   command,
			"output":    output,
			"exit_code": exitCode,
			"cancelled": cancelled,
			"excluded":  excludeFromCtx,
		})
		m.sessMgr.AddEntry(m.session, session.SessionEntry{
			ID:         session.GenerateID(),
			ParentID:   session.EffectiveLeafID(m.session),
			Type:       "custom",
			CustomType: "bash",
			Content:    bashData,
			Timestamp:  time.Now(),
		})
	}

	if m.program != nil {
		if err != nil {
			m.program.Send(BashExecResultMsg{
				Command:  command,
				Output:   fmt.Sprintf("Error: %v", err),
				ExitCode: -1,
			})
			return
		}
		m.program.Send(BashExecResultMsg{
			Command:   command,
			Output:    result.Output,
			ExitCode:  result.ExitCode,
			Cancelled: result.Cancelled,
		})
	}
}

// handleSlashCmd processes a slash command and returns the result string.
// Local commands (model, thinking, quit, hotkeys) are handled here;
// everything else is forwarded to the commands.Handle() subsystem.
func extractBashCommand(argsJSON string) string {
	needle := `"command": "`
	idx := strings.Index(argsJSON, needle)
	if idx < 0 {
		return argsJSON
	}
	start := idx + len(needle)
	end := strings.IndexByte(argsJSON[start:], '"')
	if end < 0 {
		return argsJSON
	}
	return argsJSON[start : start+end]
}

// calcModelCost computes the total cost from token counts using model pricing.
// Model costs are per 1M tokens.
func calcModelCost(tokensIn, tokensOut, cacheR, cacheW int, mc types.Model) float64 {
	const scale = 1_000_000.0
	cost := float64(tokensIn)*mc.Cost.Input/scale +
		float64(tokensOut)*mc.Cost.Output/scale +
		float64(cacheR)*mc.Cost.CacheRead/scale +
		float64(cacheW)*mc.Cost.CacheWrite/scale
	return cost
}

// ─── Model Parsing ─────────────────────────────────────────────────────────

// formatRelativeDate returns a human-readable relative date like pi-mono.
// Examples: "now", "5m", "2h", "3d", "2w", "1mo", "1y"
