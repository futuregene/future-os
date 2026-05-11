package types

import (
	"encoding/json"
	"testing"
)

func TestMessageJSON(t *testing.T) {
	tests := []struct {
		name string
		msg  Message
		want string
	}{
		{
			name: "user message",
			msg: Message{
				Role:    "user",
				Content: json.RawMessage(`[{"type":"text","text":"hello"}]`),
			},
			want: `{"role":"user","content":[{"type":"text","text":"hello"}]}`,
		},
		{
			name: "assistant with tool calls",
			msg: Message{
				Role:    "assistant",
				Content: json.RawMessage(`[{"type":"text","text":"ok"}]`),
				ToolCalls: []ToolCall{
					{ID: "call_1", Type: "function", Function: ToolCallFn{Name: "bash", Arguments: json.RawMessage(`{}`)}},
				},
			},
			want: `{"role":"assistant","content":[{"type":"text","text":"ok"}],"tool_calls":[{"id":"call_1","type":"function","function":{"name":"bash","arguments":{}}}]}`,
		},
		{
			name: "tool message",
			msg: Message{
				Role:       "tool",
				Content:    json.RawMessage(`"result"`),
				ToolCallID: "call_1",
			},
			want: `{"role":"tool","content":"result","tool_call_id":"call_1"}`,
		},
		{
			name: "empty message",
			msg:  Message{},
			want: `{"role":""}`,
		},
		{
			name: "with name field",
			msg: Message{
				Role:    "user",
				Content: json.RawMessage(`[{"type":"text","text":"hi"}]`),
				Name:    "alice",
			},
			want: `{"role":"user","content":[{"type":"text","text":"hi"}],"name":"alice"}`,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			data, err := json.Marshal(tt.msg)
			if err != nil {
				t.Fatalf("marshal: %v", err)
			}
			if string(data) != tt.want {
				t.Errorf("marshal = %s, want %s", string(data), tt.want)
			}
		})
	}
}

func TestMessageUnmarshal(t *testing.T) {
	input := `{"role":"user","content":[{"type":"text","text":"hello"}]}`
	var msg Message
	if err := json.Unmarshal([]byte(input), &msg); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if msg.Role != "user" {
		t.Errorf("role = %s, want user", msg.Role)
	}
}

func TestTextContent(t *testing.T) {
	tc := TextContent{Type: "text", Text: "hello"}
	data, err := json.Marshal(tc)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	var out TextContent
	if err := json.Unmarshal(data, &out); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if out.Type != "text" || out.Text != "hello" {
		t.Errorf("roundtrip failed: got %+v", out)
	}
}

func TestToolCallJSON(t *testing.T) {
	tc := ToolCall{
		ID:   "call_1",
		Type: "function",
		Function: ToolCallFn{
			Name:      "bash",
			Arguments: json.RawMessage(`{"command":"ls"}`),
		},
	}
	data, err := json.Marshal(tc)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	var out ToolCall
	if err := json.Unmarshal(data, &out); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if out.ID != "call_1" || out.Function.Name != "bash" {
		t.Errorf("roundtrip failed: got %+v", out)
	}
}

func TestUsageStruct(t *testing.T) {
	u := Usage{
		PromptTokens:     100,
		CompletionTokens: 50,
		TotalTokens:      150,
	}
	data, err := json.Marshal(u)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	if string(data) != `{"prompt_tokens":100,"completion_tokens":50,"total_tokens":150}` {
		t.Errorf("marshal = %s", string(data))
	}
}

func TestStreamEventStruct(t *testing.T) {
	evt := StreamEvent{
		Type:     "text_delta",
		Text:     "hello",
		ToolName: "bash",
		ToolID:   "call_1",
		Usage:    &Usage{PromptTokens: 10, CompletionTokens: 20, TotalTokens: 30},
	}
	if evt.Type != "text_delta" {
		t.Errorf("type = %s", evt.Type)
	}
	if evt.Usage.TotalTokens != 30 {
		t.Errorf("total tokens = %d", evt.Usage.TotalTokens)
	}
}

func TestToolDef(t *testing.T) {
	td := ToolDef{
		Type: "function",
		Function: FunctionDef{
			Name:        "bash",
			Description: "run a command",
			Parameters:  json.RawMessage(`{"type":"object"}`),
		},
	}
	data, err := json.Marshal(td)
	if err != nil {
		t.Fatalf("marshal: %v", err)
	}
	var out ToolDef
	if err := json.Unmarshal(data, &out); err != nil {
		t.Fatalf("unmarshal: %v", err)
	}
	if out.Function.Name != "bash" {
		t.Errorf("name = %s", out.Function.Name)
	}
}

func TestAgentTool(t *testing.T) {
	called := false
	at := AgentTool{
		Def: ToolDef{
			Type: "function",
			Function: FunctionDef{
				Name: "test",
			},
		},
		Handler: func(args json.RawMessage) (string, error) {
			called = true
			return "ok", nil
		},
		Guidelines: []string{"be careful"},
	}
	result, err := at.Handler(json.RawMessage(`{}`))
	if err != nil {
		t.Fatalf("handler: %v", err)
	}
	if result != "ok" {
		t.Errorf("result = %s", result)
	}
	if !called {
		t.Error("handler not called")
	}
	if len(at.Guidelines) != 1 {
		t.Errorf("guidelines = %v", at.Guidelines)
	}
}

func TestAgentConfig(t *testing.T) {
	cfg := AgentConfig{
		SystemPrompt:   "you are helpful",
		MaxTurns:       10,
		ThinkingBudget: 4000,
	}
	if cfg.MaxTurns != 10 {
		t.Errorf("max turns = %d", cfg.MaxTurns)
	}
	if cfg.ThinkingBudget != 4000 {
		t.Errorf("thinking budget = %d", cfg.ThinkingBudget)
	}
}

