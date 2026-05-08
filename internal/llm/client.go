package llm

import (
	"context"
	"encoding/json"
	"fmt"
	"strings"

	"github.com/huichen/cobalt/pkg/types"
	openai "github.com/openai/openai-go"
	"github.com/openai/openai-go/option"
	"github.com/openai/openai-go/packages/param"
	"github.com/openai/openai-go/packages/ssestream"
	"github.com/openai/openai-go/shared"
)

// Client uses the official OpenAI Go SDK for chat completions.
type Client struct {
	client openai.Client

	ReasoningEffort    string
	ToolChoice         interface{}
	EnableCacheControl bool
	StreamOpts         *StreamOptions
	OnPayload          func(payload []byte)
	OnResponse         func(statusCode int, headers map[string][]string)
	IsCloudflare       bool
	IsCopilot          bool
}

func NewClient(baseURL, apiKey string) *Client {
	opts := []option.RequestOption{
		option.WithAPIKey(apiKey),
	}
	if baseURL != "" && !strings.Contains(baseURL, "api.openai.com") {
		opts = append(opts, option.WithBaseURL(baseURL))
	}

	c := &Client{
		client: openai.NewClient(opts...),
	}
	if strings.Contains(strings.ToLower(baseURL), "cloudflare") || strings.Contains(strings.ToLower(baseURL), "workers.dev") {
		c.IsCloudflare = true
	}
	return c
}

// StreamChat sends a streaming chat completion and returns events via channel.
func (c *Client) StreamChat(model string, messages []types.Message, tools []types.ToolDef, systemPrompt string) (<-chan types.StreamEvent, error) {
	msgs := convertToOpenAIMessages(messages)

	params := openai.ChatCompletionNewParams{
		Model:    shared.ChatModel(model),
		Messages: msgs,
		Tools:    convertToOpenAITools(tools),
	}

	// ToolChoice
	if c.ToolChoice != nil {
		switch v := c.ToolChoice.(type) {
		case string:
			params.ToolChoice = openai.ChatCompletionToolChoiceOptionUnionParam{
				OfAuto: openai.String(v),
			}
		case map[string]interface{}:
			if fn, ok := v["function"].(map[string]interface{}); ok {
				if name, ok := fn["name"].(string); ok {
					params.ToolChoice = openai.ChatCompletionToolChoiceOptionParamOfChatCompletionNamedToolChoice(
						openai.ChatCompletionNamedToolChoiceFunctionParam{Name: name},
					)
				}
			}
		}
	}

	ctx := context.Background()
	stream := c.client.Chat.Completions.NewStreaming(ctx, params)

	events := make(chan types.StreamEvent, 16)
	go c.streamFromSDK(stream, events)
	return events, nil
}

