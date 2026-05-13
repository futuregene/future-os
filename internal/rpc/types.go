package rpc

import "github.com/huichen/xihu/pkg/types"

// =============================================================================
// RPC Commands (stdin) — mirrors pi-mono rpc-types.ts
// =============================================================================

// RpcCommand is the union of all possible RPC commands.
type RpcCommand struct {
	ID string `json:"id,omitempty"`

	// Command type
	Type string `json:"type"`

	// Prompting
	Message           string              `json:"message,omitempty"`
	Images            []types.ImageContent `json:"images,omitempty"`
	StreamingBehavior string              `json:"streamingBehavior,omitempty"` // "steer" | "followUp"

	// new_session
	ParentSession string `json:"parentSession,omitempty"`

	// set_model
	Provider string `json:"provider,omitempty"`
	ModelID  string `json:"modelId,omitempty"`

	// set_thinking_level
	Level string `json:"level,omitempty"` // "off" | "minimal" | "low" | "medium" | "high" | "xhigh"

	// set_steering_mode / set_follow_up_mode
	Mode string `json:"mode,omitempty"` // "all" | "one-at-a-time"

	// compact
	CustomInstructions string `json:"customInstructions,omitempty"`

	// set_auto_compaction / set_auto_retry
	Enabled bool `json:"enabled,omitempty"`

	// bash
	Command string `json:"command,omitempty"`

	// Session
	SessionPath string `json:"sessionPath,omitempty"`
	SessionID   string `json:"sessionId,omitempty"`
	EntryID     string `json:"entryId,omitempty"`
	Name        string `json:"name,omitempty"`
	OutputPath  string `json:"outputPath,omitempty"`
}

// =============================================================================
// RPC Responses (stdout)
// =============================================================================

// RpcResponse is a response to a command.
type RpcResponse struct {
	ID      string      `json:"id,omitempty"`
	Type    string      `json:"type"`    // always "response"
	Command string      `json:"command"` // the command this responds to
	Success bool        `json:"success"`
	Data    interface{} `json:"data,omitempty"`
	Error   string      `json:"error,omitempty"`
}

// =============================================================================
// RPC Session State
// =============================================================================

// RpcSessionState is returned by get_state.
type RpcSessionState struct {
	Model                string   `json:"model,omitempty"`
	ThinkingLevel        string   `json:"thinkingLevel"`
	IsStreaming          bool     `json:"isStreaming"`
	IsCompacting         bool     `json:"isCompacting"`
	SteeringMode         string   `json:"steeringMode"`
	FollowUpMode         string   `json:"followUpMode"`
	SessionFile          string   `json:"sessionFile,omitempty"`
	SessionID            string   `json:"sessionId"`
	SessionName          string   `json:"sessionName,omitempty"`
	AutoCompactionEnabled bool     `json:"autoCompactionEnabled"`
	MessageCount         int      `json:"messageCount"`
	PendingMessageCount  int      `json:"pendingMessageCount"`
	// Welcome info (populated in server mode, empty in one-shot mode)
	Version       string   `json:"version,omitempty"`
	CWD           string   `json:"cwd,omitempty"`
	Skills        []string `json:"skills,omitempty"`
	ContextFiles  []string `json:"contextFiles,omitempty"`
	Extensions    []string `json:"extensions,omitempty"`
	// Context usage
	ContextTokens  int     `json:"contextTokens,omitempty"`
	ContextWindow  int     `json:"contextWindow,omitempty"`
	ContextPercent float64  `json:"contextPercent,omitempty"`
}

// =============================================================================
// Extension UI Events (bidirectional)
// =============================================================================

// RpcExtensionUIRequest is emitted when an extension needs user input.
type RpcExtensionUIRequest struct {
	Type   string `json:"type"` // "extension_ui_request"
	ID     string `json:"id"`
	Method string `json:"method"` // "select" | "confirm" | "input" | "editor" | "notify" | "setStatus" | "setWidget" | "setTitle" | "set_editor_text"

	// select / confirm / input / editor
	Title       string   `json:"title,omitempty"`
	Message     string   `json:"message,omitempty"`
	Options     []string `json:"options,omitempty"`
	Placeholder string   `json:"placeholder,omitempty"`
	Prefill     string   `json:"prefill,omitempty"`
	Timeout     int      `json:"timeout,omitempty"`

	// notify
	NotifyType string `json:"notifyType,omitempty"` // "info" | "warning" | "error"

	// setStatus
	StatusKey  string `json:"statusKey,omitempty"`
	StatusText string `json:"statusText,omitempty"`

	// setWidget
	WidgetKey       string   `json:"widgetKey,omitempty"`
	WidgetLines     []string `json:"widgetLines,omitempty"`
	WidgetPlacement string   `json:"widgetPlacement,omitempty"` // "aboveEditor" | "belowEditor"

	// setTitle / set_editor_text
	Text string `json:"text,omitempty"`
}

// RpcExtensionUIResponse is the response to an extension UI request.
type RpcExtensionUIResponse struct {
	Type      string `json:"type"` // "extension_ui_response"
	ID        string `json:"id"`
	Value     string `json:"value,omitempty"`
	Confirmed bool   `json:"confirmed,omitempty"`
	Cancelled bool   `json:"cancelled,omitempty"`
}
