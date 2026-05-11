package agentsession

import (
	"encoding/json"
	"fmt"
	"strings"
	"time"

	"github.com/huichen/xihu/internal/engine"
	"github.com/huichen/xihu/internal/extensions"
	"github.com/huichen/xihu/internal/session"
	"github.com/huichen/xihu/internal/tools"
	"github.com/huichen/xihu/pkg/types"
)

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

// Fork creates a new session forked from a specific entry.
func (s *AgentSession) Fork(entryID string) (*AgentSession, error) {
	newSess := &session.Session{
		ID:              session.GenerateID(),
		CWD:             s.cwd,
		Model:           s.engine.Model,
		BaseURL:         s.engine.Session.GetBaseURL(),
		Name:            s.engine.Session.Name + " (fork)",
		ParentSessionID: s.engine.Session.ID,
		CreatedAt:       time.Now(),
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
		Engine:         newEng,
		CWD:            s.cwd,
		ScopedModels:   s.scopedModels,
		MaxRetries:     s.maxRetries,
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
