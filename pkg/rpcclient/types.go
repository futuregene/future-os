// Package rpcclient provides the Go RPC client for interacting with xihu in RPC mode.
// It mirrors the pi-mono TypeScript RpcClient class (rpc-client.ts) with all
// 29 commands and helper methods (waitForIdle, collectEvents, promptAndWait).
//
// Usage:
//
//	client := rpcclient.New(rpcclient.Options{
//		Provider: "openai",
//		Model:    "gpt-4o",
//	})
//	err := client.Start(context.Background())
//	defer client.Stop()
//
//	// Subscribe to events
//	client.OnEvent(func(event json.RawMessage) {
//		fmt.Println("event:", string(event))
//	})
//
//	// Send prompt
//	client.Prompt("Hello")
//	client.WaitForIdle(60000)
//
//	// Query state
//	state, _ := client.GetState()
package rpcclient

import (
	"encoding/json"
)

// =============================================================================
// Client Options — mirrors pi-mono RpcClientOptions
// =============================================================================

// Options configures the RPC client.
type Options struct {
	// CLI path to the xihu binary. If empty, uses "xihu" from PATH.
	CliPath string

	// Working directory for the agent subprocess.
	Cwd string

	// Environment variables to pass to the subprocess.
	Env []string

	// Provider to use (e.g. "openai", "anthropic", "deepseek").
	Provider string

	// Model ID to use (e.g. "gpt-4o", "claude-sonnet-4").
	Model string

	// Additional CLI arguments passed to the agent.
	Args []string

	// API key for the provider (passed as --api-key).
	APIKey string
}

// =============================================================================
// Model Info — mirrors pi-mono ModelInfo
// =============================================================================

// ModelInfo describes an available model.
type ModelInfo struct {
	Provider      string `json:"provider"`
	ID            string `json:"id"`
	ContextWindow int    `json:"contextWindow"`
	Reasoning     bool   `json:"reasoning"`
}

// =============================================================================
// Event Types
// =============================================================================

// EventHandler is a callback for agent events. It mirrors pi-mono's
// RpcEventListener: (event: AgentEvent) => void.
//
// The event is passed as raw JSON; consumers can unmarshal into typed
// structures as needed.
type EventHandler func(event json.RawMessage)

// =============================================================================
// RPC Protocol Types — mirrors pi-mono rpc-types.ts
// =============================================================================

// rpcCommand is the command sent to the agent on stdin.
// These fields match pi-mono's RpcCommand union type exactly.
type rpcCommand struct {
	ID   string `json:"id,omitempty"`
	Type string `json:"type"`

	// Prompting
	Message           string          `json:"message,omitempty"`
	Images            []ImageContent  `json:"images,omitempty"`
	StreamingBehavior string          `json:"streamingBehavior,omitempty"` // "steer" | "followUp"
	ParentSession     string          `json:"parentSession,omitempty"`

	// Model
	Provider string `json:"provider,omitempty"`
	ModelID  string `json:"modelId,omitempty"`

	// Thinking
	Level string `json:"level,omitempty"` // "off" | "minimal" | "low" | "medium" | "high" | "xhigh"

	// Queue modes
	Mode string `json:"mode,omitempty"` // "all" | "one-at-a-time"

	// Compaction
	CustomInstructions string `json:"customInstructions,omitempty"`

	// Auto compaction / retry
	Enabled bool `json:"enabled,omitempty"`

	// Bash
	Command string `json:"command,omitempty"`

	// Session
	SessionPath string `json:"sessionPath,omitempty"`
	EntryID     string `json:"entryId,omitempty"`
	Name        string `json:"name,omitempty"`
	OutputPath  string `json:"outputPath,omitempty"`
}

// ImageContent matches pi-mono's ImageContent for multimodal messages.
type ImageContent struct {
	Type     string       `json:"type"`
	MimeType string       `json:"mime_type,omitempty"`
	Data     string       `json:"data,omitempty"`
	Source   *ImageSource `json:"source,omitempty"`
}

// ImageSource for Anthropic-format image blocks.
type ImageSource struct {
	Type      string `json:"type"`
	MediaType string `json:"media_type"`
	Data      string `json:"data"`
}

// =============================================================================
// Response Types
// =============================================================================

// rpcResponse is a response to a command. Mirrors pi-mono's RpcResponse.
type rpcResponse struct {
	ID      string          `json:"id,omitempty"`
	Type    string          `json:"type"`    // always "response"
	Command string          `json:"command"` // the command this responds to
	Success bool            `json:"success"`
	Data    json.RawMessage `json:"data,omitempty"`
	Error   string          `json:"error,omitempty"`
}

// =============================================================================
// Session State — mirrors pi-mono RpcSessionState
// =============================================================================

// SessionState is returned by GetState.
type SessionState struct {
	Model                 string `json:"model,omitempty"`
	ThinkingLevel         string `json:"thinkingLevel"`
	IsStreaming           bool   `json:"isStreaming"`
	IsCompacting          bool   `json:"isCompacting"`
	SteeringMode          string `json:"steeringMode"`
	FollowUpMode          string `json:"followUpMode"`
	SessionFile           string `json:"sessionFile,omitempty"`
	SessionID             string `json:"sessionId"`
	SessionName           string `json:"sessionName,omitempty"`
	AutoCompactionEnabled bool   `json:"autoCompactionEnabled"`
	MessageCount          int    `json:"messageCount"`
	PendingMessageCount   int    `json:"pendingMessageCount"`
}

// =============================================================================
// Session Stats — mirrors pi-mono SessionStats
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

// =============================================================================
// Bash Result — mirrors pi-mono BashResult
// =============================================================================

// BashResult holds the result of a bash command execution.
type BashResult struct {
	Output         string `json:"output"`
	ExitCode       int    `json:"exitCode"`
	Cancelled      bool   `json:"cancelled"`
	Truncated      bool   `json:"truncated"`
	FullOutputPath string `json:"fullOutputPath,omitempty"`
}

// =============================================================================
// Compaction Result — mirrors pi-mono CompactionResult
// =============================================================================

// CompactionResult holds the result of a compaction operation.
type CompactionResult struct {
	Summary          string `json:"summary"`
	FirstKeptEntryID string `json:"firstKeptEntryId"`
	TokensBefore     int    `json:"tokensBefore"`
}

// =============================================================================
// Slash Command — mirrors pi-mono RpcSlashCommand
// =============================================================================

// SlashCommandInfo is a command available for invocation via prompt.
type SlashCommandInfo struct {
	Name        string     `json:"name"`
	Description string     `json:"description,omitempty"`
	Source      string     `json:"source"` // "extension" | "prompt" | "skill"
	SourceInfo  SourceInfo `json:"sourceInfo"`
}

// SourceInfo carries metadata about where a resource originates.
type SourceInfo struct {
	Path    string `json:"path"`
	Source  string `json:"source"`
	Scope   string `json:"scope"`
	Origin  string `json:"origin"`
	BaseDir string `json:"baseDir,omitempty"`
}

// =============================================================================
// Fork Messages — mirrors pi-mono get_fork_messages response
// =============================================================================

// ForkMessage is a message available for forking.
type ForkMessage struct {
	EntryID string `json:"entryId"`
	Text    string `json:"text"`
}
