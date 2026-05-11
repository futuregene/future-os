package types

import (
	"encoding/json"
	"strings"
)

// ---------------------------------------------------------------------------
// AgentMessage — internal conversation representation
// Separated from LLM wire format (Message) following TS pi-mono pattern.
// The agent loop works with AgentMessage internally; ConvertToLLM() produces
// the LLM-compatible Message slice before each API call.
// ---------------------------------------------------------------------------

// AgentMessage represents an internal conversation message.
// Content uses ContentBlock interface for polymorphic blocks (text, image_url, tool_result)
// matching TS pi-mono's ContentBlock[] union type.
type AgentMessage struct {
	Role      string           `json:"role"`
	Content   []ContentBlock   `json:"content"`
	Thinking  string           `json:"thinking,omitempty"`
	ToolCalls []AgentToolCall  `json:"tool_calls,omitempty"`
	ToolCallID string          `json:"tool_call_id,omitempty"`
	Metadata  map[string]interface{} `json:"metadata,omitempty"`
}

// AgentToolCall is an internal tool call representation.
type AgentToolCall struct {
	ID   string          `json:"id"`
	Name string          `json:"name"`
	Args json.RawMessage `json:"args"`
}

// ─── ContentBlock interface & implementations ───────────────────────────────

// ContentBlock is the interface for polymorphic message content.
type ContentBlock interface {
	BlockType() string
	json.Marshaler
}

// TextBlock is a plain text content block.
type TextBlock struct {
	Text string `json:"text"`
}

func (b TextBlock) BlockType() string { return "text" }
func (b TextBlock) MarshalJSON() ([]byte, error) {
	return json.Marshal(map[string]string{"type": "text", "text": b.Text})
}

// ImageBlock is an image content block for multimodal models.
type ImageBlock struct {
	MimeType string `json:"mime_type,omitempty"`
	Data     string `json:"data,omitempty"`
	URL      string `json:"url,omitempty"`
}

func (b ImageBlock) BlockType() string { return "image_url" }
func (b ImageBlock) MarshalJSON() ([]byte, error) {
	m := map[string]interface{}{"type": "image_url"}
	if b.URL != "" {
		m["image_url"] = map[string]string{"url": b.URL}
	} else {
		m["image_url"] = map[string]string{"url": "data:" + b.MimeType + ";base64," + b.Data}
	}
	return json.Marshal(m)
}

// ToolResultBlock represents a tool execution result inside a message.
type ToolResultBlock struct {
	ToolCallID string `json:"tool_call_id"`
	Content    string `json:"content"`
	IsError    bool   `json:"is_error,omitempty"`
}

func (b ToolResultBlock) BlockType() string { return "tool_result" }
func (b ToolResultBlock) MarshalJSON() ([]byte, error) {
	m := map[string]interface{}{
		"type":         "tool_result",
		"tool_call_id": b.ToolCallID,
		"content":      b.Content,
	}
	if b.IsError {
		m["is_error"] = true
	}
	return json.Marshal(m)
}

// ─── AgentMessage helpers ───────────────────────────────────────────────────

// Text returns concatenated text from all TextBlocks.
func (m AgentMessage) Text() string {
	var s string
	for _, b := range m.Content {
		if tb, ok := b.(TextBlock); ok {
			s += tb.Text
		}
	}
	return s
}

// AddText appends a TextBlock.
func (m *AgentMessage) AddText(text string) {
	m.Content = append(m.Content, TextBlock{Text: text})
}

// AddImage appends an ImageBlock.
func (m *AgentMessage) AddImage(mimeType, data string) {
	m.Content = append(m.Content, ImageBlock{MimeType: mimeType, Data: data})
}

