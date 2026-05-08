package llm

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/anthropics/anthropic-sdk-go"
	"github.com/anthropics/anthropic-sdk-go/option"
	"github.com/anthropics/anthropic-sdk-go/packages/param"
	"github.com/anthropics/anthropic-sdk-go/packages/ssestream"
	"github.com/huichen/xihu/pkg/types"
)

// AnthropicClient uses the official Anthropic Go SDK.
type AnthropicClient struct {
	client anthropic.Client

	StreamOpts   *StreamOptions
	OnPayload    func(payload []byte)
	OnResponse   func(statusCode int, headers map[string][]string)
	IsCopilot    bool
	StealthMode  bool
	ThinkingOpts *ThinkingOptions
	CacheEnabled bool

	// ActiveCtx, if set, is used as the context for streaming HTTP requests.
	ActiveCtx context.Context
}

// SetActiveCtx sets the context used for streaming HTTP requests.
func (c *AnthropicClient) SetActiveCtx(ctx context.Context) {
	c.ActiveCtx = ctx
}

type ThinkingOptions struct {
	Enabled      bool
	BudgetTokens int
	Effort       string
}

type AnthropicStreamOptions struct {
	Thinking     *ThinkingOptions
	CacheEnabled bool
}

func resolveBudget(to *ThinkingOptions) int {
	if to == nil || !to.Enabled {
		return 0
	}
	if to.BudgetTokens > 0 {
		return to.BudgetTokens
	}
	switch to.Effort {
	case "low":
		return 4000
	case "medium":
		return 8000
	case "high":
		return 16000
	case "xhigh":
		return 24000
	case "max":
		return 32000
	}
	return 16000
}

func NewAnthropicClient(baseURL, apiKey string) *AnthropicClient {
	opts := []option.RequestOption{
		option.WithAPIKey(apiKey),
	}
	if baseURL != "" && !strings.Contains(baseURL, "api.anthropic.com") {
		opts = append(opts, option.WithBaseURL(baseURL))
	}
	return &AnthropicClient{
		client: anthropic.NewClient(opts...),
	}
}

func (c *AnthropicClient) StreamChat(model string, messages []types.Message, tools []types.ToolDef, systemPrompt string) (<-chan types.StreamEvent, error) {
	return c.StreamChatWithOptions(model, messages, tools, systemPrompt, nil)
}

func (c *AnthropicClient) StreamChatWithOptions(model string, messages []types.Message, tools []types.ToolDef, systemPrompt string, opts *AnthropicStreamOptions) (<-chan types.StreamEvent, error) {
	antMsgs, system := convertAnthropicMessages(messages, systemPrompt)
	antTools := convertAnthropicTools(tools, c.StealthMode)

	budget := resolveBudget(c.ThinkingOpts)
	if opts != nil && opts.Thinking != nil {
		budget = resolveBudget(opts.Thinking)
	}

	params := anthropic.MessageNewParams{
		Model:     anthropic.Model(model),
		MaxTokens: 8192,
		Messages:  antMsgs,
		System:    []anthropic.TextBlockParam{{Text: system}},
		Tools:     antTools,
	}

	if budget > 0 {
		params.Thinking = anthropic.ThinkingConfigParamUnion{
			OfEnabled: &anthropic.ThinkingConfigEnabledParam{
				BudgetTokens: int64(budget),
			},
		}
	}

	ctx := context.Background()
	if c.ActiveCtx != nil {
		ctx = c.ActiveCtx
	}
	stream := c.client.Messages.NewStreaming(ctx, params)

	events := make(chan types.StreamEvent, 16)
	go c.streamAnthropicSDK(stream, events)
	return events, nil
}

