package types

import "encoding/json"

// ---------------------------------------------------------------------------
// AgentMessage — internal conversation representation
// Separated from LLM wire format (Message) following TS pi-mono pattern.
// The agent loop works with AgentMessage internally; ConvertToLLM() produces
// the LLM-compatible Message slice before each API call.
// ---------------------------------------------------------------------------

// AgentMessage represents an internal conversation message.
// Unlike Message (which directly mirrors the LLM wire format), AgentMessage
// uses typed content, explicit thinking/reasoning separation, and supports
// metadata that should NOT be sent to the LLM.
type AgentMessage struct {
	Role    string        `json:"role"`              // "system", "user", "assistant", "tool"
	Content string        `json:"content"`           // plain text content (user/assistant text)
	Thinking string       `json:"thinking,omitempty"` // reasoning/thinking that stays internal
	ToolCalls []AgentToolCall `json:"tool_calls,omitempty"` // outgoing tool calls from assistant
	ToolCallID string     `json:"tool_call_id,omitempty"`   // tool result correlation ID
	// Metadata is NOT sent to the LLM; it's for extensions, UI, and persistence.
	Metadata map[string]interface{} `json:"metadata,omitempty"`
}

// AgentToolCall is an internal tool call representation.
type AgentToolCall struct {
	ID       string          `json:"id"`
	Name     string          `json:"name"`
	Args     json.RawMessage `json:"args"`
}

// ConvertToLLM converts a slice of AgentMessages to LLM-compatible Messages.
// System messages go first (position 0). Thinking content is stored in
// ReasoningContent for provider-aware echo-back.
func ConvertToLLM(msgs []AgentMessage) []Message {
	out := make([]Message, 0, len(msgs))
	for _, m := range msgs {
		llm := m.toLLM()
		out = append(out, llm)
	}
	return out
}

func (m AgentMessage) toLLM() Message {
	llm := Message{
		Role:             m.Role,
		ReasoningContent: m.Thinking,
		ToolCallID:       m.ToolCallID,
	}

	switch m.Role {
	case "tool":
		// Tool results: wrap content as text block
		tc := TextContent{Type: "text", Text: m.Content}
		b, _ := json.Marshal([]TextContent{tc})
		llm.Content = b

	case "assistant":
		// Assistant: text + tool calls
		if m.Content != "" {
			tc := TextContent{Type: "text", Text: m.Content}
			b, _ := json.Marshal([]TextContent{tc})
			llm.Content = b
		}
		if len(m.ToolCalls) > 0 {
			llm.ToolCalls = make([]ToolCall, len(m.ToolCalls))
			for i, tc := range m.ToolCalls {
				llm.ToolCalls[i] = ToolCall{
					ID:   tc.ID,
					Type: "function",
					Function: ToolCallFn{
						Name:      tc.Name,
						Arguments: tc.Args,
					},
				}
			}
		}

	default:
		// system, user: plain text content
		if m.Content != "" {
			tc := TextContent{Type: "text", Text: m.Content}
			b, _ := json.Marshal([]TextContent{tc})
			llm.Content = b
		}
	}

	return llm
}

// ConvertFromLLM converts LLM Messages back to AgentMessages.
func ConvertFromLLM(msgs []Message) []AgentMessage {
	out := make([]AgentMessage, 0, len(msgs))
	for _, m := range msgs {
		out = append(out, agentMessageFromLLM(m))
	}
	return out
}

func agentMessageFromLLM(m Message) AgentMessage {
	am := AgentMessage{
		Role:       m.Role,
		Thinking:   m.ReasoningContent,
		ToolCallID: m.ToolCallID,
	}

	// Extract text content
	if len(m.Content) > 0 {
		var blocks []TextContent
		if err := json.Unmarshal(m.Content, &blocks); err == nil {
			for _, b := range blocks {
				if b.Type == "text" && b.Text != "" {
					am.Content += b.Text
				}
			}
		}
	}

	// Convert tool calls
	if len(m.ToolCalls) > 0 {
		am.ToolCalls = make([]AgentToolCall, len(m.ToolCalls))
		for i, tc := range m.ToolCalls {
			am.ToolCalls[i] = AgentToolCall{
				ID:   tc.ID,
				Name: tc.Function.Name,
				Args: tc.Function.Arguments,
			}
		}
	}

	return am
}

// AgentMessageFromLLM converts a single LLM Message to AgentMessage.
func AgentMessageFromLLM(m Message) AgentMessage {
	return agentMessageFromLLM(m)
}

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

// ImageContent is an image content block for multimodal models.
// Matches pi-mono's ImageContent type for @file image attachments.
type ImageContent struct {
	Type     string     `json:"type"`
	MimeType string     `json:"mime_type,omitempty"`
	Data     string     `json:"data,omitempty"`
	Source   *ImageSource `json:"source,omitempty"`
}

