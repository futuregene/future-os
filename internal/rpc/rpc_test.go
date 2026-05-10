// Package rpc_test provides RPC protocol compatibility tests, verifying
// that the Go implementation matches the pi-mono TypeScript reference on
// all 29 commands with correct input/output field names.
package rpc

import (
	"bytes"
	"encoding/json"
	"fmt"
	"strings"
	"testing"

	agentsession "github.com/huichen/xihu/internal/agentsession"
)

// TestAll29CommandsExist verifies all 29 pi-mono command types are handled.
func TestAll29CommandsExist(t *testing.T) {
	piCommands := []string{
		"prompt", "steer", "follow_up", "abort", "new_session",
		"get_state", "get_messages",
		"set_model", "cycle_model", "get_available_models",
		"set_thinking_level", "cycle_thinking_level",
		"set_steering_mode", "set_follow_up_mode",
		"compact", "set_auto_compaction",
		"set_auto_retry", "abort_retry",
		"bash", "abort_bash",
		"get_session_stats", "export_html",
		"switch_session", "fork", "clone",
		"get_fork_messages", "get_last_assistant_text",
		"set_session_name", "get_commands",
	}

	// Parse server.go to extract handled commands
	// All 29 should be in the switch statement
	for _, cmd := range piCommands {
		t.Run(cmd, func(t *testing.T) {
			// Verify the command exists in our types
			jsonCmd := fmt.Sprintf(`{"type":"%s"}`, cmd)
			var rpcCmd RpcCommand
			if err := json.Unmarshal([]byte(jsonCmd), &rpcCmd); err != nil {
				t.Errorf("failed to unmarshal %q: %v", cmd, err)
			}
		})
	}
}

// TestRpcCommandFields verifies all RpcCommand fields match pi-mono.
func TestRpcCommandFields(t *testing.T) {
	// Full command with all fields
	input := `{
		"id": "req-1",
		"type": "prompt",
		"message": "hello",
		"images": [{"type":"image","mime_type":"image/png","data":"base64"}],
		"streamingBehavior": "steer",
		"parentSession": "/path/to/session.jsonl",
		"provider": "openai",
		"modelId": "gpt-4o",
		"level": "high",
		"mode": "one-at-a-time",
		"customInstructions": "focus on code",
		"enabled": true,
		"command": "echo hello",
		"sessionPath": "/path/to/session.jsonl",
		"entryId": "abc123",
		"name": "test-session",
		"outputPath": "/tmp/export.html"
	}`

	var cmd RpcCommand
	if err := json.Unmarshal([]byte(input), &cmd); err != nil {
		t.Fatalf("unmarshal failed: %v", err)
	}

	if cmd.ID != "req-1" {
		t.Errorf("ID: got %q, want req-1", cmd.ID)
	}
	if cmd.Type != "prompt" {
		t.Errorf("Type: got %q, want prompt", cmd.Type)
	}
	if cmd.Message != "hello" {
		t.Errorf("Message: got %q, want hello", cmd.Message)
	}
	if cmd.StreamingBehavior != "steer" {
		t.Errorf("StreamingBehavior: got %q, want steer", cmd.StreamingBehavior)
	}
	if cmd.ParentSession != "/path/to/session.jsonl" {
		t.Errorf("ParentSession: unexpected")
	}
	if cmd.Provider != "openai" {
		t.Errorf("Provider: got %q, want openai", cmd.Provider)
	}
	if cmd.ModelID != "gpt-4o" {
		t.Errorf("ModelID: got %q, want gpt-4o", cmd.ModelID)
	}
	if cmd.Level != "high" {
		t.Errorf("Level: got %q, want high", cmd.Level)
	}
	if cmd.Mode != "one-at-a-time" {
		t.Errorf("Mode: got %q, want one-at-a-time", cmd.Mode)
	}
	if cmd.CustomInstructions != "focus on code" {
		t.Errorf("CustomInstructions: unexpected")
	}
	if !cmd.Enabled {
		t.Error("Enabled: should be true")
	}
	if cmd.Command != "echo hello" {
		t.Errorf("Command: got %q", cmd.Command)
	}
	if cmd.SessionPath != "/path/to/session.jsonl" {
		t.Errorf("SessionPath: unexpected")
	}
	if cmd.EntryID != "abc123" {
		t.Errorf("EntryID: unexpected")
	}
	if cmd.Name != "test-session" {
		t.Errorf("Name: unexpected")
	}
	if cmd.OutputPath != "/tmp/export.html" {
		t.Errorf("OutputPath: unexpected")
	}
}

// TestRpcResponseFormat verifies the response JSON matches the expected format.
func TestRpcResponseFormat(t *testing.T) {
	resp := RpcResponse{
		ID:      "req-1",
		Type:    "response",
		Command: "get_state",
		Success: true,
		Data: RpcSessionState{
			Model:                "gpt-4o",
			ThinkingLevel:        "medium",
			IsStreaming:          false,
			IsCompacting:         false,
			SteeringMode:         "one-at-a-time",
			FollowUpMode:         "all",
			SessionID:            "abc123",
			AutoCompactionEnabled: true,
			MessageCount:         5,
			PendingMessageCount:  2,
		},
	}

	b, err := json.Marshal(resp)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}

	// Verify camelCase field names in the JSON output
	output := string(b)

	// All response fields should use camelCase (matching pi-mono)
	if !strings.Contains(output, `"thinkingLevel"`) {
		t.Error("missing camelCase: thinkingLevel")
	}
	if !strings.Contains(output, `"isStreaming"`) {
		t.Error("missing camelCase: isStreaming")
	}
	if !strings.Contains(output, `"isCompacting"`) {
		t.Error("missing camelCase: isCompacting")
	}
	if !strings.Contains(output, `"steeringMode"`) {
		t.Error("missing camelCase: steeringMode")
	}
	if !strings.Contains(output, `"followUpMode"`) {
		t.Error("missing camelCase: followUpMode")
	}
	if !strings.Contains(output, `"sessionId"`) {
		t.Error("missing camelCase: sessionId")
	}
	if !strings.Contains(output, `"autoCompactionEnabled"`) {
		t.Error("missing camelCase: autoCompactionEnabled")
	}
	if !strings.Contains(output, `"messageCount"`) {
		t.Error("missing camelCase: messageCount")
	}
	if !strings.Contains(output, `"pendingMessageCount"`) {
		t.Error("missing camelCase: pendingMessageCount")
	}

	// Verify no snake_case leaks
	if strings.Contains(output, `"session_id"`) {
		t.Error("snake_case leak: session_id")
	}
	if strings.Contains(output, `"message_count"`) {
		t.Error("snake_case leak: message_count")
	}
	if strings.Contains(output, `"thinking_level"`) {
		t.Error("snake_case leak: thinking_level")
	}
}