func (c *AnthropicClient) streamAnthropicSDK(stream *ssestream.Stream[anthropic.MessageStreamEventUnion], events chan<- types.StreamEvent) {
	defer close(events)

	currentBlockType := ""
	var toolID, toolName string
	var toolArgs strings.Builder
	textStarted := false
	thinkingStarted := false

	for stream.Next() {
		evt := stream.Current()

		switch evt.Type {
		case "content_block_start":
			currentBlockType = evt.ContentBlock.Type
			switch currentBlockType {
			case "text":
				if !textStarted {
					textStarted = true
					events <- types.StreamEvent{Type: "text_start"}
				}
			case "thinking", "redacted_thinking":
				if !thinkingStarted {
					thinkingStarted = true
					events <- types.StreamEvent{Type: "thinking_start"}
				}
			case "tool_use":
				toolID = evt.ContentBlock.ID
				toolName = evt.ContentBlock.Name
				toolArgs.Reset()
				events <- types.StreamEvent{
					Type:     "toolcall_start",
					ToolName: toolName,
					ToolID:   toolID,
				}
			}

		case "content_block_delta":
			dt := evt.Delta.Type
			switch dt {
			case "text_delta":
				if evt.Delta.Text != "" {
					events <- types.StreamEvent{Type: "text_delta", Text: evt.Delta.Text}
				}
			case "thinking_delta":
				if evt.Delta.Thinking != "" {
					events <- types.StreamEvent{Type: "thinking_delta", Text: evt.Delta.Thinking}
				}
			case "input_json_delta":
				if evt.Delta.PartialJSON != "" {
					toolArgs.WriteString(evt.Delta.PartialJSON)
					events <- types.StreamEvent{Type: "toolcall_delta", Text: evt.Delta.PartialJSON}
				}
			}

		case "content_block_stop":
			switch currentBlockType {
			case "text":
				if textStarted {
					events <- types.StreamEvent{Type: "text_end"}
					textStarted = false
				}
			case "thinking", "redacted_thinking":
				if thinkingStarted {
					events <- types.StreamEvent{Type: "thinking_end"}
					thinkingStarted = false
				}
			case "tool_use":
				args := toolArgs.String()
				if args == "" {
					args = "{}"
				}
				events <- types.StreamEvent{
					Type: "toolcall_end",
					ToolCall: &types.ToolCall{
						ID:   toolID,
						Type: "function",
						Function: types.ToolCallFn{
							Name:      toolName,
							Arguments: json.RawMessage(args),
						},
					},
				}
			}
			currentBlockType = ""

		case "message_delta":
			events <- types.StreamEvent{
				Type: "usage",
				Usage: &types.Usage{
					CompletionTokens: int(evt.Usage.OutputTokens),
				},
			}

		case "message_stop":
			if textStarted {
				events <- types.StreamEvent{Type: "text_end"}
			}
			if thinkingStarted {
				events <- types.StreamEvent{Type: "thinking_end"}
			}
		}
	}

	events <- types.StreamEvent{Type: "stop"}
}

func convertAnthropicMessages(messages []types.Message, systemPrompt string) ([]anthropic.MessageParam, string) {
	var system string
	antMsgs := make([]anthropic.MessageParam, 0, len(messages))

	for _, msg := range messages {
		content := extractMsgText(msg.Content)
		switch msg.Role {
		case "system":
			system = content
		case "user":
			antMsgs = append(antMsgs, anthropic.NewUserMessage(anthropic.NewTextBlock(content)))
		case "assistant":
			if len(msg.ToolCalls) > 0 {
				blocks := make([]anthropic.ContentBlockParamUnion, 0, len(msg.ToolCalls)+1)
				if content != "" {
					blocks = append(blocks, anthropic.NewTextBlock(content))
				}
				for _, tc := range msg.ToolCalls {
					blocks = append(blocks, anthropic.ContentBlockParamUnion{
						OfToolUse: &anthropic.ToolUseBlockParam{
							ID:    tc.ID,
							Name:  tc.Function.Name,
							Input: json.RawMessage(tc.Function.Arguments),
						},
					})
				}
				antMsgs = append(antMsgs, anthropic.MessageParam{
					Role:    anthropic.MessageParamRoleAssistant,
					Content: blocks,
				})
			} else {
				antMsgs = append(antMsgs, anthropic.NewAssistantMessage(anthropic.NewTextBlock(content)))
			}
		case "tool":
			antMsgs = append(antMsgs, anthropic.NewUserMessage(
				anthropic.NewToolResultBlock(msg.ToolCallID, content, false),
			))
		}
	}

	if system == "" && systemPrompt != "" {
		system = systemPrompt
	}
	return antMsgs, system
}

func convertAnthropicTools(tools []types.ToolDef, stealthMode bool) []anthropic.ToolUnionParam {
	result := make([]anthropic.ToolUnionParam, 0, len(tools))
	for _, t := range tools {
		name := t.Function.Name
		desc := t.Function.Description
		if stealthMode {
			if mapped, ok := stealthToolName[name]; ok {
				name = mapped
			}
		}
		var schema anthropic.ToolInputSchemaParam
		if err := json.Unmarshal(t.Function.Parameters, &schema); err != nil {
			// Fallback: wrap parameters in a schema
			var propsMap map[string]interface{}
			json.Unmarshal(t.Function.Parameters, &propsMap)
			schema = anthropic.ToolInputSchemaParam{Properties: propsMap}
		}
		result = append(result, anthropic.ToolUnionParam{
			OfTool: &anthropic.ToolParam{
				Name:        name,
				Description: anthropic.String(desc),
				InputSchema: schema,
			},
		})
	}
	return result
}

var stealthToolName = map[string]string{
	"bash": "Bash", "read": "Read", "write": "Edit",
	"edit": "Edit", "grep": "Grep", "ls": "LS", "find": "Find",
}

var _ = param.Opt[string]{}
var _ = ssestream.Stream[anthropic.MessageStreamEventUnion]{}
var _ = fmt.Sprintf
var _ = strings.Builder{}