func TestAgentConfigTransformContext(t *testing.T) {
	called := false
	cfg := AgentConfig{
		TransformContext: func(messages []Message, text string) []Message {
			called = true
			return messages
		},
	}
	if cfg.TransformContext == nil {
		t.Fatal("TransformContext is nil")
	}
	result := cfg.TransformContext(nil, "")
	if called == false {
		t.Error("TransformContext not called properly")
	}
	_ = result
}

func TestAgentConfigStopCondition(t *testing.T) {
	cfg := AgentConfig{
		StopCondition: func(messages []Message, lastResponse string) bool {
			return lastResponse == "STOP"
		},
	}
	if cfg.StopCondition == nil {
		t.Fatal("StopCondition is nil")
	}
	if !cfg.StopCondition(nil, "STOP") {
		t.Error("StopCondition should return true for STOP")
	}
	if cfg.StopCondition(nil, "GO") {
		t.Error("StopCondition should return false for GO")
	}
}

func TestModelStruct(t *testing.T) {
	m := Model{
		ID:       "gpt-4o",
		Provider: "openai",
		API:      "openai-completions",
		BaseURL:  "https://api.openai.com/v1",
	}
	if m.ID != "gpt-4o" {
		t.Errorf("id = %s", m.ID)
	}
	if m.Provider != "openai" {
		t.Errorf("provider = %s", m.Provider)
	}
}

func TestLLMProviderInterface(t *testing.T) {
	// Verify that Client and AnthropicClient satisfy LLMProvider
	var _ LLMProvider = (*mockProvider)(nil)
	_ = &mockProvider{}
}

type mockProvider struct{}

func (m *mockProvider) StreamChat(model string, messages []Message, tools []ToolDef, systemPrompt string) (<-chan StreamEvent, error) {
	ch := make(chan StreamEvent)
	close(ch)
	return ch, nil
}

// ─── ContentBlock Tests ──────────────────────────────────────────────────────

func TestTextBlock_BlockType(t *testing.T) {
	tb := TextBlock{Text: "hello"}
	if tb.BlockType() != "text" {
		t.Errorf("BlockType = %s, want text", tb.BlockType())
	}
}

func TestTextBlock_MarshalJSON(t *testing.T) {
	tb := TextBlock{Text: "hello world"}
	data, err := tb.MarshalJSON()
	if err != nil {
		t.Fatalf("MarshalJSON: %v", err)
	}
	want := `{"text":"hello world","type":"text"}`
	// Use json.Unmarshal to compare structure rather than exact string order
	var got, expected map[string]interface{}
	json.Unmarshal(data, &got)
	json.Unmarshal([]byte(want), &expected)
	if got["type"] != expected["type"] || got["text"] != expected["text"] {
		t.Errorf("MarshalJSON = %s, want type=text, text=hello world", string(data))
	}
}

func TestTextBlock_ContentBlockInterface(t *testing.T) {
	var cb ContentBlock = TextBlock{Text: "test"}
	if cb.BlockType() != "text" {
		t.Errorf("ContentBlock interface BlockType = %s, want text", cb.BlockType())
	}
}

func TestImageBlock_BlockType(t *testing.T) {
	ib := ImageBlock{MimeType: "image/png", Data: "base64data"}
	if ib.BlockType() != "image_url" {
		t.Errorf("BlockType = %s, want image_url", ib.BlockType())
	}
}

func TestImageBlock_MarshalJSON_WithURL(t *testing.T) {
	ib := ImageBlock{URL: "https://example.com/image.png"}
	data, err := ib.MarshalJSON()
	if err != nil {
		t.Fatalf("MarshalJSON: %v", err)
	}
	var got map[string]interface{}
	json.Unmarshal(data, &got)
	if got["type"] != "image_url" {
		t.Errorf("type = %v, want image_url", got["type"])
	}
	inner, ok := got["image_url"].(map[string]interface{})
	if !ok {
		t.Fatalf("image_url is not a map: %T", got["image_url"])
	}
	if inner["url"] != "https://example.com/image.png" {
		t.Errorf("url = %v", inner["url"])
	}
}

func TestImageBlock_MarshalJSON_WithData(t *testing.T) {
	ib := ImageBlock{MimeType: "image/jpeg", Data: "abc123"}
	data, err := ib.MarshalJSON()
	if err != nil {
		t.Fatalf("MarshalJSON: %v", err)
	}
	var got map[string]interface{}
	json.Unmarshal(data, &got)
	inner, ok := got["image_url"].(map[string]interface{})
	if !ok {
		t.Fatalf("image_url is not a map")
	}
	if inner["url"] != "data:image/jpeg;base64,abc123" {
		t.Errorf("url = %v, want data:image/jpeg;base64,abc123", inner["url"])
	}
}

func TestImageBlock_MarshalJSON_URLOverridesData(t *testing.T) {
	// When both URL and Data are set, URL should take precedence
	ib := ImageBlock{MimeType: "image/png", Data: "ignored", URL: "https://example.com/pic.png"}
	data, _ := ib.MarshalJSON()
	var got map[string]interface{}
	json.Unmarshal(data, &got)
	inner := got["image_url"].(map[string]interface{})
	if inner["url"] != "https://example.com/pic.png" {
		t.Errorf("URL should take precedence over data URI: got %v", inner["url"])
	}
}

func TestImageBlock_ContentBlockInterface(t *testing.T) {
	var cb ContentBlock = ImageBlock{MimeType: "image/png", Data: "x"}
	if cb.BlockType() != "image_url" {
		t.Errorf("ContentBlock interface BlockType = %s, want image_url", cb.BlockType())
	}
}

