// Package agentsession provides AgentSession — the central abstraction for
// agent lifecycle and session management, shared between all run modes
// (CLI, web, RPC). It wraps the Engine and adds higher-level session
// control: event subscription, prompt/steer/followUp, model management,
// compaction, bash execution, and session statistics.
//
// This is the Go equivalent of pi-mono's AgentSession class.
package agentsession

import (
	"context"
	"encoding/json"
	"fmt"
	"log"
	"strings"
	"sync"
	"time"

	"github.com/huichen/xihu/internal/agent"
	"github.com/huichen/xihu/internal/compaction"
	"github.com/huichen/xihu/internal/engine"
	"github.com/huichen/xihu/internal/extensions"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/tools"
	"github.com/huichen/xihu/pkg/types"
)

// =============================================================================
// AgentSessionEvent — events emitted by AgentSession
// =============================================================================

// AgentSessionEvent is a union of all events that AgentSession can emit.
type AgentSessionEvent struct {
	Type string `json:"type"` // event type

	// agent_start / agent_end / turn_* / message_* / tool_* / stop / error / usage
	// (forwarded from agent loop, same as events.EventType)

	// queue_update
	Steering []string `json:"steering,omitempty"`
	FollowUp []string `json:"follow_up,omitempty"`

	// compaction_start / compaction_end
	Reason   string                        `json:"reason,omitempty"`   // "manual" | "threshold" | "overflow"
	Result   *compaction.CompactionResult  `json:"result,omitempty"`
	Aborted  bool                          `json:"aborted,omitempty"`
	WillRetry bool                         `json:"will_retry,omitempty"`
	ErrorMessage string                    `json:"error_message,omitempty"`

	// auto_retry_start / auto_retry_end
	Attempt      int    `json:"attempt,omitempty"`
	MaxAttempts  int    `json:"max_attempts,omitempty"`
	DelayMs      int    `json:"delay_ms,omitempty"`
	Success      bool   `json:"success,omitempty"`
	FinalError   string `json:"final_error,omitempty"`

	// thinking_level_changed
	Level string `json:"level,omitempty"`

	// session_info_changed
	Name string `json:"name,omitempty"`
}

// AgentSessionEventListener is a function that receives events.
type AgentSessionEventListener func(event AgentSessionEvent)

// =============================================================================
// Config & Config types
// =============================================================================

// AgentSessionConfig holds creation options for AgentSession.
type AgentSessionConfig struct {
	// Engine is the underlying engine (required).
	Engine *engine.Engine

	// CWD is the working directory (default: from engine).
	CWD string

	// ScopedModels are models available for cycling (from --models flag).
	ScopedModels []string

	// MaxRetries is max auto-retry attempts (0 = use default, 3).
	MaxRetries int

	// AutoCompaction enables automatic context compaction.
	AutoCompaction bool

	// AutoRetry enables automatic retry on transient errors.
	AutoRetry bool
}

// defaults fills in sensible defaults.
func (c *AgentSessionConfig) defaults() {
	if c.CWD == "" && c.Engine != nil {
		c.CWD = c.Engine.Config.CWD
	}
	if c.MaxRetries <= 0 {
		c.MaxRetries = 3
	}
}

// ModelCycleEntry is a scoped model entry for cycling.
type ModelCycleEntry struct {
	Model         string
	ThinkingLevel string
}

// Thinking levels ordered for cycling.
var thinkingLevels = []string{"off", "minimal", "low", "medium", "high", "xhigh"}

// Steering modes.
const (
	SteeringModeAll          = "all"
	SteeringModeOneAtATime   = "one-at-a-time"
	FollowUpModeAll          = "all"
	FollowUpModeOneAtATime   = "one-at-a-time"
)

// =============================================================================
// AgentSession
// =============================================================================

// AgentSession is the central abstraction for agent lifecycle and session
// management. It wraps Engine and adds event subscription, prompt queuing,
// model/thinking management, compaction, bash execution, and session stats.
type AgentSession struct {
	engine *engine.Engine

	// Event subscription
	listeners []AgentSessionEventListener
	mu        sync.RWMutex

	// Steering / FollowUp queues
	steeringMessages []string
	followUpMessages []string
	steeringMode     string
	followUpMode     string

	// Compaction & retry
	autoCompaction bool
	autoRetry      bool
	maxRetries     int
	retryAttempt   int

	// Abort controllers
	compactionCancel context.CancelFunc
	bashCancel       context.CancelFunc
	retryCancel      context.CancelFunc

	// Scoped models for cycling
	scopedModels []string

	// CWD
	cwd string

	// Track whether streaming is in progress
	isStreaming bool

	// agentCtx / agentCancel for the active agent run
	agentCtx    context.Context
	agentCancel context.CancelFunc

	// Token usage tracking
	totalInputTokens  int
	totalOutputTokens int
	totalCost         float64
}

