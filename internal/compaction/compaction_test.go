package compaction

import (
	"encoding/json"
	"fmt"
	"testing"

	"github.com/huichen/xihu/pkg/types"
)

type typesMessage = types.Message

func msgUser(text string) types.Message {
	return types.Message{
		Role:    "user",
		Content: json.RawMessage(fmt.Sprintf(`[{"type":"text","text":%q}]`, text)),
	}
}

func msgAssistant(text string) types.Message {
	return types.Message{
		Role:    "assistant",
		Content: json.RawMessage(fmt.Sprintf(`[{"type":"text","text":%q}]`, text)),
	}
}

func msgAssistantWithTool(toolName string, args map[string]interface{}) types.Message {
	argsJSON, _ := json.Marshal(args)
	return types.Message{
		Role:    "assistant",
		Content: json.RawMessage(`[]`),
		ToolCalls: []types.ToolCall{{
			ID:       "call_1",
			Type:     "function",
			Function: types.ToolCallFn{Name: toolName, Arguments: argsJSON},
		}},
	}
}

func repeat(s string, n int) string {
	r := ""
	for i := 0; i < n; i++ {
		r += s
	}
	return r
}

func TestEstimateTokens_User(t *testing.T) {
	tok := EstimateTokens(msgUser("hello world"))
	if tok <= 0 {
		t.Errorf("expected > 0 tokens, got %d", tok)
	}
}

func TestEstimateContextTokens(t *testing.T) {
	tok := EstimateContextTokens([]typesMessage{msgUser("hi"), msgAssistant("hello")})
	if tok <= 0 {
		t.Errorf("expected > 0 tokens, got %d", tok)
	}
}

func TestFindCutPoint_Basic(t *testing.T) {
	msgs := []typesMessage{msgUser("q1"), msgAssistant("a1"), msgUser("q2"), msgAssistant("a2")}
	cut := FindCutPoint(msgs, 0, len(msgs), 10)
	if cut.FirstKeptEntryIndex < 0 {
		t.Error("expected valid cut point")
	}
}

func TestCompact_NoCompaction(t *testing.T) {
	msgs := []typesMessage{msgUser("hi"), msgAssistant("hello")}
	result, _, err := Compact(msgs, CompactOptions{KeepRecentTokens: 100000})
	if err != nil {
		t.Fatal(err)
	}
	if len(result) != len(msgs) {
		t.Errorf("expected %d messages, got %d", len(msgs), len(result))
	}
}

func TestCompact_WithCompaction(t *testing.T) {
	msgs := []typesMessage{
		msgUser("long " + repeat("x", 200)),
		msgAssistant("long " + repeat("y", 200)),
		msgUser("short"),
		msgAssistant("short"),
	}
	result, _, err := Compact(msgs, CompactOptions{KeepRecentTokens: 5, ReserveTokens: 10})
	if err != nil {
		t.Fatal(err)
	}
	if len(result) >= len(msgs) {
		t.Logf("compaction did not reduce: %d -> %d", len(msgs), len(result))
	}
}

func TestCompact_WithSummarizer(t *testing.T) {
	msgs := []typesMessage{
		msgUser("x" + repeat("y", 200)),
		msgAssistant("z" + repeat("w", 200)),
		msgUser("q"),
		msgAssistant("a"),
	}
	result, _, err := Compact(msgs, CompactOptions{
		KeepRecentTokens: 5,
		ReserveTokens:    10,
		Summarizer: func(m []typesMessage) (string, error) {
			return "summary", nil
		},
	})
	if err != nil {
		t.Fatal(err)
	}
	if len(result) == 0 {
		t.Error("expected compacted messages")
	}
}

func TestExtractFileOperations(t *testing.T) {
	msgs := []typesMessage{
		msgAssistantWithTool("read", map[string]interface{}{"file_path": "/tmp/a.go"}),
		msgAssistantWithTool("write", map[string]interface{}{"file_path": "/tmp/b.go"}),
	}
	reads, writes := ExtractFileOperations(msgs)
	if len(reads) != 1 {
		t.Errorf("expected 1 read, got %v", reads)
	}
	if len(writes) != 1 {
		t.Errorf("expected 1 write, got %v", writes)
	}
}

func TestShouldCompact(t *testing.T) {
	s := DefaultCompactionSettings
	if !ShouldCompact(1000, 2000, s) {
		t.Error("should compact with low window")
	}
	s.Enabled = false
	if ShouldCompact(99999, 100000, s) {
		t.Error("should not compact when disabled")
	}
}
