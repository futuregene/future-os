package llm

import (
	"context"
	"encoding/json"
	"fmt"
	"strconv"
	"strings"

	"github.com/huichen/xihu/pkg/types"
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
	ThinkingBudget     int // 0 = off/disabled, >0 = token budget (legacy fallback)
	StreamOpts         *StreamOptions
	OnPayload          func(payload []byte)
	OnResponse         func(statusCode int, headers map[string][]string)
	IsCloudflare       bool
	IsCopilot          bool

	// Thinking context — populated from ModelInfo by engine.
	// When set, these drive API parameter formatting per pi's compat model.
	ThinkingLevel                        string
	ThinkingLevelMap                     map[string]interface{}
	CompatThinkingFormat                 string // "openai" | "openrouter" | "deepseek" | "zai" | "qwen" | "qwen-chat-template"
	CompatSupportsReasoningEffort        bool
	CompatRequiresReasoningOnAssistant   bool

	// ActiveCtx, if set, is used as the context for streaming HTTP requests.
	// When cancelled, the HTTP request is aborted. Set by agent loop for interrupt support.
	ActiveCtx context.Context
}

// SetActiveCtx sets the context used for streaming HTTP requests.
func (c *Client) SetActiveCtx(ctx context.Context) {
	c.ActiveCtx = ctx
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
	// Prepend system prompt to messages
	if systemPrompt != "" {
		sysContent, _ := json.Marshal([]types.TextContent{{Type: "text", Text: systemPrompt}})
		sysMsg := types.Message{Role: "system", Content: sysContent}
		messages = append([]types.Message{sysMsg}, messages...)
	}
	// Pass requiresReasoningOnAssistant flag for DeepSeek-style compat
	msgs := convertToOpenAIMessages(messages, c.CompatRequiresReasoningOnAssistant && c.ThinkingLevel != "off")

	params := openai.ChatCompletionNewParams{
		Model:    shared.ChatModel(model),
		Messages: msgs,
		Tools:    convertToOpenAITools(tools),
	}

	// Thinking / reasoning control — aligned with pi's openai-completions.ts
	// Uses compat.thinkingFormat (from model metadata) to decide parameter format.
	// Falls back to legacy budget_tokens approach when no compat is set.
	if c.ThinkingLevel != "" && c.CompatThinkingFormat != "" {
		reasoningEnabled := c.ThinkingLevel != "off"
		levelValue := c.ThinkingLevel
		// Apply thinkingLevelMap mapping: e.g. deepseek "high" → "high", "xhigh" → "max"
		if mapped, ok := c.ThinkingLevelMap[c.ThinkingLevel]; ok && mapped != nil {
			levelValue = fmt.Sprint(mapped)
		}

		switch c.CompatThinkingFormat {
		case "zai":
			params.SetExtraFields(map[string]any{"enable_thinking": reasoningEnabled})
		case "qwen":
			params.SetExtraFields(map[string]any{"enable_thinking": reasoningEnabled})
		case "qwen-chat-template":
			params.SetExtraFields(map[string]any{
				"chat_template_kwargs": map[string]any{
					"enable_thinking":  reasoningEnabled,
					"preserve_thinking": true,
				},
			})
		case "deepseek":
			thinkingType := "disabled"
			if reasoningEnabled {
				thinkingType = "enabled"
			}
			extra := map[string]any{
				"thinking": map[string]any{"type": thinkingType},
			}
			if reasoningEnabled {
				extra["reasoning_effort"] = levelValue
			}
			params.SetExtraFields(extra)
		case "openrouter":
			if reasoningEnabled {
				params.SetExtraFields(map[string]any{
					"reasoning": map[string]any{"effort": levelValue},
				})
			} else {
				offVal := c.thinkingLevelMapOffValue()
				if offVal != "" {
					params.SetExtraFields(map[string]any{
						"reasoning": map[string]any{"effort": offVal},
					})
				}
			}
		default: // "openai" or unknown
			if reasoningEnabled {
				if c.CompatSupportsReasoningEffort {
					params.SetExtraFields(map[string]any{"reasoning_effort": levelValue})
				}
			} else if c.CompatSupportsReasoningEffort {
				offVal := c.thinkingLevelMapOffValue()
				if offVal != "" {
					params.SetExtraFields(map[string]any{"reasoning_effort": offVal})
				}
			}
		}
	} else if c.ThinkingBudget > 0 {
		// Legacy fallback: enable thinking with budget for models without compat metadata
		params.SetExtraFields(map[string]any{
			"thinking": map[string]any{
				"type":          "enabled",
				"budget_tokens": c.ThinkingBudget,
			},
		})
	} else if c.ThinkingBudget == 0 && c.ReasoningEffort == "" && c.ThinkingLevel == "" {
		// Legacy fallback: explicitly disable thinking
		params.SetExtraFields(map[string]any{
			"thinking": map[string]any{
				"type": "disabled",
			},
		})
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
	if c.ActiveCtx != nil {
		ctx = c.ActiveCtx
	}
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
	thinkingStarted := false
	var reasoningContent string

	for stream.Next() {
		chunk := stream.Current()

		// Usage
		if chunk.Usage.TotalTokens > 0 {
			usage := &types.Usage{
				PromptTokens:     int(chunk.Usage.PromptTokens),
				CompletionTokens: int(chunk.Usage.CompletionTokens),
				TotalTokens:      int(chunk.Usage.TotalTokens),
			}
			// Cache read tokens: prefer OpenAI standard field (PromptTokensDetails.CachedTokens),
			// fallback to Anthropic-style ExtraField (cache_read_input_tokens)
			if chunk.Usage.PromptTokensDetails.CachedTokens > 0 {
				usage.CacheReadTokens = int(chunk.Usage.PromptTokensDetails.CachedTokens)
			} else if cr, ok := chunk.Usage.JSON.ExtraFields["cache_read_input_tokens"]; ok {
				if v, err := strconv.Atoi(fmt.Sprint(cr.Raw())); err == nil {
					usage.CacheReadTokens = v
				}
			}
			// Cache write tokens: try OpenAI-style ExtraField first (cache_write_tokens),
			// then Anthropic-style (cache_creation_input_tokens)
			if cw, ok := chunk.Usage.PromptTokensDetails.JSON.ExtraFields["cache_write_tokens"]; ok {
				if v, err := strconv.Atoi(fmt.Sprint(cw.Raw())); err == nil {
					usage.CacheWriteTokens = v
				}
			} else if cw, ok := chunk.Usage.JSON.ExtraFields["cache_creation_input_tokens"]; ok {
				if v, err := strconv.Atoi(fmt.Sprint(cw.Raw())); err == nil {
					usage.CacheWriteTokens = v
				}
			}
			events <- types.StreamEvent{
				Type:  "usage",
				Usage: usage,
			}
		}

		for _, choice := range chunk.Choices {
			delta := choice.Delta

			// Reasoning content (from ExtraFields for non-OpenAI providers like DeepSeek)
			if reasoningField, ok := delta.JSON.ExtraFields["reasoning_content"]; ok {
				// Raw() returns JSON-encoded value; skip JSON null
				raw := reasoningField.Raw()
				if raw == "null" {
					// skip — field present but value is JSON null
				} else {
					// Try JSON unquote (handles \n, \", \\, etc.)
					if unquoted, err := strconv.Unquote(raw); err == nil {
						raw = unquoted
					} else if len(raw) >= 2 && raw[0] == '"' && raw[len(raw)-1] == '"' {
						raw = raw[1 : len(raw)-1]
					}
					reasoningContent += raw
					if !thinkingStarted {
						thinkingStarted = true
						events <- types.StreamEvent{Type: "thinking_start"}
					}
					events <- types.StreamEvent{
						Type: "thinking_delta",
						Text: raw,
					}
				}
			}

			// Text content
			if delta.Content != "" {
				if thinkingStarted {
					thinkingStarted = false
					events <- types.StreamEvent{Type: "thinking_end"}
				}
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
			if len(delta.ToolCalls) > 0 && thinkingStarted {
				thinkingStarted = false
				events <- types.StreamEvent{Type: "thinking_end"}
			}
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
							toolCallStarted[idx] = false // prevent double-emit at end-of-stream
						}
					}
				}
			}
		}
	}

	// End any open thinking stream
	if thinkingStarted {
		events <- types.StreamEvent{
			Type: "thinking_end",
			Text: reasoningContent,
		}
	}
	// End any open streams
	if thinkingStarted {
		events <- types.StreamEvent{Type: "thinking_end"}
	}
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
	// Surface stream errors
	if err := stream.Err(); err != nil {
		events <- types.StreamEvent{
			Type:  "error",
			Text:  err.Error(),
		}
	}
	events <- types.StreamEvent{Type: "stop"}
}