// New creates a new AgentSession from config.
func New(cfg AgentSessionConfig) (*AgentSession, error) {
	cfg.defaults()

	eng := cfg.Engine
	if eng == nil {
		return nil, fmt.Errorf("agentsession: Engine is required")
	}

	s := &AgentSession{
		engine:          eng,
		cwd:             cfg.CWD,
		scopedModels:    cfg.ScopedModels,
		maxRetries:      cfg.MaxRetries,
		autoCompaction:  cfg.AutoCompaction,
		autoRetry:       cfg.AutoRetry,
		steeringMode:    SteeringModeAll,
		followUpMode:    FollowUpModeAll,
	}

	return s, nil
}

// =============================================================================
// Read-only State Access
// =============================================================================

// Engine returns the underlying engine.
func (s *AgentSession) Engine() *engine.Engine { return s.engine }

// Loop returns the underlying agent loop (for TUI compatibility).
func (s *AgentSession) Loop() *agent.Loop { return s.engine.Loop }

// Session returns the underlying session.
func (s *AgentSession) Session() *session.Session { return s.engine.Session }

// State returns the agent loop state.
func (s *AgentSession) State() *agent.Loop { return s.engine.Loop }

// Model returns the current model ID.
func (s *AgentSession) Model() string {
	return s.engine.Model
}

// IsStreaming returns whether the agent is currently streaming.
func (s *AgentSession) IsStreaming() bool {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return s.isStreaming
}

// SessionID returns the current session ID.
func (s *AgentSession) SessionID() string {
	return s.engine.Session.ID
}

// SessionFile returns the session file path (if any).
func (s *AgentSession) SessionFile() string {
	// xihu sessions are managed by Manager with encoded CWD
	return session.DefaultDir(s.cwd) + "/" + s.engine.Session.ID + ".jsonl"
}

// SessionName returns the session display name.
func (s *AgentSession) SessionName() string {
	return s.engine.Session.GetSessionName()
}

// SessionManager returns the underlying session manager.
func (s *AgentSession) SessionManager() *session.Manager {
	return s.engine.SessionManager
}

// SteeringMode returns the current steering mode.
func (s *AgentSession) SteeringMode() string {
	return s.steeringMode
}

// FollowUpMode returns the current follow-up mode.
func (s *AgentSession) FollowUpMode() string {
	return s.followUpMode
}

// CWD returns the working directory.
func (s *AgentSession) CWD() string {
	return s.cwd
}

// PendingMessageCount returns the number of pending messages in queues.
func (s *AgentSession) PendingMessageCount() int {
	s.mu.RLock()
	defer s.mu.RUnlock()
	return len(s.steeringMessages) + len(s.followUpMessages)
}

// =============================================================================
// Event Subscription
// =============================================================================

// Subscribe registers an event listener and returns an unsubscribe function.
func (s *AgentSession) Subscribe(listener AgentSessionEventListener) func() {
	s.mu.Lock()
	s.listeners = append(s.listeners, listener)
	s.mu.Unlock()

	return func() {
		s.mu.Lock()
		defer s.mu.Unlock()
		for i, l := range s.listeners {
			if fmt.Sprintf("%p", l) == fmt.Sprintf("%p", listener) {
				s.listeners = append(s.listeners[:i], s.listeners[i+1:]...)
				return
			}
		}
	}
}

// emit sends an event to all listeners.
func (s *AgentSession) emit(event AgentSessionEvent) {
	s.mu.RLock()
	listeners := make([]AgentSessionEventListener, len(s.listeners))
	copy(listeners, s.listeners)
	s.mu.RUnlock()

	for _, l := range listeners {
		l(event)
	}
}

// emitQueueUpdate emits a queue_update event.
func (s *AgentSession) emitQueueUpdate() {
	s.mu.RLock()
	steering := make([]string, len(s.steeringMessages))
	copy(steering, s.steeringMessages)
	followUp := make([]string, len(s.followUpMessages))
	copy(followUp, s.followUpMessages)
	s.mu.RUnlock()

	s.emit(AgentSessionEvent{
		Type:     "queue_update",
		Steering: steering,
		FollowUp: followUp,
	})
}

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

	// Run the agent loop
	finalText, finalMessages, err := s.engine.Loop.RunStreamingWithMessages(ctx, messages, func(text string) {
		// onText callback — could forward as streaming event
	})

	// Emit agent_end
	s.emit(AgentSessionEvent{Type: "agent_end"})

	if err != nil {
		s.emit(AgentSessionEvent{Type: "error", ErrorMessage: err.Error()})
		return err
	}

	// Save to session
	newEntries := session.MessagesToEntries(finalMessages, "")
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
	select {
	case s.engine.Loop.SteeringQueue <- text:
	default:
		// Channel full, message will be visible in queue_update
	}
	return nil
}