func TestToolResultBlock_BlockType(t *testing.T) {
	trb := ToolResultBlock{ToolCallID: "call_1", Content: "result"}
	if trb.BlockType() != "tool_result" {
		t.Errorf("BlockType = %s, want tool_result", trb.BlockType())
	}
}

func TestToolResultBlock_MarshalJSON_Normal(t *testing.T) {
	trb := ToolResultBlock{ToolCallID: "call_1", Content: "result content"}
	data, err := trb.MarshalJSON()
	if err != nil {
		t.Fatalf("MarshalJSON: %v", err)
	}
	var got map[string]interface{}
	json.Unmarshal(data, &got)
	if got["type"] != "tool_result" {
		t.Errorf("type = %v", got["type"])
	}
	if got["tool_call_id"] != "call_1" {
		t.Errorf("tool_call_id = %v", got["tool_call_id"])
	}
	if got["content"] != "result content" {
		t.Errorf("content = %v", got["content"])
	}
	if _, exists := got["is_error"]; exists {
		t.Error("is_error should not be present when false")
	}
}

func TestToolResultBlock_MarshalJSON_Error(t *testing.T) {
	trb := ToolResultBlock{ToolCallID: "call_2", Content: "error!", IsError: true}
	data, err := trb.MarshalJSON()
	if err != nil {
		t.Fatalf("MarshalJSON: %v", err)
	}
	var got map[string]interface{}
	json.Unmarshal(data, &got)
	if got["is_error"] != true {
		t.Errorf("is_error = %v, want true", got["is_error"])
	}
}

func TestToolResultBlock_ContentBlockInterface(t *testing.T) {
	var cb ContentBlock = ToolResultBlock{ToolCallID: "c1", Content: "ok"}
	if cb.BlockType() != "tool_result" {
		t.Errorf("ContentBlock interface BlockType = %s, want tool_result", cb.BlockType())
	}
}

// ─── AgentMessage Helper Tests ───────────────────────────────────────────────

func TestAgentMessage_Text(t *testing.T) {
	m := AgentMessage{
		Role: "user",
		Content: []ContentBlock{
			TextBlock{Text: "Hello "},
			TextBlock{Text: "World"},
		},
	}
	if m.Text() != "Hello World" {
		t.Errorf("Text = %q, want %q", m.Text(), "Hello World")
	}
}

func TestAgentMessage_Text_NoTextBlocks(t *testing.T) {
	m := AgentMessage{
		Role:    "user",
		Content: []ContentBlock{},
	}
	if m.Text() != "" {
		t.Errorf("Text = %q, want empty", m.Text())
	}
}

func TestAgentMessage_Text_MixedBlocks(t *testing.T) {
	m := AgentMessage{
		Role: "user",
		Content: []ContentBlock{
			TextBlock{Text: "text1"},
			ImageBlock{MimeType: "image/png", Data: "xxx"},
			TextBlock{Text: "text2"},
		},
	}
	if m.Text() != "text1text2" {
		t.Errorf("Text = %q, want %q", m.Text(), "text1text2")
	}
}

func TestAgentMessage_AddText(t *testing.T) {
	m := &AgentMessage{Role: "user"}
	m.AddText("first")
	m.AddText("second")
	if len(m.Content) != 2 {
		t.Fatalf("len(Content) = %d, want 2", len(m.Content))
	}
	tb1, ok := m.Content[0].(TextBlock)
	if !ok || tb1.Text != "first" {
		t.Errorf("Content[0] = %+v, want TextBlock{Text:first}", m.Content[0])
	}
	tb2, ok := m.Content[1].(TextBlock)
	if !ok || tb2.Text != "second" {
		t.Errorf("Content[1] = %+v, want TextBlock{Text:second}", m.Content[1])
	}
}

func TestAgentMessage_AddImage(t *testing.T) {
	m := &AgentMessage{Role: "user"}
	m.AddImage("image/png", "base64stuff")
	if len(m.Content) != 1 {
		t.Fatalf("len(Content) = %d, want 1", len(m.Content))
	}
	ib, ok := m.Content[0].(ImageBlock)
	if !ok {
		t.Fatalf("Content[0] is not ImageBlock: %T", m.Content[0])
	}
	if ib.MimeType != "image/png" || ib.Data != "base64stuff" {
		t.Errorf("ImageBlock = %+v, want MimeType=image/png Data=base64stuff", ib)
	}
}

func TestAgentMessage_MarshalJSON(t *testing.T) {
	m := AgentMessage{
		Role: "user",
		Content: []ContentBlock{
			TextBlock{Text: "hello"},
			ImageBlock{MimeType: "image/png", Data: "abc"},
		},
		Thinking:   "thinking...",
		ToolCallID: "tc_1",
	}
	data, err := json.Marshal(m)
	if err != nil {
		t.Fatalf("Marshal: %v", err)
	}
	var got map[string]interface{}
	json.Unmarshal(data, &got)
	if got["role"] != "user" {
		t.Errorf("role = %v", got["role"])
	}
	if got["thinking"] != "thinking..." {
		t.Errorf("thinking = %v", got["thinking"])
	}
	if got["tool_call_id"] != "tc_1" {
		t.Errorf("tool_call_id = %v", got["tool_call_id"])
	}
}