func convertToOpenAIMessages(messages []types.Message, needsEmptyReasoningOnAssistant bool) []openai.ChatCompletionMessageParamUnion {
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
				ap := openai.ChatCompletionAssistantMessageParam{
					Content:   openai.ChatCompletionAssistantMessageParamContentUnion{OfString: param.NewOpt(content)},
					ToolCalls: tcs,
				}
				// Inject reasoning_content for providers that require it echoed back
				if msg.ReasoningContent != "" {
					ap.SetExtraFields(map[string]any{"reasoning_content": msg.ReasoningContent})
				} else if needsEmptyReasoningOnAssistant {
					ap.SetExtraFields(map[string]any{"reasoning_content": ""})
				}
				result = append(result, openai.ChatCompletionMessageParamUnion{OfAssistant: &ap})
			} else {
				amsg := openai.AssistantMessage(content)
				// For DeepSeek compat: inject empty reasoning_content on replayed messages
				if needsEmptyReasoningOnAssistant {
					result = append(result, amsg)
					last := result[len(result)-1]
					if last.OfAssistant != nil {
						last.OfAssistant.SetExtraFields(map[string]any{"reasoning_content": ""})
					}
				} else {
					result = append(result, amsg)
				}
			}
			// Inject reasoning_content into the last assistant message if present
			if msg.ReasoningContent != "" && len(result) > 0 {
				last := result[len(result)-1]
				if last.OfAssistant != nil {
					last.OfAssistant.SetExtraFields(map[string]any{"reasoning_content": msg.ReasoningContent})
				}
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

// thinkingLevelMapOffValue returns the off-level mapping value from ThinkingLevelMap.
// Returns empty string if not found or not a string (null values are skipped).
func (c *Client) thinkingLevelMapOffValue() string {
	if c.ThinkingLevelMap == nil {
		return ""
	}
	val, ok := c.ThinkingLevelMap["off"]
	if !ok || val == nil {
		return ""
	}
	s, ok := val.(string)
	if !ok {
		return ""
	}
	return s
}

var _ = context.Background
var _ = fmt.Sprintf
