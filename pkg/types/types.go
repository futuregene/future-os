package types

import "encoding/json"

// Message represents a conversation message
type Message struct {
	Role    string          `json:"role"`
	Content json.RawMessage `json:"content,omitempty"`
	ToolCalls []ToolCall    `json:"tool_calls,omitempty"`
	ToolCallID string       `json:"tool_call_id,omitempty"`
	Name       string       `json:"name,omitempty"`
	// ReasoningContent preserves thinking/reasoning tokens that must be
	// echoed back on subsequent API requests (required by DeepSeek, o-series, etc.)
	ReasoningContent string `json:"reasoning_content,omitempty"`
}

// TextContent is a text content block
type TextContent struct {
	Type string `json:"type"`
	Text string `json:"text"`
}

// ToolCall represents a tool call from the assistant
type ToolCall struct {
	ID       string          `json:"id"`
	Type     string          `json:"type"`
	Function ToolCallFn      `json:"function"`
}

type ToolCallFn struct {
	Name      string          `json:"name"`
	Arguments json.RawMessage `json:"arguments"`
}

// Usage tracks token consumption from the API response
type Usage struct {
	PromptTokens     int `json:"prompt_tokens"`
	CompletionTokens int `json:"completion_tokens"`
	TotalTokens      int `json:"total_tokens"`
}

// StreamEvent is emitted during streaming.
// Type is one of: text_start, text_delta, text_end,
// thinking_start, thinking_delta, thinking_end,
// toolcall_start, toolcall_delta, toolcall_end,
// tool_call (legacy), stop, error, usage.
type StreamEvent struct {
	Type     string    // event type (see above)
	Text     string    // delta text content
	ToolCall *ToolCall // complete tool call (tool_call / toolcall_end)
	ToolName string    // tool name (toolcall_start)
	ToolID   string    // tool call ID (toolcall_start)
	Usage    *Usage    // set when Type is "usage" (final usage stats)
}

// Tool definition for the LLM
type ToolDef struct {
	Type     string       `json:"type"`
	Function FunctionDef  `json:"function"`
}

type FunctionDef struct {
	Name        string             `json:"name"`
	Description string             `json:"description"`
	Parameters  json.RawMessage    `json:"parameters"`
}

// AgentTool wraps a tool definition with a handler
type AgentTool struct {
	Def        ToolDef
	Handler    func(args json.RawMessage) (string, error)
	Guidelines []string
}

// AgentConfig configures the agent loop
type AgentConfig struct {
	SystemPrompt   string
	MaxTurns       int
	ThinkingBudget int // tokens for thinking
	MaxRetries     int // max auto-retry attempts (0 = no retry)

	// TransformContext is called before each LLM call to transform the message list.
	// Useful for compaction, injection, or other context manipulation.
	// The string argument is the current full response text so far.
	TransformContext func([]Message, string) []Message

	// StopCondition is checked after each turn. If it returns true, the loop stops early
	// even if tool calls remain or MaxTurns hasn't been reached.
	StopCondition func(messages []Message, lastResponse string) bool
}

// Model identifies a model
type Model struct {
	ID       string
	Provider string
	API      string // "openai-completions", "anthropic-messages"
	BaseURL  string
}

// LLMProvider abstracts streaming chat across providers
type LLMProvider interface {
	StreamChat(model string, messages []Message, tools []ToolDef, systemPrompt string) (<-chan StreamEvent, error)
}