func TestAgentMessage_MarshalJSON_WithToolCalls(t *testing.T) {
	m := AgentMessage{
		Role:    "assistant",
		Content: []ContentBlock{TextBlock{Text: "ok"}},
		ToolCalls: []AgentToolCall{
			{ID: "call_1", Name: "bash", Args: json.RawMessage(`{"cmd":"ls"}`)},
		},
	}
	data, err := json.Marshal(m)
	if err != nil {
		t.Fatalf("Marshal: %v", err)
	}
	var got map[string]interface{}
	json.Unmarshal(data, &got)
	toolCalls := got["tool_calls"].([]interface{})
	if len(toolCalls) != 1 {
		t.Fatalf("tool_calls len = %d", len(toolCalls))
	}
	tc := toolCalls[0].(map[string]interface{})
	if tc["id"] != "call_1" || tc["name"] != "bash" {
		t.Errorf("tool_call = %+v", tc)
	}
}

// ─── Conversion Tests: ConvertToLLM ──────────────────────────────────────────

func TestConvertToLLM_TextOnly(t *testing.T) {
	ams := []AgentMessage{
		{
			Role:    "user",
			Content: []ContentBlock{TextBlock{Text: "hello"}},
		},
	}
	msgs := ConvertToLLM(ams)
	if len(msgs) != 1 {
		t.Fatalf("len = %d, want 1", len(msgs))
	}
	if msgs[0].Role != "user" {
		t.Errorf("Role = %s", msgs[0].Role)
	}
	// Content should be marshalled JSON array
	if string(msgs[0].Content) != `[{"text":"hello","type":"text"}]` {
		t.Errorf("Content = %s", string(msgs[0].Content))
	}
}

func TestConvertToLLM_WithToolCalls(t *testing.T) {
	ams := []AgentMessage{
		{
			Role:    "assistant",
			Content: []ContentBlock{TextBlock{Text: "result"}},
			ToolCalls: []AgentToolCall{
				{ID: "call_1", Name: "bash", Args: json.RawMessage(`{"command":"ls"}`)},
			},
		},
	}
	msgs := ConvertToLLM(ams)
	if len(msgs[0].ToolCalls) != 1 {
		t.Fatalf("ToolCalls len = %d", len(msgs[0].ToolCalls))
	}
	tc := msgs[0].ToolCalls[0]
	if tc.ID != "call_1" {
		t.Errorf("ToolCall ID = %s", tc.ID)
	}
	if tc.Type != "function" {
		t.Errorf("ToolCall Type = %s", tc.Type)
	}
	if tc.Function.Name != "bash" {
		t.Errorf("Function.Name = %s", tc.Function.Name)
	}
	if string(tc.Function.Arguments) != `{"command":"ls"}` {
		t.Errorf("Function.Arguments = %s", string(tc.Function.Arguments))
	}
}

func TestConvertToLLM_WithThinking(t *testing.T) {
	ams := []AgentMessage{
		{
			Role:     "assistant",
			Thinking: "let me think...",
			Content:  []ContentBlock{TextBlock{Text: "answer"}},
		},
	}
	msgs := ConvertToLLM(ams)
	if msgs[0].ReasoningContent != "let me think..." {
		t.Errorf("ReasoningContent = %q", msgs[0].ReasoningContent)
	}
}

func TestConvertToLLM_WithToolCallID(t *testing.T) {
	ams := []AgentMessage{
		{
			Role:       "tool",
			ToolCallID: "call_xyz",
			Content:    []ContentBlock{TextBlock{Text: "tool output"}},
		},
	}
	msgs := ConvertToLLM(ams)
	if msgs[0].ToolCallID != "call_xyz" {
		t.Errorf("ToolCallID = %s", msgs[0].ToolCallID)
	}
}

func TestConvertToLLM_EmptyContent(t *testing.T) {
	ams := []AgentMessage{
		{Role: "user"},
	}
	msgs := ConvertToLLM(ams)
	if msgs[0].Content != nil {
		t.Errorf("Content should be nil for empty content, got %s", string(msgs[0].Content))
	}
}

func TestConvertToLLM_MultipleMessages(t *testing.T) {
	ams := []AgentMessage{
		{Role: "system", Content: []ContentBlock{TextBlock{Text: "sys"}}},
		{Role: "user", Content: []ContentBlock{TextBlock{Text: "usr"}}},
		{Role: "assistant", Content: []ContentBlock{TextBlock{Text: "ast"}}},
	}
	msgs := ConvertToLLM(ams)
	if len(msgs) != 3 {
		t.Fatalf("len = %d, want 3", len(msgs))
	}
	for i, role := range []string{"system", "user", "assistant"} {
		if msgs[i].Role != role {
			t.Errorf("msgs[%d].Role = %s, want %s", i, msgs[i].Role, role)
		}
	}
}

func TestConvertToLLM_ImageBlock(t *testing.T) {
	ams := []AgentMessage{
		{
			Role: "user",
			Content: []ContentBlock{
				TextBlock{Text: "look at this"},
				ImageBlock{URL: "https://example.com/img.png"},
			},
		},
	}
	msgs := ConvertToLLM(ams)
	// Verify content contains both text and image blocks
	var blocks []map[string]interface{}
	json.Unmarshal(msgs[0].Content, &blocks)
	if len(blocks) != 2 {
		t.Fatalf("blocks len = %d, want 2: %s", len(blocks), string(msgs[0].Content))
	}
	if blocks[0]["type"] != "text" || blocks[0]["text"] != "look at this" {
		t.Errorf("block[0] = %+v", blocks[0])
	}
	if blocks[1]["type"] != "image_url" {
		t.Errorf("block[1].type = %v", blocks[1]["type"])
	}
}

// ─── Conversion Tests: ConvertFromLLM ────────────────────────────────────────

