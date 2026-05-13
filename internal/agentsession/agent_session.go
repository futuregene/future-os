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
	"fmt"
	"log"
	"sync"
	"time"

	"github.com/huichen/xihu/internal/agent"
	"github.com/huichen/xihu/internal/compaction"
	"github.com/huichen/xihu/internal/engine"
	"github.com/huichen/xihu/internal/session"
)

// =============================================================================
// AgentSessionEvent — events emitted by AgentSession
// =============================================================================

// AgentSessionEvent is a union of all events that AgentSession can emit.
type AgentSessionEvent struct {
	Type string `json:"type"` // event type

	// agent_start / agent_end / turn_* / message_* / tool_* / stop / error / usage
	// (forwarded from agent loop, same as events.EventType)

	// text_chunk — streaming text emitted during prompt/steer/follow_up
	Text string `json:"text,omitempty"`

	// queue_update
	Steering []string `json:"steering,omitempty"`
	FollowUp []string `json:"follow_up,omitempty"`

	// compaction_start / compaction_end
	Reason       string                       `json:"reason,omitempty"` // "manual" | "threshold" | "overflow"
	Result       *compaction.CompactionResult `json:"result,omitempty"`
	Aborted      bool                         `json:"aborted,omitempty"`
	WillRetry    bool                         `json:"will_retry,omitempty"`
	ErrorMessage string                       `json:"error_message,omitempty"`

	// auto_retry_start / auto_retry_end
	Attempt     int    `json:"attempt,omitempty"`
	MaxAttempts int    `json:"max_attempts,omitempty"`
	DelayMs     int    `json:"delay_ms,omitempty"`
	Success     bool   `json:"success,omitempty"`
	FinalError  string `json:"final_error,omitempty"`

	// thinking_level_changed
	Level string `json:"level,omitempty"`

	// session_info_changed
	Name string `json:"name,omitempty"`

	// tool_start / tool_delta / tool_end
	ToolID   string `json:"tool_id,omitempty"`
	ToolName string `json:"tool_name,omitempty"`
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
	SteeringModeAll        = "all"
	SteeringModeOneAtATime = "one-at-a-time"
	FollowUpModeAll        = "all"
	FollowUpModeOneAtATime = "one-at-a-time"
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

	// Event-driven persistence: debounced auto-save on agent_end/turn_end/message_end
	saveTimer   *time.Timer
	saveMu      sync.Mutex
	savePending bool
}

// New creates a new AgentSession from config.
func New(cfg AgentSessionConfig) (*AgentSession, error) {
	cfg.defaults()

	eng := cfg.Engine
	if eng == nil {
		return nil, fmt.Errorf("agentsession: Engine is required")
	}

	s := &AgentSession{
		engine:         eng,
		cwd:            cfg.CWD,
		scopedModels:   cfg.ScopedModels,
		maxRetries:     cfg.MaxRetries,
		autoCompaction: cfg.AutoCompaction,
		autoRetry:      cfg.AutoRetry,
		steeringMode:   SteeringModeAll,
		followUpMode:   FollowUpModeAll,
	}

	// Wire up event-driven session persistence (non-blocking, debounced)
	s.setupAutoSave()

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

// ContextStats holds context usage information.
type ContextStats struct {
	Tokens int     // estimated current context size in tokens
	Window int     // model context window in tokens
	Percent float64 // usage percentage 0-100
}

// GetContextStats returns estimated context usage from session messages.
func (s *AgentSession) GetContextStats() ContextStats {
	s.mu.RLock()
	defer s.mu.RUnlock()

	// Build context messages from session entries
	entries := s.engine.Session.Entries
	messages := session.BuildContext(entries)

	// Estimate token count
	tokens := compaction.EstimateContextTokens(messages)

	// Get context window from model info
	window := s.engine.ModelInfo.ContextWindow

	var percent float64
	if window > 0 {
		percent = float64(tokens) / float64(window) * 100
	}

	return ContextStats{
		Tokens:  tokens,
		Window:  window,
		Percent: percent,
	}
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

// setupAutoSave wires up a self-subscriber that triggers a debounced session
// save on agent_end, turn_end, and message_end events. The save is non-blocking.
func (s *AgentSession) setupAutoSave() {
	s.Subscribe(func(event AgentSessionEvent) {
		switch event.Type {
		case "agent_end", "turn_end", "message_end":
			s.debouncedSave()
		}
	})
}

// debouncedSave triggers a non-blocking, debounced session save. Multiple
// rapid calls within the debounce window (200ms) are coalesced into a single
// save operation.
func (s *AgentSession) debouncedSave() {
	s.saveMu.Lock()
	defer s.saveMu.Unlock()

	if s.savePending {
		return // already scheduled
	}
	s.savePending = true

	if s.saveTimer != nil {
		s.saveTimer.Stop()
	}

	s.saveTimer = time.AfterFunc(200*time.Millisecond, func() {
		s.saveMu.Lock()
		s.savePending = false
		s.saveMu.Unlock()

		if s.engine != nil && s.engine.SessionManager != nil && s.engine.Session != nil {
			if err := s.engine.SessionManager.Save(s.engine.Session); err != nil {
				log.Printf("agentsession: auto-save session: %v", err)
			}
		}
	})
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