// ImageSource is used in Anthropic-format image content blocks.
type ImageSource struct {
	Type      string `json:"type"`
	MediaType string `json:"media_type"`
	Data      string `json:"data"`
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
	PromptTokens      int `json:"prompt_tokens"`
	CompletionTokens  int `json:"completion_tokens"`
	TotalTokens       int `json:"total_tokens"`
	CacheReadTokens   int `json:"cache_read_tokens,omitempty"`
	CacheWriteTokens  int `json:"cache_write_tokens,omitempty"`
}

// StreamEvent is emitted during streaming.
// Type is one of: text_start, text_delta, text_end,
// thinking_start, thinking_delta, thinking_end,
// toolcall_start, toolcall_delta, toolcall_end,
// tool_call (legacy), stop, error, usage.
type StreamEvent struct {
	Type       string    // event type (see above)
	Text       string    // delta text content
	ToolCall   *ToolCall // complete tool call (tool_call / toolcall_end)
	ToolName   string    // tool name (toolcall_start)
	ToolID     string    // tool call ID (toolcall_start)
	Usage      *Usage    // set when Type is "usage" (final usage stats)
	StopReason string    // Anthropic stop_reason, mapped: "stop"|"length"|"toolUse"|"error"|"aborted" (TS pi-mono)
	ErrorText  string    // error message when StopReason is "error" (TS pi-mono: refusal)
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

	// BeforeToolCall is called before executing each tool call.
	// Return nil/empty to proceed normally. Return a non-empty ToolCallResult
	// to skip execution and use the provided result/error directly.
	// TS pi-mono: beforeToolCall hook in the 3-stage pipeline.
	BeforeToolCall func(toolName string, toolCallID string, args json.RawMessage) *ToolCallResult

	// PrepareToolCall is called before executing each tool call (after BeforeToolCall).
	// It can transform the tool arguments (e.g., path sanitization, value coercion).
	// Return the modified args. Return nil to use the original args unchanged.
	// TS pi-mono: prepareToolCall stage in the 3-stage pipeline.
	PrepareToolCall func(toolName string, args json.RawMessage) json.RawMessage

	// FinalizeToolCall is called after executing each tool call (before AfterToolCall).
	// It can transform the tool result (e.g., truncation, redaction, formatting).
	// Return the final result and error to use.
	// TS pi-mono: finalizeToolCall stage in the 3-stage pipeline.
	FinalizeToolCall func(toolName string, result string, execErr error) (finalResult string, finalErr error)

	// AfterToolCall is called after executing each tool call.
	// The result can be modified by returning a non-nil ToolCallResult.
	// TS pi-mono: afterToolCall hook in the 3-stage pipeline.
	AfterToolCall func(toolName string, toolCallID string, args json.RawMessage, result string, execErr error) *ToolCallResult

	// ToolsExecutionMode is the tool execution strategy: "parallel" (default) or "sequential".
	// "parallel" executes multiple tool calls concurrently via goroutines.
	// "sequential" executes them one at a time, in order.
	// TS pi-mono: executionMode field in agent loop.
	ToolsExecutionMode string
}

// ToolCallResult allows hooks to override or intercept tool execution.
// If Result is non-empty, the tool is NOT executed and Result is used as the output.
// If IsError is true, Result is treated as an error message.
type ToolCallResult struct {
	Result  string // tool output (used if non-empty, skipping execution)
	IsError bool   // treat Result as an error
}

// Model identifies a model with full metadata, mirroring TS pi-mono's Model<Api>.
type Model struct {
	ID       string `json:"id"`
	Name     string `json:"name"`
	Provider string `json:"provider"`
	API      string `json:"api"` // "openai-completions", "anthropic-messages", etc.
	BaseURL  string `json:"baseUrl"`
	ContextWindow int  `json:"contextWindow"`
	MaxTokens     int  `json:"maxTokens"`
	Reasoning     bool `json:"reasoning"`
	// InputTypes lists supported input modalities: "text", "image"
	InputTypes []string `json:"input,omitempty"`
	// Cost per 1M tokens (0 = unknown)
	Cost struct {
		Input     float64 `json:"input"`
		Output    float64 `json:"output"`
		CacheRead float64 `json:"cacheRead"`
		CacheWrite float64 `json:"cacheWrite"`
	} `json:"cost,omitempty"`
	// ThinkingLevelMap maps thinking level names to provider-specific values
	ThinkingLevelMap map[string]interface{} `json:"thinkingLevelMap,omitempty"`
	// Headers are custom HTTP headers added to requests for this model
	Headers map[string]string `json:"headers,omitempty"`
	// Compat holds provider-specific compatibility flags
	Compat interface{} `json:"compat,omitempty"`
}

// LLMProvider abstracts streaming chat across providers
type LLMProvider interface {
	StreamChat(model string, messages []Message, tools []ToolDef, systemPrompt string) (<-chan StreamEvent, error)
}