func TestConvertFromLLM_TextContentArray(t *testing.T) {
	msgs := []Message{
		{
			Role:    "user",
			Content: json.RawMessage(`[{"type":"text","text":"hello world"}]`),
		},
	}
	ams := ConvertFromLLM(msgs)
	if len(ams) != 1 {
		t.Fatalf("len = %d, want 1", len(ams))
	}
	if ams[0].Role != "user" {
		t.Errorf("Role = %s", ams[0].Role)
	}
	if ams[0].Text() != "hello world" {
		t.Errorf("Text = %q", ams[0].Text())
	}
}

func TestConvertFromLLM_StringContent(t *testing.T) {
	msgs := []Message{
		{
			Role:    "user",
			Content: json.RawMessage(`"plain string content"`),
		},
	}
	ams := ConvertFromLLM(msgs)
	if len(ams) != 1 {
		t.Fatalf("len = %d, want 1", len(ams))
	}
	if ams[0].Text() != "plain string content" {
		t.Errorf("Text = %q", ams[0].Text())
	}
}

func TestConvertFromLLM_EmptyStringContent(t *testing.T) {
	msgs := []Message{
		{
			Role:    "user",
			Content: json.RawMessage(`""`),
		},
	}
	ams := ConvertFromLLM(msgs)
	if len(ams[0].Content) != 0 {
		t.Errorf("Content should be empty for empty string, got %d blocks", len(ams[0].Content))
	}
}

func TestConvertFromLLM_ImageContent(t *testing.T) {
	msgs := []Message{
		{
			Role: "user",
			Content: json.RawMessage(`[
				{"type":"text","text":"describe this"},
				{"type":"image_url","mime_type":"image/png","data":"abc123"}
			]`),
		},
	}
	ams := ConvertFromLLM(msgs)
	if len(ams[0].Content) != 2 {
		t.Fatalf("Content len = %d, want 2", len(ams[0].Content))
	}
	// First block should be text
	if tb, ok := ams[0].Content[0].(TextBlock); !ok || tb.Text != "describe this" {
		t.Errorf("Content[0] = %+v", ams[0].Content[0])
	}
	// Second block should be image
	ib, ok := ams[0].Content[1].(ImageBlock)
	if !ok {
		t.Fatalf("Content[1] is not ImageBlock: %T", ams[0].Content[1])
	}
	if ib.MimeType != "image/png" || ib.Data != "abc123" {
		t.Errorf("ImageBlock = %+v", ib)
	}
}

func TestConvertFromLLM_ImageContentWithSource(t *testing.T) {
	msgs := []Message{
		{
			Role: "user",
			Content: json.RawMessage(`[
				{"type":"image_url","source":{"type":"base64","media_type":"image/jpeg","data":"b64data"}}
			]`),
		},
	}
	ams := ConvertFromLLM(msgs)
	ib, ok := ams[0].Content[0].(ImageBlock)
	if !ok {
		t.Fatalf("expected ImageBlock, got %T", ams[0].Content[0])
	}
	if ib.URL != "data:image/jpeg;base64,b64data" {
		t.Errorf("URL = %q", ib.URL)
	}
}

func TestConvertFromLLM_WithToolCalls(t *testing.T) {
	msgs := []Message{
		{
			Role: "assistant",
			ToolCalls: []ToolCall{
				{
					ID:   "call_1",
					Type: "function",
					Function: ToolCallFn{
						Name:      "bash",
						Arguments: json.RawMessage(`{"cmd":"ls"}`),
					},
				},
			},
		},
	}
	ams := ConvertFromLLM(msgs)
	if len(ams[0].ToolCalls) != 1 {
		t.Fatalf("ToolCalls len = %d", len(ams[0].ToolCalls))
	}
	tc := ams[0].ToolCalls[0]
	if tc.ID != "call_1" || tc.Name != "bash" {
		t.Errorf("AgentToolCall = %+v", tc)
	}
	if string(tc.Args) != `{"cmd":"ls"}` {
		t.Errorf("Args = %s", string(tc.Args))
	}
}

func TestConvertFromLLM_WithReasoningContent(t *testing.T) {
	msgs := []Message{
		{
			Role:             "assistant",
			ReasoningContent: "step 1: think, step 2: answer",
			Content:          json.RawMessage(`[{"type":"text","text":"answer"}]`),
		},
	}
	ams := ConvertFromLLM(msgs)
	if ams[0].Thinking != "step 1: think, step 2: answer" {
		t.Errorf("Thinking = %q", ams[0].Thinking)
	}
}

func TestConvertFromLLM_WithToolCallID(t *testing.T) {
	msgs := []Message{
		{
			Role:       "tool",
			ToolCallID: "call_xyz",
			Content:    json.RawMessage(`"result"`),
		},
	}
	ams := ConvertFromLLM(msgs)
	if ams[0].ToolCallID != "call_xyz" {
		t.Errorf("ToolCallID = %s", ams[0].ToolCallID)
	}
}

func TestConvertFromLLM_EmptyContent(t *testing.T) {
	msgs := []Message{
		{Role: "user"},
	}
	ams := ConvertFromLLM(msgs)
	if len(ams[0].Content) != 0 {
		t.Errorf("Content len = %d, want 0", len(ams[0].Content))
	}
}

func TestConvertFromLLM_UnknownBlockType(t *testing.T) {
	msgs := []Message{
		{
			Role:    "user",
			Content: json.RawMessage(`[{"type":"unknown_type","data":"something"},{"type":"text","text":"valid"}]`),
		},
	}
	ams := ConvertFromLLM(msgs)
	// Unknown type should be skipped, only text should remain
	if len(ams[0].Content) != 1 {
		t.Fatalf("Content len = %d, want 1 (unknown type skipped)", len(ams[0].Content))
	}
	if tb, ok := ams[0].Content[0].(TextBlock); !ok || tb.Text != "valid" {
		t.Errorf("Content[0] = %+v", ams[0].Content[0])
	}
}