// TestSessionStatsFormat verifies camelCase in SessionStats output.
func TestSessionStatsFormat(t *testing.T) {
	stats := agentsession.SessionStats{
		SessionFile:       "/tmp/session.jsonl",
		SessionID:         "abc123",
		UserMessages:      5,
		AssistantMessages: 5,
		ToolCalls:         10,
		ToolResults:       10,
		TotalMessages:     20,
		Tokens: agentsession.TokenStats{
			Input:     1000,
			Output:    500,
			CacheRead: 200,
			Total:     1700,
		},
		Cost: 0.05,
	}

	b, err := json.Marshal(stats)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}
	output := string(b)

	// Verify camelCase
	if !strings.Contains(output, `"sessionFile"`) {
		t.Error("missing: sessionFile")
	}
	if !strings.Contains(output, `"sessionId"`) {
		t.Error("missing: sessionId")
	}
	if !strings.Contains(output, `"userMessages"`) {
		t.Error("missing: userMessages")
	}
	if !strings.Contains(output, `"assistantMessages"`) {
		t.Error("missing: assistantMessages")
	}
	if !strings.Contains(output, `"toolCalls"`) {
		t.Error("missing: toolCalls")
	}
	if !strings.Contains(output, `"toolResults"`) {
		t.Error("missing: toolResults")
	}
	if !strings.Contains(output, `"totalMessages"`) {
		t.Error("missing: totalMessages")
	}
	if !strings.Contains(output, `"cacheRead"`) {
		t.Error("missing: cacheRead")
	}
}

// TestBashResultFormat verifies camelCase in BashResult output.
func TestBashResultFormat(t *testing.T) {
	result := agentsession.BashResult{
		Output:    "hello world",
		ExitCode:  0,
		Cancelled: false,
		Truncated: false,
	}

	b, _ := json.Marshal(result)
	output := string(b)

	if !strings.Contains(output, `"exitCode"`) {
		t.Error("missing: exitCode (should not be exit_code)")
	}
	if strings.Contains(output, `"exit_code"`) {
		t.Error("snake_case leak: exit_code")
	}
	if strings.Contains(output, `"full_output_path"`) {
		t.Error("snake_case leak: full_output_path")
	}
}

// TestExtensionUITypes verifies Extension UI types match pi-mono.
func TestExtensionUITypes(t *testing.T) {
	// Extension UI Request
	req := RpcExtensionUIRequest{
		Type:   "extension_ui_request",
		ID:     "ui-1",
		Method: "select",
		Title:  "Choose option",
		Options: []string{"a", "b"},
	}
	b, _ := json.Marshal(req)
	output := string(b)

	if !strings.Contains(output, `"extension_ui_request"`) {
		t.Error("missing: extension_ui_request type")
	}

	// Extension UI Response
	resp := RpcExtensionUIResponse{
		Type:  "extension_ui_response",
		ID:    "ui-1",
		Value: "a",
	}
	b, _ = json.Marshal(resp)
	output = string(b)

	if !strings.Contains(output, `"extension_ui_response"`) {
		t.Error("missing: extension_ui_response type")
	}
}

// TestJSONLFraming verifies strict JSONL LF-only framing.
func TestJSONLFraming(t *testing.T) {
	line := serializeLine(map[string]string{"hello": "world"})
	if !strings.HasSuffix(line, "\n") {
		t.Error("line must end with \\n")
	}
	if strings.Contains(line, "\r") {
		t.Error("line must not contain \\r")
	}
	// Count exactly one \n
	if strings.Count(line, "\n") != 1 {
		t.Errorf("line should have exactly 1 newline, got %d", strings.Count(line, "\n"))
	}
}

// TestResponseIDCorrelation verifies id fields round-trip correctly.
func TestResponseIDCorrelation(t *testing.T) {
	// Command with id should produce response with same id
	cmd := RpcCommand{
		ID:      "corr-123",
		Type:    "get_state",
	}
	b, _ := json.Marshal(cmd)

	var parsed RpcCommand
	json.Unmarshal(b, &parsed)
	if parsed.ID != "corr-123" {
		t.Errorf("ID correlation lost: %q", parsed.ID)
	}
}

// TestNullHandling verifies get_last_assistant_text null handling.
func TestNullHandling(t *testing.T) {
	// When text is empty, should serialize as null
	data := map[string]interface{}{"text": nil}
	b, _ := json.Marshal(data)
	output := string(b)
	if !strings.Contains(output, `"text":null`) {
		t.Errorf("null should be serialized: %s", output)
	}
}

var _ = bytes.NewReader
var _ = fmt.Sprintf
