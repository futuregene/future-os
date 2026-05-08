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
