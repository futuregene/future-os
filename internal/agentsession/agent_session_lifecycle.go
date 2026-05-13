package agentsession

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"time"

	"github.com/huichen/xihu/internal/compaction"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/pkg/types"
)

// =============================================================================
// Prompting
// =============================================================================

// PromptOptions holds options for Prompt().
type PromptOptions struct {
	// StreamingBehavior: "steer" or "followUp" (required if agent is streaming).
	StreamingBehavior string

	// Images are optional image attachments (base64 encoded).
	Images []types.ImageContent
}

// Prompt sends a user message to the agent.
// - If the agent is streaming, queues via steer or followUp based on StreamingBehavior.
// - Handles slash commands and system commands.
// Returns an error if streaming and no StreamingBehavior specified.
func (s *AgentSession) Prompt(text string, opts *PromptOptions) error {
	// Check for slash commands first
	if len(text) > 0 && text[0] == '/' {
		// Let commands.Slash.Handle process it
		// For now, skip and treat as regular prompt
	}

	s.mu.RLock()
	streaming := s.isStreaming
	s.mu.RUnlock()

	if streaming {
		if opts == nil || opts.StreamingBehavior == "" {
			return fmt.Errorf("agent is already processing; specify streamingBehavior ('steer' or 'followUp') to queue")
		}
		if opts.StreamingBehavior == "followUp" {
			return s.queueFollowUp(text)
		}
		return s.queueSteer(text)
	}

	return s.runPrompt(text, opts)
}

// runPrompt executes the actual prompt (non-streaming case).
func (s *AgentSession) runPrompt(text string, opts *PromptOptions) error {
	// Set streaming flag
	s.mu.Lock()
	s.isStreaming = true
	s.mu.Unlock()

	defer func() {
		s.mu.Lock()
		s.isStreaming = false
		s.mu.Unlock()
	}()

	// Build message list from session context
	var messages []types.Message
	entries := s.engine.Session.Entries
	if len(entries) > 0 {
		messages = session.BuildContext(entries)
	}

	// Build user message with optional images
	var contentBlocks []json.RawMessage
	textBlock, _ := json.Marshal(types.TextContent{Type: "text", Text: text})
	contentBlocks = append(contentBlocks, textBlock)

	if opts != nil {
		for _, img := range opts.Images {
			imgBlock, _ := json.Marshal(img)
			contentBlocks = append(contentBlocks, imgBlock)
		}
	}

	content, _ := json.Marshal(contentBlocks)
	userMsg := types.Message{
		Role:    "user",
		Content: content,
	}
	messages = append(messages, userMsg)

	// Create cancellable context
	ctx, cancel := context.WithCancel(context.Background())
	s.agentCtx = ctx
	s.agentCancel = cancel

	// Emit agent_start
	s.emit(AgentSessionEvent{Type: "agent_start"})

	// Run the agent loop — convert session messages to internal AgentMessage format.
	agentMessages := types.ConvertFromLLM(messages)
	finalText, finalAgentMessages, err := s.engine.Loop.RunStreamingWithMessages(ctx, agentMessages, func(text string) {
		s.emit(AgentSessionEvent{Type: "text_chunk", Text: text})
	})

	// Emit agent_end with the full assistant response text
	s.emit(AgentSessionEvent{Type: "agent_end", Text: finalText})

	if err != nil {
		s.emit(AgentSessionEvent{Type: "error", ErrorMessage: err.Error()})
		return err
	}

	// Save to session — convert AgentMessages back to LLM Message format for persistence.
	newEntries := session.MessagesToEntries(types.ConvertToLLM(finalAgentMessages), "")
	s.engine.Session.Entries = append(s.engine.Session.Entries, newEntries...)
	s.engine.Session.UpdatedAt = time.Now()

	// Persist
	if err := s.engine.SessionManager.Save(s.engine.Session); err != nil {
		log.Printf("agentsession: save session: %v", err)
	}

	_ = finalText
	return nil
}

// =============================================================================
// Steering / FollowUp / Abort
// =============================================================================

// Steer queues a steering message while the agent is running.
// Delivered after the current assistant turn finishes.
func (s *AgentSession) Steer(text string) error {
	return s.queueSteer(text)
}

// FollowUp queues a follow-up message to be processed after the agent finishes.
func (s *AgentSession) FollowUp(text string) error {
	return s.queueFollowUp(text)
}

// queueSteer adds a message to the steering queue.
func (s *AgentSession) queueSteer(text string) error {
	s.mu.Lock()
	s.steeringMessages = append(s.steeringMessages, text)
	s.mu.Unlock()

	s.emitQueueUpdate()

	// Also push to the loop's steering channel
	s.engine.Loop.Steer(text)
	return nil
}

// queueFollowUp adds a message to the follow-up queue.
func (s *AgentSession) queueFollowUp(text string) error {
	s.mu.Lock()
	s.followUpMessages = append(s.followUpMessages, text)
	s.mu.Unlock()

	s.emitQueueUpdate()

	// Also push to the loop's followUp channel
	s.engine.Loop.FollowUp(text)
	return nil
}

// Abort aborts the current agent operation.
func (s *AgentSession) Abort() {
	s.mu.Lock()
	defer s.mu.Unlock()

	if s.agentCancel != nil {
		s.agentCancel()
		s.agentCancel = nil
	}
}