// queueFollowUp adds a message to the follow-up queue.
func (s *AgentSession) queueFollowUp(text string) error {
	s.mu.Lock()
	s.followUpMessages = append(s.followUpMessages, text)
	s.mu.Unlock()

	s.emitQueueUpdate()

	// Also push to the loop's followUp channel
	select {
	case s.engine.Loop.FollowUpQueue <- text:
	default:
	}
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
	s.engine.Loop.SteeringMode = mode
}

// SetFollowUpMode sets how follow-up messages are processed.
func (s *AgentSession) SetFollowUpMode(mode string) {
	s.followUpMode = mode
	s.engine.Loop.FollowUpMode = mode
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
		Type:     "compaction_end",
		Reason:   "manual",
		Result:   result,
		Aborted:  false,
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

// =============================================================================
// Bash Execution
// =============================================================================

// BashResult holds the result of a bash command execution.
type BashResult struct {
	Output         string `json:"output"`
	ExitCode       int    `json:"exitCode"`
	Cancelled      bool   `json:"cancelled"`
	Truncated      bool   `json:"truncated"`
	FullOutputPath string `json:"fullOutputPath,omitempty"`
}

// ExecuteBash executes a bash command and records it in session context.
func (s *AgentSession) ExecuteBash(command string) (*BashResult, error) {
	handler := tools.BashTool().Handler

	args, _ := json.Marshal(map[string]interface{}{
		"command": command,
		"timeout": 120000,
	})

	output, err := handler(args)
	if err != nil {
		return &BashResult{
			Output:   output,
			ExitCode: 1,
		}, err
	}

	return &BashResult{
		Output:   output,
		ExitCode: 0,
	}, nil
}

// =============================================================================
// Session Statistics
// =============================================================================

// SessionStats holds session usage statistics.
type SessionStats struct {
	SessionFile       string     `json:"sessionFile,omitempty"`
	SessionID         string     `json:"sessionId"`
	UserMessages      int        `json:"userMessages"`
	AssistantMessages int        `json:"assistantMessages"`
	ToolCalls         int        `json:"toolCalls"`
	ToolResults       int        `json:"toolResults"`
	TotalMessages     int        `json:"totalMessages"`
	Tokens            TokenStats `json:"tokens"`
	Cost              float64    `json:"cost"`
}

// TokenStats holds token usage breakdown.
type TokenStats struct {
	Input     int `json:"input"`
	Output    int `json:"output"`
	CacheRead int `json:"cacheRead"`
	Total     int `json:"total"`
}

// GetSessionStats returns session usage statistics.
func (s *AgentSession) GetSessionStats() *SessionStats {
	var userCount, assistantCount, toolResultsCount, toolCallsCount int
	var totalInput, totalOutput int
	var totalCost float64

	for _, entry := range s.engine.Session.Entries {
		switch entry.Type {
		case session.EntryTypeUser:
			userCount++
		case session.EntryTypeAssistant:
			assistantCount++
			if len(entry.ToolCalls) > 0 {
				toolCallsCount += len(entry.ToolCalls)
			}
		case session.EntryTypeTool:
			toolResultsCount++
		}
	}

	_ = totalInput
	_ = totalOutput
	_ = totalCost

	return &SessionStats{
		SessionFile:       s.SessionFile(),
		SessionID:         s.SessionID(),
		UserMessages:      userCount,
		AssistantMessages: assistantCount,
		ToolCalls:         toolCallsCount,
		ToolResults:       toolResultsCount,
		TotalMessages:     len(s.engine.Session.Entries),
		Tokens: TokenStats{
			Input:  totalInput,
			Output: totalOutput,
			Total:  totalInput + totalOutput,
		},
		Cost: totalCost,
	}
}

// =============================================================================
// Session Management
// =============================================================================

// NewSession starts a fresh session.
func (s *AgentSession) NewSession() error {
	s.engine.Session = &session.Session{
		ID:        session.GenerateID(),
		CWD:       s.cwd,
		Model:     s.engine.Model,
		BaseURL:   s.engine.Session.GetBaseURL(),
		CreatedAt: time.Now(),
	}
	return nil
}

// SetSessionName sets the session display name.
func (s *AgentSession) SetSessionName(name string) {
	s.engine.Session.SetSessionName(name)
	s.emit(AgentSessionEvent{
		Type: "session_info_changed",
		Name: name,
	})
}

// GetLastAssistantText returns the last assistant message text.
func (s *AgentSession) GetLastAssistantText() string {
	entries := s.engine.Session.Entries
	for i := len(entries) - 1; i >= 0; i-- {
		if entries[i].Type == session.EntryTypeAssistant {
			var content []types.TextContent
			if err := json.Unmarshal(entries[i].Content, &content); err == nil {
				for _, c := range content {
					if c.Type == "text" {
						return c.Text
					}
				}
			}
		}
	}
	return ""
}

// GetMessages returns all session entries as messages.
func (s *AgentSession) GetMessages() []types.Message {
	if len(s.engine.Session.Entries) == 0 {
		return nil
	}
	return session.BuildContext(s.engine.Session.Entries)
}

// Dispose cleans up the agent session.
func (s *AgentSession) Dispose() {
	s.Abort()
	s.AbortRetry()

	s.mu.Lock()
	defer s.mu.Unlock()

	s.listeners = nil
	s.steeringMessages = nil
	s.followUpMessages = nil
}

// =============================================================================
// Internal helpers
// =============================================================================

func (s *AgentSession) getLeafParentID() string {
	entries := s.engine.Session.Entries
	if len(entries) == 0 {
		return ""
	}
	// Find the most recent user/assistant entry to attach to
	// Simple approach: use the last non-compaction entry ID
	for i := len(entries) - 1; i >= 0; i-- {
		if entries[i].Type != session.EntryTypeCompaction {
			return entries[i].ID
		}
	}
	return ""
}

// =============================================================================
// Fork / Clone support
// =============================================================================

// ForkForks creates a new session forked from a specific entry.
func (s *AgentSession) Fork(entryID string) (*AgentSession, error) {
	newSess := &session.Session{
		ID:        session.GenerateID(),
		CWD:       s.cwd,
		Model:     s.engine.Model,
		BaseURL:   s.engine.Session.GetBaseURL(),
		Name:      s.engine.Session.Name + " (fork)",
		ParentSessionID: s.engine.Session.ID,
		CreatedAt: time.Now(),
	}

	// Copy entries up to the fork point
	for _, entry := range s.engine.Session.Entries {
		newSess.Entries = append(newSess.Entries, entry)
		if entry.ID == entryID {
			break
		}
	}

	// Create new engine options reusing the current one
	opts := engine.EngineOptions{
		BaseURL:        s.engine.Session.GetBaseURL(),
		APIKey:         "", // TODO: plumb through
		Model:          s.engine.Model,
		CWD:            s.cwd,
		SessionManager: s.engine.SessionManager,
	}

	newEng, err := engine.NewEngine(opts)
	if err != nil {
		return nil, fmt.Errorf("fork: %w", err)
	}
	newEng.Session = newSess

	cfg := AgentSessionConfig{
		Engine:        newEng,
		CWD:           s.cwd,
		ScopedModels:  s.scopedModels,
		MaxRetries:    s.maxRetries,
		AutoCompaction: s.autoCompaction,
		AutoRetry:      s.autoRetry,
	}

	return New(cfg)
}

// GetUserMessagesForFork returns user messages usable for forking.
func (s *AgentSession) GetUserMessagesForFork() []ForkMessage {
	var result []ForkMessage
	for _, entry := range s.engine.Session.Entries {
		if entry.Type == session.EntryTypeUser {
			var content []types.TextContent
			if err := json.Unmarshal(entry.Content, &content); err == nil {
				for _, c := range content {
					if c.Type == "text" && c.Text != "" {
						result = append(result, ForkMessage{
							EntryID: entry.ID,
							Text:    c.Text,
						})
					}
				}
			}
		}
	}
	return result
}

// ForkMessage is a user message suitable for forking.
type ForkMessage struct {
	EntryID string `json:"entry_id"`
	Text    string `json:"text"`
}

// SlashCommand describes a command available for invocation.
// Mirrors pi-mono's RpcSlashCommand.
type SlashCommand struct {
	Name        string `json:"name"`
	Description string `json:"description,omitempty"`
	Source      string `json:"source"` // "extension" | "prompt" | "skill"
}

// GetCommands returns all available commands from extensions, prompts, and skills.
// Mirrors pi-mono's getCommands().
func (s *AgentSession) GetCommands() []SlashCommand {
	var cmds []SlashCommand

	// Extension slash commands
	for cmdName := range extensions.GetAllSlashCommands() {
		cmds = append(cmds, SlashCommand{
			Name:        strings.TrimPrefix(cmdName, "/"),
			Description: fmt.Sprintf("Extension command: %s", cmdName),
			Source:      "extension",
		})
	}

	// Prompt templates
	for name := range extensions.GetAllPrompts() {
		cmds = append(cmds, SlashCommand{
			Name:        name,
			Description: "Prompt template",
			Source:      "prompt",
		})
	}

	return cmds
}