// ConvertToLLM converts AgentMessages to LLM-compatible Messages.
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

	if len(m.Content) > 0 {
		b, _ := json.Marshal(m.Content)
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

	if len(m.Content) > 0 {
		var rawBlocks []json.RawMessage
		if err := json.Unmarshal(m.Content, &rawBlocks); err == nil {
			for _, raw := range rawBlocks {
				var tc struct{ Type string }
				if json.Unmarshal(raw, &tc) != nil {
					continue
				}
				switch tc.Type {
				case "text":
					var tb TextContent
					if json.Unmarshal(raw, &tb) == nil && tb.Text != "" {
						am.Content = append(am.Content, TextBlock{Text: tb.Text})
					}
				case "image_url":
					var ib ImageContent
					if json.Unmarshal(raw, &ib) == nil {
						block := ImageBlock{MimeType: ib.MimeType, Data: ib.Data}
						if ib.Source != nil {
							block.URL = "data:" + ib.Source.MediaType + ";base64," + ib.Source.Data
						}
						// Fallback: parse image_url.url (OpenAI format)
						if block.MimeType == "" && block.Data == "" && block.URL == "" {
							var wrapper struct {
								ImageURL struct{ URL string } `json:"image_url"`
							}
							if json.Unmarshal(raw, &wrapper) == nil && wrapper.ImageURL.URL != "" {
								block.URL = wrapper.ImageURL.URL
								// Parse data URI
								if strings.HasPrefix(block.URL, "data:") {
									parts := strings.SplitN(block.URL[5:], ";", 2)
									if len(parts) == 2 {
										block.MimeType = parts[0]
										if strings.HasPrefix(parts[1], "base64,") {
											block.Data = parts[1][7:]
										}
									}
								}
							}
						}
						am.Content = append(am.Content, block)
					}
				}
			}
		} else {
			var s string
			if json.Unmarshal(m.Content, &s) == nil && s != "" {
				am.Content = append(am.Content, TextBlock{Text: s})
			}
		}
	}

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

// Message represents a conversation message
type Message struct {
	Role    string          `json:"role"`
	Content json.RawMessage `json:"content,omitempty"`
	ToolCalls []ToolCall    `json:"tool_calls,omitempty"`
	ToolCallID string       `json:"tool_call_id,omitempty"`
	Name       string       `json:"name,omitempty"`
	ReasoningContent string `json:"reasoning_content,omitempty"`
}

// TextContent is a text content block
type TextContent struct {
	Type string `json:"type"`
	Text string `json:"text"`
}

// ImageContent is an image content block for multimodal models.
type ImageContent struct {
	Type     string       `json:"type"`
	MimeType string       `json:"mime_type,omitempty"`
	Data     string       `json:"data,omitempty"`
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
	ID       string     `json:"id"`
	Type     string     `json:"type"`
	Function ToolCallFn `json:"function"`
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
type StreamEvent struct {
	Type       string
	Text       string
	ToolCall   *ToolCall
	ToolName   string
	ToolID     string
	Usage      *Usage
	StopReason string
	ErrorText  string
}

// Tool definition for the LLM
type ToolDef struct {
	Type     string       `json:"type"`
	Function FunctionDef  `json:"function"`
}

type FunctionDef struct {
	Name        string          `json:"name"`
	Description string          `json:"description"`
	Parameters  json.RawMessage `json:"parameters"`
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
	ThinkingBudget int
	MaxRetries     int

	TransformContext func([]Message, string) []Message
	StopCondition    func(messages []Message, lastResponse string) bool

	BeforeToolCall    func(toolName string, toolCallID string, args json.RawMessage) *ToolCallResult
	PrepareToolCall   func(toolName string, args json.RawMessage) json.RawMessage
	FinalizeToolCall  func(toolName string, result string, execErr error) (finalResult string, finalErr error)
	AfterToolCall     func(toolName string, toolCallID string, args json.RawMessage, result string, execErr error) *ToolCallResult
	ToolsExecutionMode string
}

// ToolCallResult allows hooks to override or intercept tool execution.
type ToolCallResult struct {
	Result  string
	IsError bool
}

// Model identifies a model with full metadata.
type Model struct {
	ID       string `json:"id"`
	Name     string `json:"name"`
	Provider string `json:"provider"`
	API      string `json:"api"`
	BaseURL  string `json:"baseUrl"`
	ContextWindow int  `json:"contextWindow"`
	MaxTokens     int  `json:"maxTokens"`
	Reasoning     bool `json:"reasoning"`
	InputTypes []string `json:"input,omitempty"`
	Cost struct {
		Input      float64 `json:"input"`
		Output     float64 `json:"output"`
		CacheRead  float64 `json:"cacheRead"`
		CacheWrite float64 `json:"cacheWrite"`
	} `json:"cost,omitempty"`
	ThinkingLevelMap map[string]interface{} `json:"thinkingLevelMap,omitempty"`
	Headers          map[string]string      `json:"headers,omitempty"`
	Compat           interface{}            `json:"compat,omitempty"`
}

// LLMProvider abstracts streaming chat across providers
type LLMProvider interface {
	StreamChat(model string, messages []Message, tools []ToolDef, systemPrompt string) (<-chan StreamEvent, error)
}