func TestConvertFromLLM_MalformedJSON(t *testing.T) {
	msgs := []Message{
		{
			Role:    "user",
			Content: json.RawMessage(`{invalid json`),
		},
	}
	ams := ConvertFromLLM(msgs)
	// Should not panic, content should be empty
	if len(ams[0].Content) != 0 {
		t.Errorf("Content len = %d, want 0 for malformed JSON", len(ams[0].Content))
	}
}

func TestConvertFromLLM_MultipleMessages(t *testing.T) {
	msgs := []Message{
		{Role: "system", Content: json.RawMessage(`[{"type":"text","text":"sys"}]`)},
		{Role: "user", Content: json.RawMessage(`[{"type":"text","text":"usr"}]`)},
		{Role: "assistant", Content: json.RawMessage(`[{"type":"text","text":"ast"}]`)},
	}
	ams := ConvertFromLLM(msgs)
	if len(ams) != 3 {
		t.Fatalf("len = %d, want 3", len(ams))
	}
	for i, role := range []string{"system", "user", "assistant"} {
		if ams[i].Role != role {
			t.Errorf("ams[%d].Role = %s", i, ams[i].Role)
		}
	}
}

// ─── Round-Trip Conversion Tests ─────────────────────────────────────────────

func TestRoundTrip_TextOnly(t *testing.T) {
	original := AgentMessage{
		Role: "user",
		Content: []ContentBlock{
			TextBlock{Text: "hello world"},
		},
	}
	llm := ConvertToLLM([]AgentMessage{original})
	back := ConvertFromLLM(llm)
	if len(back) != 1 {
		t.Fatalf("round-trip len = %d", len(back))
	}
	if back[0].Role != original.Role {
		t.Errorf("Role: %s != %s", back[0].Role, original.Role)
	}
	if back[0].Text() != original.Text() {
		t.Errorf("Text: %q != %q", back[0].Text(), original.Text())
	}
}

func TestRoundTrip_WithImage(t *testing.T) {
	original := AgentMessage{
		Role: "user",
		Content: []ContentBlock{
			TextBlock{Text: "look at this"},
			ImageBlock{MimeType: "image/png", Data: "base64data"},
		},
	}
	llm := ConvertToLLM([]AgentMessage{original})
	back := ConvertFromLLM(llm)
	if len(back[0].Content) != 2 {
		t.Fatalf("Content len = %d, want 2", len(back[0].Content))
	}
	// Check text block
	if tb, ok := back[0].Content[0].(TextBlock); !ok || tb.Text != "look at this" {
		t.Errorf("TextBlock = %+v", back[0].Content[0])
	}
	// Check image block
	if ib, ok := back[0].Content[1].(ImageBlock); !ok {
		t.Errorf("expected ImageBlock, got %T", back[0].Content[1])
	} else if ib.MimeType != "image/png" || ib.Data != "base64data" {
		t.Errorf("ImageBlock = %+v", ib)
	}
}

func TestRoundTrip_WithToolCalls(t *testing.T) {
	original := AgentMessage{
		Role: "assistant",
		Content: []ContentBlock{
			TextBlock{Text: "I'll run that command"},
		},
		ToolCalls: []AgentToolCall{
			{ID: "call_1", Name: "bash", Args: json.RawMessage(`{"command":"ls","dir":"/tmp"}`)},
		},
	}
	llm := ConvertToLLM([]AgentMessage{original})
	back := ConvertFromLLM(llm)
	if len(back[0].ToolCalls) != 1 {
		t.Fatalf("ToolCalls len = %d", len(back[0].ToolCalls))
	}
	tc := back[0].ToolCalls[0]
	if tc.ID != "call_1" || tc.Name != "bash" {
		t.Errorf("ToolCall ID/Name = %s/%s", tc.ID, tc.Name)
	}
}

func TestRoundTrip_WithThinking(t *testing.T) {
	original := AgentMessage{
		Role:     "assistant",
		Thinking: "I need to think about this carefully...",
		Content:  []ContentBlock{TextBlock{Text: "here is my answer"}},
	}
	llm := ConvertToLLM([]AgentMessage{original})
	back := ConvertFromLLM(llm)
	if back[0].Thinking != original.Thinking {
		t.Errorf("Thinking: %q != %q", back[0].Thinking, original.Thinking)
	}
}

func TestRoundTrip_WithToolCallID(t *testing.T) {
	original := AgentMessage{
		Role:       "tool",
		ToolCallID: "call_xyz_123",
		Content:    []ContentBlock{TextBlock{Text: "tool result"}},
	}
	llm := ConvertToLLM([]AgentMessage{original})
	back := ConvertFromLLM(llm)
	if back[0].ToolCallID != "call_xyz_123" {
		t.Errorf("ToolCallID: %q != %q", back[0].ToolCallID, original.ToolCallID)
	}
}

// ─── JSON Schema Generation Tests ────────────────────────────────────────────

// Test structs for SchemaOf
type testSchemaBasic struct {
	Name   string  `json:"name"`
	Age    int     `json:"age"`
	Score  float64 `json:"score"`
	Active bool    `json:"active"`
}

type testSchemaWithTags struct {
	Command string `json:"command" jsonschema:"required,description=The command to run"`
	Timeout int    `json:"timeout,omitempty" jsonschema:"description=Timeout in seconds"`
}

type testSchemaNested struct {
	Title  string          `json:"title"`
	Config testSchemaBasic `json:"config"`
}