// ClearQueue clears all queued messages and returns them.
func (s *AgentSession) ClearQueue() (steering, followUp []string) {
	s.mu.Lock()
	steering = s.steeringMessages
	followUp = s.followUpMessages
	s.steeringMessages = nil
	s.followUpMessages = nil
	s.mu.Unlock()

	s.emitQueueUpdate()
	return
}

// =============================================================================
// Model Management
// =============================================================================

// SetModel switches to the specified model.
func (s *AgentSession) SetModel(model string) error {
	s.engine.Model = model
	s.engine.Session.Model = model
	s.engine.Loop.Model = model

	// Record in session
	if s.engine.Session != nil && s.engine.SessionManager != nil {
		entry := session.SessionEntry{
			ID:        session.GenerateID(),
			ParentID:  s.getLeafParentID(),
			Type:      session.EntryTypeModelChange,
			Model:     model,
			Timestamp: time.Now(),
		}
		s.engine.Session.Entries = append(s.engine.Session.Entries, entry)
		s.engine.SessionManager.Save(s.engine.Session)
	}

	return nil
}

// CycleModel cycles to the next available model.
// Returns the new model, or empty string if only one model.
func (s *AgentSession) CycleModel(direction string) string {
	if len(s.scopedModels) <= 1 {
		return ""
	}

	current := s.engine.Model
	currentIdx := -1
	for i, m := range s.scopedModels {
		if m == current {
			currentIdx = i
			break
		}
	}

	inc := 1
	if direction == "backward" {
		inc = -1
	}

	nextIdx := 0
	if currentIdx >= 0 {
		nextIdx = (currentIdx + inc + len(s.scopedModels)) % len(s.scopedModels)
	}

	nextModel := s.scopedModels[nextIdx]
	s.SetModel(nextModel)
	return nextModel
}

// =============================================================================
// Thinking Level Management
// =============================================================================

// SetThinkingLevel sets the thinking/reasoning level.
func (s *AgentSession) SetThinkingLevel(level string) {
	// Convert level to budget
	budget := thinkingLevelToBudget(level)
	s.engine.Loop.Config.ThinkingBudget = budget

	s.emit(AgentSessionEvent{
		Type:  "thinking_level_changed",
		Level: level,
	})
}

// CycleThinkingLevel cycles to the next thinking level.
func (s *AgentSession) CycleThinkingLevel() string {
	current := ""
	switch {
	case s.engine.Loop.Config.ThinkingBudget == 0:
		current = "off"
	case s.engine.Loop.Config.ThinkingBudget <= 4000:
		current = "low"
	case s.engine.Loop.Config.ThinkingBudget <= 8000:
		current = "medium"
	case s.engine.Loop.Config.ThinkingBudget <= 16000:
		current = "high"
	default:
		current = "xhigh"
	}

	// Find current and cycle
	for i, l := range thinkingLevels {
		if l == current {
			next := thinkingLevels[(i+1)%len(thinkingLevels)]
			s.SetThinkingLevel(next)
			return next
		}
	}
	return "off"
}

func thinkingLevelToBudget(level string) int {
	switch level {
	case "off":
		return 0
	case "minimal":
		return 2000
	case "low":
		return 4000
	case "medium":
		return 8000
	case "high":
		return 16000
	case "xhigh":
		return 24000
	default:
		return 0
	}
}

// =============================================================================
// Queue Mode Management
// =============================================================================

// SetSteeringMode sets how steering messages are processed.
func (s *AgentSession) SetSteeringMode(mode string) {
	s.steeringMode = mode
	s.engine.Loop.SteeringQueue.Mode = mode
}

// SetFollowUpMode sets how follow-up messages are processed.
func (s *AgentSession) SetFollowUpMode(mode string) {
	s.followUpMode = mode
	s.engine.Loop.FollowUpQueue.Mode = mode
}

// =============================================================================
// Compaction
// =============================================================================

// Compact manually compacts the session context.
func (s *AgentSession) Compact(customInstructions string) (*compaction.CompactionResult, error) {
	// Abort current operation first
	s.Abort()

	s.emit(AgentSessionEvent{
		Type:   "compaction_start",
		Reason: "manual",
	})

	entries := s.engine.Session.Entries
	messages := session.BuildContext(entries)

	compacted, result, err := compaction.Compact(messages, compaction.CompactOptions{
		ReserveTokens:    160000,
		KeepRecentTokens: 80000,
	})

	if err != nil {
		s.emit(AgentSessionEvent{
			Type:         "compaction_end",
			Reason:       "manual",
			Aborted:      false,
			ErrorMessage: fmt.Sprintf("compaction failed: %v", err),
		})
		return nil, err
	}

	_ = compacted

	s.emit(AgentSessionEvent{
		Type:    "compaction_end",
		Reason:  "manual",
		Result:  result,
		Aborted: false,
	})

	return result, nil
}

// SetAutoCompaction toggles automatic compaction.
func (s *AgentSession) SetAutoCompaction(enabled bool) {
	s.autoCompaction = enabled
}

// =============================================================================
// Auto-Retry
// =============================================================================

// SetAutoRetry toggles automatic retry on transient errors.
func (s *AgentSession) SetAutoRetry(enabled bool) {
	s.autoRetry = enabled
}

// AbortRetry cancels an in-progress retry.
func (s *AgentSession) AbortRetry() {
	if s.retryCancel != nil {
		s.retryCancel()
		s.retryCancel = nil
	}
}