func (c *Client) streamFromSDK(stream *ssestream.Stream[openai.ChatCompletionChunk], events chan<- types.StreamEvent) {
	defer close(events)

	textStarted := false
	var toolCallAccum map[int]*types.ToolCall
	toolCallStarted := make(map[int]bool)

	for stream.Next() {
		chunk := stream.Current()

		// Usage
		if chunk.Usage.TotalTokens > 0 {
			events <- types.StreamEvent{
				Type: "usage",
				Usage: &types.Usage{
					PromptTokens:     int(chunk.Usage.PromptTokens),
					CompletionTokens: int(chunk.Usage.CompletionTokens),
					TotalTokens:      int(chunk.Usage.TotalTokens),
				},
			}
		}

		for _, choice := range chunk.Choices {
			delta := choice.Delta

			// Text content
			if delta.Content != "" {
				if !textStarted {
					textStarted = true
					events <- types.StreamEvent{Type: "text_start"}
				}
				events <- types.StreamEvent{
					Type: "text_delta",
					Text: delta.Content,
				}
			}

			// Tool calls
			for _, tc := range delta.ToolCalls {
				idx := int(tc.Index)
				if tc.ID != "" {
					if toolCallAccum == nil {
						toolCallAccum = make(map[int]*types.ToolCall)
					}
					toolCallAccum[idx] = &types.ToolCall{
						ID:   tc.ID,
						Type: string(tc.Type),
					}
					if !toolCallStarted[idx] {
						toolCallStarted[idx] = true
						events <- types.StreamEvent{
							Type:     "toolcall_start",
							ToolName: tc.Function.Name,
							ToolID:   tc.ID,
						}
					}
				}
				if tc.Function.Name != "" {
					if existing, ok := toolCallAccum[idx]; ok {
						existing.Function.Name = tc.Function.Name
					}
				}
				if tc.Function.Arguments != "" {
					events <- types.StreamEvent{
						Type: "toolcall_delta",
						Text: tc.Function.Arguments,
					}
					if existing, ok := toolCallAccum[idx]; ok {
						existing.Function.Arguments = json.RawMessage(
							string(existing.Function.Arguments) + tc.Function.Arguments,
						)
					}
				}
			}

			// Finish reason
			if choice.FinishReason != "" {
				if textStarted {
					events <- types.StreamEvent{Type: "text_end"}
					textStarted = false
				}
				if choice.FinishReason == "tool_calls" {
					for idx, tc := range toolCallAccum {
						if toolCallStarted[idx] {
							events <- types.StreamEvent{
								Type:     "toolcall_end",
								ToolCall: tc,
							}
						}
					}
				}
			}
		}
	}

	// End any open streams
	if textStarted {
		events <- types.StreamEvent{Type: "text_end"}
	}
	for idx, tc := range toolCallAccum {
		if toolCallStarted[idx] {
			events <- types.StreamEvent{
				Type:     "toolcall_end",
				ToolCall: tc,
			}
		}
	}
	events <- types.StreamEvent{Type: "stop"}
}

func convertToOpenAIMessages(messages []types.Message) []openai.ChatCompletionMessageParamUnion {
	result := make([]openai.ChatCompletionMessageParamUnion, 0, len(messages))
	for _, msg := range messages {
		content := extractMsgText(msg.Content)
		switch msg.Role {
		case "system":
			result = append(result, openai.SystemMessage(content))
		case "user":
			result = append(result, openai.UserMessage(content))
		case "assistant":
			if len(msg.ToolCalls) > 0 {
				tcs := make([]openai.ChatCompletionMessageToolCallParam, len(msg.ToolCalls))
				for j, tc := range msg.ToolCalls {
					tcs[j] = openai.ChatCompletionMessageToolCallParam{
						ID:   tc.ID,
						Type: "function",
						Function: openai.ChatCompletionMessageToolCallFunctionParam{
							Name:      tc.Function.Name,
							Arguments: string(tc.Function.Arguments),
						},
					}
				}
				var emptyContent openai.ChatCompletionAssistantMessageParamContentUnion
				if content != "" {
					emptyContent.OfString = param.NewOpt(content)
				}
				msg := openai.ChatCompletionAssistantMessageParam{
					Content:   emptyContent,
					ToolCalls: tcs,
				}
				result = append(result, openai.ChatCompletionMessageParamUnion{OfAssistant: &msg})
			} else {
				result = append(result, openai.AssistantMessage(content))
			}
		case "tool":
			result = append(result, openai.ToolMessage(content, msg.ToolCallID))
		}
	}
	return result
}

func convertToOpenAITools(tools []types.ToolDef) []openai.ChatCompletionToolParam {
	result := make([]openai.ChatCompletionToolParam, len(tools))
	for i, t := range tools {
		var paramsMap map[string]interface{}
		if err := json.Unmarshal(t.Function.Parameters, &paramsMap); err != nil {
			paramsMap = map[string]interface{}{"type": "object"}
		}
		result[i] = openai.ChatCompletionToolParam{
			Type: "function",
			Function: shared.FunctionDefinitionParam{
				Name:        t.Function.Name,
				Description: param.NewOpt(t.Function.Description),
				Parameters:  shared.FunctionParameters(paramsMap),
			},
		}
	}
	return result
}

func extractMsgText(content json.RawMessage) string {
	var blocks []types.TextContent
	if err := json.Unmarshal(content, &blocks); err == nil {
		if len(blocks) > 0 {
			return blocks[0].Text
		}
		return ""
	}
	var s string
	if err := json.Unmarshal(content, &s); err == nil {
		return s
	}
	return string(content)
}

var _ = context.Background
var _ = fmt.Sprintf