type testSchemaWithSlice struct {
	Names []string `json:"names"`
}

type testSchemaWithMap struct {
	Attributes map[string]string `json:"attributes"`
}

type testSchemaWithPointer struct {
	Name *string `json:"name"`
}

type testSchemaUnexported struct {
	Name   string `json:"name"`
	secret string // unexported, should be skipped
}

type testSchemaWithJsonDash struct {
	Name    string `json:"name"`
	Ignored string `json:"-"`
}

func TestSchemaOf_String(t *testing.T) {
	type S struct {
		Val string `json:"val"`
	}
	schema := SchemaOf[S]()
	var m map[string]interface{}
	if err := json.Unmarshal(schema, &m); err != nil {
		t.Fatalf("unmarshal schema: %v", err)
	}
	props := m["properties"].(map[string]interface{})
	valSchema := props["val"].(map[string]interface{})
	if valSchema["type"] != "string" {
		t.Errorf("type = %v, want string", valSchema["type"])
	}
	reqs := m["required"].([]interface{})
	if reqs[0] != "val" {
		t.Errorf("required[0] = %v, want val", reqs[0])
	}
}

func TestSchemaOf_Int(t *testing.T) {
	type S struct {
		Val int `json:"val"`
	}
	schema := SchemaOf[S]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	props := m["properties"].(map[string]interface{})
	if props["val"].(map[string]interface{})["type"] != "integer" {
		t.Errorf("int should map to integer")
	}
}

func TestSchemaOf_IntVariants(t *testing.T) {
	type S struct {
		V1 int8  `json:"v1"`
		V2 int16 `json:"v2"`
		V3 int32 `json:"v3"`
		V4 int64 `json:"v4"`
	}
	schema := SchemaOf[S]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	props := m["properties"].(map[string]interface{})
	for _, key := range []string{"v1", "v2", "v3", "v4"} {
		if props[key].(map[string]interface{})["type"] != "integer" {
			t.Errorf("%s type should be integer", key)
		}
	}
}

func TestSchemaOf_UintVariants(t *testing.T) {
	type S struct {
		V1 uint   `json:"v1"`
		V2 uint8  `json:"v2"`
		V3 uint16 `json:"v3"`
		V4 uint32 `json:"v4"`
		V5 uint64 `json:"v5"`
	}
	schema := SchemaOf[S]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	props := m["properties"].(map[string]interface{})
	for _, key := range []string{"v1", "v2", "v3", "v4", "v5"} {
		if props[key].(map[string]interface{})["type"] != "integer" {
			t.Errorf("%s type should be integer", key)
		}
	}
}

func TestSchemaOf_Float(t *testing.T) {
	type S struct {
		V1 float32 `json:"v1"`
		V2 float64 `json:"v2"`
	}
	schema := SchemaOf[S]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	props := m["properties"].(map[string]interface{})
	for _, key := range []string{"v1", "v2"} {
		if props[key].(map[string]interface{})["type"] != "number" {
			t.Errorf("%s type should be number", key)
		}
	}
}

func TestSchemaOf_Bool(t *testing.T) {
	type S struct {
		Val bool `json:"val"`
	}
	schema := SchemaOf[S]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	props := m["properties"].(map[string]interface{})
	if props["val"].(map[string]interface{})["type"] != "boolean" {
		t.Errorf("bool should map to boolean")
	}
}

func TestSchemaOf_Slice(t *testing.T) {
	schema := SchemaOf[testSchemaWithSlice]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	props := m["properties"].(map[string]interface{})
	namesSchema := props["names"].(map[string]interface{})
	if namesSchema["type"] != "array" {
		t.Errorf("type = %v, want array", namesSchema["type"])
	}
	items := namesSchema["items"].(map[string]interface{})
	if items["type"] != "string" {
		t.Errorf("items.type = %v, want string", items["type"])
	}
}

func TestSchemaOf_ByteSlice(t *testing.T) {
	type S struct {
		Data []byte `json:"data"`
	}
	schema := SchemaOf[S]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	props := m["properties"].(map[string]interface{})
	if props["data"].(map[string]interface{})["type"] != "string" {
		t.Errorf("[]byte should map to string type")
	}
}

func TestSchemaOf_Map(t *testing.T) {
	schema := SchemaOf[testSchemaWithMap]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	props := m["properties"].(map[string]interface{})
	attrSchema := props["attributes"].(map[string]interface{})
	if attrSchema["type"] != "object" {
		t.Errorf("type = %v, want object", attrSchema["type"])
	}
	ap := attrSchema["additionalProperties"].(map[string]interface{})
	if ap["type"] != "string" {
		t.Errorf("additionalProperties.type = %v, want string", ap["type"])
	}
}

func TestSchemaOf_Struct(t *testing.T) {
	schema := SchemaOf[testSchemaBasic]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	if m["type"] != "object" {
		t.Errorf("top-level type = %v, want object", m["type"])
	}
	props := m["properties"].(map[string]interface{})
	expected := []string{"name", "age", "score", "active"}
	for _, key := range expected {
		if _, ok := props[key]; !ok {
			t.Errorf("missing property: %s", key)
		}
	}
	reqs := m["required"].([]interface{})
	if len(reqs) != 4 {
		t.Errorf("required len = %d, want 4", len(reqs))
	}
}

func TestSchemaOf_StructWithDescription(t *testing.T) {
	schema := SchemaOf[testSchemaWithTags]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	props := m["properties"].(map[string]interface{})
	cmdSchema := props["command"].(map[string]interface{})
	if cmdSchema["description"] != "The command to run" {
		t.Errorf("command description = %v", cmdSchema["description"])
	}
	timeoutSchema := props["timeout"].(map[string]interface{})
	if timeoutSchema["description"] != "Timeout in seconds" {
		t.Errorf("timeout description = %v", timeoutSchema["description"])
	}
}

func TestSchemaOf_StructWithRequired(t *testing.T) {
	schema := SchemaOf[testSchemaWithTags]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	reqs := m["required"].([]interface{})
	// "command" has required tag, "timeout" has omitempty
	hasCommand := false
	hasTimeout := false
	for _, r := range reqs {
		if r == "command" {
			hasCommand = true
		}
		if r == "timeout" {
			hasTimeout = true
		}
	}
	if !hasCommand {
		t.Error("command should be in required (has jsonschema:\"required\")")
	}
	if hasTimeout {
		t.Error("timeout should NOT be in required (has omitempty, no required tag)")
	}
}

func TestSchemaOf_NestedStruct(t *testing.T) {
	schema := SchemaOf[testSchemaNested]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	props := m["properties"].(map[string]interface{})
	configSchema := props["config"].(map[string]interface{})
	if configSchema["type"] != "object" {
		t.Errorf("nested config type = %v, want object", configSchema["type"])
	}
	configProps := configSchema["properties"].(map[string]interface{})
	if _, ok := configProps["name"]; !ok {
		t.Error("nested config missing 'name' property")
	}
}

func TestSchemaOf_PointerType(t *testing.T) {
	schema := SchemaOf[testSchemaWithPointer]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	props := m["properties"].(map[string]interface{})
	nameSchema := props["name"].(map[string]interface{})
	// Pointer to string should be dereferenced to string type
	if nameSchema["type"] != "string" {
		t.Errorf("pointer to string type = %v, want string", nameSchema["type"])
	}
}

func TestSchemaOf_UnexportedFields(t *testing.T) {
	schema := SchemaOf[testSchemaUnexported]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	props := m["properties"].(map[string]interface{})
	if _, ok := props["secret"]; ok {
		t.Error("unexported field 'secret' should not appear in schema")
	}
	if _, ok := props["name"]; !ok {
		t.Error("exported field 'name' should appear in schema")
	}
}

func TestSchemaOf_JsonDash(t *testing.T) {
	schema := SchemaOf[testSchemaWithJsonDash]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	props := m["properties"].(map[string]interface{})
	if _, ok := props["Ignored"]; ok {
		t.Error("field with json:\"-\" should not appear in schema")
	}
	if _, ok := props["name"]; !ok {
		t.Error("field 'name' should appear in schema")
	}
}

func TestSchemaOf_BasicTypes(t *testing.T) {
	// Test primitive types directly
	type S struct {
		Str   string  `json:"str"`
		Int   int     `json:"int"`
		Float float64 `json:"float"`
		Bool  bool    `json:"bool"`
	}
	schema := SchemaOf[S]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	props := m["properties"].(map[string]interface{})
	typeMap := map[string]string{"str": "string", "int": "integer", "float": "number", "bool": "boolean"}
	for key, expectedType := range typeMap {
		actualType := props[key].(map[string]interface{})["type"].(string)
		if actualType != expectedType {
			t.Errorf("%s type = %s, want %s", key, actualType, expectedType)
		}
	}
}

func TestSchemaOf_EmptyStruct(t *testing.T) {
	type Empty struct{}
	schema := SchemaOf[Empty]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	if m["type"] != "object" {
		t.Errorf("type = %v, want object", m["type"])
	}
	props := m["properties"].(map[string]interface{})
	if len(props) != 0 {
		t.Errorf("properties len = %d, want 0", len(props))
	}
	// No required fields
	if _, ok := m["required"]; ok {
		t.Error("empty struct should have no required field")
	}
}

func TestMustSchemaOf(t *testing.T) {
	type S struct {
		Val string `json:"val"`
	}
	s1 := MustSchemaOf[S]()
	s2 := SchemaOf[S]()
	if string(s1) != string(s2) {
		t.Errorf("MustSchemaOf differs from SchemaOf")
	}
}

func TestSchemaOfExample(t *testing.T) {
	schema := SchemaOfExample()
	var m map[string]interface{}
	if err := json.Unmarshal(schema, &m); err != nil {
		t.Fatalf("SchemaOfExample produced invalid JSON: %v", err)
	}
	props := m["properties"].(map[string]interface{})
	if _, ok := props["command"]; !ok {
		t.Error("SchemaOfExample missing 'command' property")
	}
	if _, ok := props["timeout"]; !ok {
		t.Error("SchemaOfExample missing 'timeout' property")
	}
}

func TestFormatSchema_Valid(t *testing.T) {
	type S struct {
		Name string `json:"name" jsonschema:"description=The name"`
	}
	schema := SchemaOf[S]()
	formatted := FormatSchema(schema)
	if formatted == "" {
		t.Error("FormatSchema returned empty string")
	}
	// Should contain indentation
	if len(formatted) <= len(string(schema)) {
		t.Error("FormatSchema should produce indented output")
	}
}

func TestFormatSchema_Invalid(t *testing.T) {
	result := FormatSchema(json.RawMessage(`{invalid`))
	if result == "" {
		t.Error("FormatSchema should return non-empty string for invalid input")
	}
	// Should contain error indicator
	if result[:1] != "<" {
		t.Errorf("FormatSchema should return error for invalid input, got: %s", result)
	}
}

func TestBuildSchema_UnknownKind(t *testing.T) {
	// Channels and other kinds should default to string
	schema := SchemaOf[chan int]()
	var m map[string]interface{}
	json.Unmarshal(schema, &m)
	if m["type"] != "string" {
		t.Errorf("unknown kind should default to string type, got %v", m["type"])
	}
}
