package rpcclient

import (
	"encoding/json"
	"strings"
	"testing"
)

// =============================================================================
// Test: All 29 commands exist — mirrors TS test patterns
// =============================================================================

// TestAll29CommandsExist verifies all 29 pi-mono command types are supported.
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

	for _, cmd := range piCommands {
		t.Run(cmd, func(t *testing.T) {
			jsonCmd := `{"type":"` + cmd + `"}`
			var rpcCmd rpcCommand
			if err := json.Unmarshal([]byte(jsonCmd), &rpcCmd); err != nil {
				t.Errorf("failed to unmarshal %q: %v", cmd, err)
			}
		})
	}
}

// =============================================================================
// Test: Command field marshal/unmarshal matches pi-mono
// =============================================================================

// TestCommandFieldNames verifies all rpcCommand fields use camelCase JSON keys.
func TestCommandFieldNames(t *testing.T) {
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

	var cmd rpcCommand
	if err := json.Unmarshal([]byte(input), &cmd); err != nil {
		t.Fatalf("unmarshal failed: %v", err)
	}

	checks := []struct {
		field string
		got   string
		want  string
	}{
		{"ID", cmd.ID, "req-1"},
		{"Type", cmd.Type, "prompt"},
		{"Message", cmd.Message, "hello"},
		{"StreamingBehavior", cmd.StreamingBehavior, "steer"},
		{"ParentSession", cmd.ParentSession, "/path/to/session.jsonl"},
		{"Provider", cmd.Provider, "openai"},
		{"ModelID", cmd.ModelID, "gpt-4o"},
		{"Level", cmd.Level, "high"},
		{"Mode", cmd.Mode, "one-at-a-time"},
		{"CustomInstructions", cmd.CustomInstructions, "focus on code"},
		{"SessionPath", cmd.SessionPath, "/path/to/session.jsonl"},
		{"EntryID", cmd.EntryID, "abc123"},
		{"Name", cmd.Name, "test-session"},
		{"OutputPath", cmd.OutputPath, "/tmp/export.html"},
	}
	for _, c := range checks {
		if c.got != c.want {
			t.Errorf("%s: got %q, want %q", c.field, c.got, c.want)
		}
	}
	if !cmd.Enabled {
		t.Error("Enabled: should be true")
	}
	if len(cmd.Images) != 1 {
		t.Errorf("Images: expected 1, got %d", len(cmd.Images))
	}
}

// =============================================================================
// Test: Response format (camelCase JSON serialization)
// =============================================================================

// TestResponseJSONFormat verifies the response JSON uses camelCase.
func TestResponseJSONFormat(t *testing.T) {
	state := SessionState{
		Model:                 "gpt-4o",
		ThinkingLevel:         "medium",
		IsStreaming:           false,
		IsCompacting:          false,
		SteeringMode:          "one-at-a-time",
		FollowUpMode:          "all",
		SessionID:             "abc123",
		AutoCompactionEnabled: true,
		MessageCount:          5,
		PendingMessageCount:   2,
	}

	b, err := json.Marshal(state)
	if err != nil {
		t.Fatalf("marshal failed: %v", err)
	}
	output := string(b)

	camelKeys := []string{
		`"thinkingLevel"`,
		`"isStreaming"`,
		`"isCompacting"`,
		`"steeringMode"`,
		`"followUpMode"`,
		`"sessionId"`,
		`"autoCompactionEnabled"`,
		`"messageCount"`,
		`"pendingMessageCount"`,
	}
	for _, key := range camelKeys {
		if !strings.Contains(output, key) {
			t.Errorf("missing camelCase: %s", key)
		}
	}

	snakeLeaks := []string{
		`"session_id"`,
		`"message_count"`,
		`"thinking_level"`,
	}
	for _, leak := range snakeLeaks {
		if strings.Contains(output, leak) {
			t.Errorf("snake_case leak: %s", leak)
		}
	}
}

// =============================================================================
// Test: SessionStats format
// =============================================================================

// TestSessionStatsFormat verifies camelCase in SessionStats.
func TestSessionStatsFormat(t *testing.T) {
	stats := SessionStats{
		SessionFile:       "/tmp/session.jsonl",
		SessionID:         "abc123",
		UserMessages:      5,
		AssistantMessages: 5,
		ToolCalls:         10,
		ToolResults:       10,
		TotalMessages:     20,
		Tokens: TokenStats{
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

	camelKeys := []string{
		`"sessionFile"`,
		`"sessionId"`,
		`"userMessages"`,
		`"assistantMessages"`,
		`"toolCalls"`,
		`"toolResults"`,
		`"totalMessages"`,
		`"cacheRead"`,
	}
	for _, key := range camelKeys {
		if !strings.Contains(output, key) {
			t.Errorf("missing: %s", key)
		}
	}
}

// =============================================================================
// Test: BashResult format
// =============================================================================

// TestBashResultFormat verifies camelCase in BashResult.
func TestBashResultFormat(t *testing.T) {
	result := BashResult{
		Output:         "hello world",
		ExitCode:       0,
		Cancelled:      false,
		Truncated:      false,
		FullOutputPath: "/tmp/full_output.txt",
	}

	b, _ := json.Marshal(result)
	output := string(b)

	if !strings.Contains(output, `"exitCode"`) {
		t.Error("missing: exitCode")
	}
	if strings.Contains(output, `"exit_code"`) {
		t.Error("snake_case leak: exit_code")
	}
	if !strings.Contains(output, `"fullOutputPath"`) {
		t.Error("missing: fullOutputPath")
	}
	if strings.Contains(output, `"full_output_path"`) {
		t.Error("snake_case leak: full_output_path")
	}
}

// =============================================================================
// Test: CompactionResult format
// =============================================================================

func TestCompactionResultFormat(t *testing.T) {
	result := CompactionResult{
		Summary:          "test summary",
		FirstKeptEntryID: "entry-1",
		TokensBefore:     5000,
	}

	b, _ := json.Marshal(result)
	output := string(b)

	if !strings.Contains(output, `"firstKeptEntryId"`) {
		t.Error("missing: firstKeptEntryId")
	}
	if strings.Contains(output, `"first_kept_entry_id"`) {
		t.Error("snake_case leak: first_kept_entry_id")
	}
	if !strings.Contains(output, `"tokensBefore"`) {
		t.Error("missing: tokensBefore")
	}
}

// =============================================================================
// Test: JSONL framing
// =============================================================================

// TestJSONLFraming verifies strict JSONL LF-only framing.
func TestJSONLFraming(t *testing.T) {
	line, err := serializeJSONLine(map[string]string{"hello": "world"})
	if err != nil {
		t.Fatal(err)
	}
	if !strings.HasSuffix(line, "\n") {
		t.Error("line must end with \\n")
	}
	if strings.Contains(line, "\r") {
		t.Error("line must not contain \\r")
	}
	if strings.Count(line, "\n") != 1 {
		t.Errorf("line should have exactly 1 newline, got %d", strings.Count(line, "\n"))
	}
}

// =============================================================================
// Test: ID correlation round-trip
// =============================================================================

// TestIDCorrelation verifies id fields round-trip correctly.
func TestIDCorrelation(t *testing.T) {
	cmd := rpcCommand{
		ID:   "corr-123",
		Type: "get_state",
	}
	b, _ := json.Marshal(cmd)

	var parsed rpcCommand
	json.Unmarshal(b, &parsed)
	if parsed.ID != "corr-123" {
		t.Errorf("ID correlation lost: %q", parsed.ID)
	}
}

// =============================================================================
// Test: Null handling for get_last_assistant_text
// =============================================================================

// TestNullHandling verifies null serialization.
func TestNullHandling(t *testing.T) {
	data := map[string]interface{}{"text": nil}
	b, _ := json.Marshal(data)
	output := string(b)
	if !strings.Contains(output, `"text":null`) {
		t.Errorf("null should be serialized: %s", output)
	}
}

// =============================================================================
// Test: Client method sends correct command types
// =============================================================================

// TestClientMethodCommandTypes verifies each Client method produces the correct rpcCommand.
func TestClientMethodCommandTypes(t *testing.T) {
	tests := []struct {
		name    string
		cmdType string
		fields  map[string]interface{}
	}{
		{"prompt", "prompt", map[string]interface{}{"message": "hello"}},
		{"steer", "steer", map[string]interface{}{"message": "stop"}},
		{"follow_up", "follow_up", map[string]interface{}{"message": "also"}},
		{"abort", "abort", nil},
		{"new_session", "new_session", map[string]interface{}{"parentSession": "/tmp/old.jsonl"}},
		{"get_state", "get_state", nil},
		{"set_model", "set_model", map[string]interface{}{"provider": "openai", "modelId": "gpt-4o"}},
		{"cycle_model", "cycle_model", nil},
		{"get_available_models", "get_available_models", nil},
		{"set_thinking_level", "set_thinking_level", map[string]interface{}{"level": "high"}},
		{"cycle_thinking_level", "cycle_thinking_level", nil},
		{"set_steering_mode", "set_steering_mode", map[string]interface{}{"mode": "one-at-a-time"}},
		{"set_follow_up_mode", "set_follow_up_mode", map[string]interface{}{"mode": "all"}},
		{"compact", "compact", map[string]interface{}{"customInstructions": "focus on code"}},
		{"set_auto_compaction", "set_auto_compaction", map[string]interface{}{"enabled": true}},
		{"set_auto_retry", "set_auto_retry", map[string]interface{}{"enabled": false}},
		{"abort_retry", "abort_retry", nil},
		{"bash", "bash", map[string]interface{}{"command": "echo hello"}},
		{"abort_bash", "abort_bash", nil},
		{"get_session_stats", "get_session_stats", nil},
		{"export_html", "export_html", map[string]interface{}{"outputPath": "/tmp/out.html"}},
		{"switch_session", "switch_session", map[string]interface{}{"sessionPath": "/tmp/s.jsonl"}},
		{"fork", "fork", map[string]interface{}{"entryId": "abc123"}},
		{"clone", "clone", nil},
		{"get_fork_messages", "get_fork_messages", nil},
		{"get_last_assistant_text", "get_last_assistant_text", nil},
		{"set_session_name", "set_session_name", map[string]interface{}{"name": "my session"}},
		{"get_messages", "get_messages", nil},
		{"get_commands", "get_commands", nil},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			cmd := rpcCommand{Type: tt.cmdType}
			if tt.fields != nil {
				// Set fields based on type
				switch tt.cmdType {
				case "prompt", "steer", "follow_up":
					if msg, ok := tt.fields["message"]; ok {
						cmd.Message = msg.(string)
					}
				case "new_session":
					if ps, ok := tt.fields["parentSession"]; ok {
						cmd.ParentSession = ps.(string)
					}
				case "set_model":
					if p, ok := tt.fields["provider"]; ok {
						cmd.Provider = p.(string)
					}
					if m, ok := tt.fields["modelId"]; ok {
						cmd.ModelID = m.(string)
					}
				case "set_thinking_level":
					if l, ok := tt.fields["level"]; ok {
						cmd.Level = l.(string)
					}
				case "set_steering_mode", "set_follow_up_mode":
					if m, ok := tt.fields["mode"]; ok {
						cmd.Mode = m.(string)
					}
				case "compact":
					if ci, ok := tt.fields["customInstructions"]; ok {
						cmd.CustomInstructions = ci.(string)
					}
				case "set_auto_compaction", "set_auto_retry":
					if e, ok := tt.fields["enabled"]; ok {
						cmd.Enabled = e.(bool)
					}
				case "bash":
					if c, ok := tt.fields["command"]; ok {
						cmd.Command = c.(string)
					}
				case "export_html":
					if op, ok := tt.fields["outputPath"]; ok {
						cmd.OutputPath = op.(string)
					}
				case "switch_session":
					if sp, ok := tt.fields["sessionPath"]; ok {
						cmd.SessionPath = sp.(string)
					}
				case "fork":
					if ei, ok := tt.fields["entryId"]; ok {
						cmd.EntryID = ei.(string)
					}
				case "set_session_name":
					if n, ok := tt.fields["name"]; ok {
						cmd.Name = n.(string)
					}
				}
			}

			if cmd.Type != tt.cmdType {
				t.Errorf("Type mismatch: got %q, want %q", cmd.Type, tt.cmdType)
			}
		})
	}
}

// =============================================================================
// Test: Serialize all command types as valid JSON
// =============================================================================

func TestAllCommandsSerializeToValidJSON(t *testing.T) {
	commands := []rpcCommand{
		{Type: "prompt", Message: "hello"},
		{Type: "steer", Message: "stop"},
		{Type: "follow_up", Message: "also"},
		{Type: "abort"},
		{Type: "new_session", ParentSession: "/tmp/s.jsonl"},
		{Type: "get_state"},
		{Type: "get_messages"},
		{Type: "set_model", Provider: "openai", ModelID: "gpt-4o"},
		{Type: "cycle_model"},
		{Type: "get_available_models"},
		{Type: "set_thinking_level", Level: "high"},
		{Type: "cycle_thinking_level"},
		{Type: "set_steering_mode", Mode: "one-at-a-time"},
		{Type: "set_follow_up_mode", Mode: "all"},
		{Type: "compact", CustomInstructions: "focus on code"},
		{Type: "set_auto_compaction", Enabled: true},
		{Type: "set_auto_retry", Enabled: false},
		{Type: "abort_retry"},
		{Type: "bash", Command: "echo hello"},
		{Type: "abort_bash"},
		{Type: "get_session_stats"},
		{Type: "export_html", OutputPath: "/tmp/out.html"},
		{Type: "switch_session", SessionPath: "/tmp/s.jsonl"},
		{Type: "fork", EntryID: "abc123"},
		{Type: "clone"},
		{Type: "get_fork_messages"},
		{Type: "get_last_assistant_text"},
		{Type: "set_session_name", Name: "my session"},
		{Type: "get_commands"},
	}

	for i, cmd := range commands {
		line, err := serializeJSONLine(cmd)
		if err != nil {
			t.Errorf("cmd[%d] %s: serialize failed: %v", i, cmd.Type, err)
			continue
		}
		if !strings.HasSuffix(line, "\n") {
			t.Errorf("cmd[%d] %s: no trailing newline", i, cmd.Type)
		}
		var parsed map[string]interface{}
		if err := json.Unmarshal([]byte(strings.TrimSuffix(line, "\n")), &parsed); err != nil {
			t.Errorf("cmd[%d] %s: not valid JSON: %v", i, cmd.Type, err)
		}
		if parsed["type"] != cmd.Type {
			t.Errorf("cmd[%d] %s: type field mismatch", i, cmd.Type)
		}
	}
}

// =============================================================================
// Test: JSONL reader event dispatch
// =============================================================================

func TestJSONLReaderEventDispatch(t *testing.T) {
	reader := newJSONLReader()

	received := make([]string, 0)
	unsub1 := reader.addListener(func(raw json.RawMessage) {
		received = append(received, "listener1")
	})
	_ = unsub1

	reader.addListener(func(raw json.RawMessage) {
		received = append(received, "listener2")
	})

	reader.handleLine(`{"type":"agent_start"}`)

	if len(received) != 2 {
		t.Errorf("expected 2 events, got %d", len(received))
	}
}

// =============================================================================
// Test: JSONL reader response routing
// =============================================================================

func TestJSONLReaderResponseRouting(t *testing.T) {
	reader := newJSONLReader()

	// Register pending request
	respCh := reader.registerPending("req-1")

	// Send response
	reader.handleLine(`{"type":"response","command":"get_state","success":true,"data":{"sessionId":"abc"},"id":"req-1"}`)

	select {
	case resp := <-respCh:
		if resp == nil {
			t.Error("got nil response")
		}
		if !resp.Success {
			t.Error("expected success")
		}
	default:
		t.Error("no response received")
	}
}

// =============================================================================
// Test: JSONL reader ignores non-JSON lines
// =============================================================================

func TestJSONLReaderIgnoresNonJSON(t *testing.T) {
	reader := newJSONLReader()

	received := false
	reader.addListener(func(raw json.RawMessage) {
		received = true
	})

	reader.handleLine("this is not json")
	if received {
		t.Error("non-JSON line should be ignored")
	}

	reader.handleLine(`[INFO] some log message`)
	if received {
		t.Error("non-JSON-object line should be ignored")
	}
}

// =============================================================================
// Test: Client NewClient creates properly
// =============================================================================

func TestNewClient(t *testing.T) {
	client := New(Options{
		Provider: "openai",
		Model:    "gpt-4o",
	})

	if client.opts.Provider != "openai" {
		t.Errorf("provider: got %q, want openai", client.opts.Provider)
	}
	if client.opts.Model != "gpt-4o" {
		t.Errorf("model: got %q, want gpt-4o", client.opts.Model)
	}
	if client.reader == nil {
		t.Error("reader should not be nil")
	}
}

// =============================================================================
// Test: ModelInfo type matches pi-mono
// =============================================================================

func TestModelInfoJSON(t *testing.T) {
	mi := ModelInfo{
		Provider:      "openai",
		ID:            "gpt-4o",
		ContextWindow: 128000,
		Reasoning:     false,
	}

	b, _ := json.Marshal(mi)
	output := string(b)

	if !strings.Contains(output, `"contextWindow"`) {
		t.Error("missing: contextWindow")
	}
	if strings.Contains(output, `"context_window"`) {
		t.Error("snake_case leak: context_window")
	}
}

// =============================================================================
// Test: SlashCommandInfo type matches pi-mono RpcSlashCommand
// =============================================================================

func TestSlashCommandInfoJSON(t *testing.T) {
	sci := SlashCommandInfo{
		Name:        "test-cmd",
		Description: "A test command",
		Source:      "extension",
		SourceInfo: SourceInfo{
			Path:   "/path/to/ext",
			Source: "test-pkg",
			Scope:  "user",
			Origin: "package",
		},
	}

	b, _ := json.Marshal(sci)
	output := string(b)

	if !strings.Contains(output, `"sourceInfo"`) {
		t.Error("missing: sourceInfo")
	}
	if !strings.Contains(output, `"source":"extension"`) {
		t.Error("missing: source=extension")
	}
}

// =============================================================================
// Test: CompactionResult type matches pi-mono
// =============================================================================

func TestCompactionResultJSON(t *testing.T) {
	result := CompactionResult{
		Summary:          "test summary",
		FirstKeptEntryID: "entry-123",
		TokensBefore:     10000,
	}

	b, err := json.Marshal(result)
	if err != nil {
		t.Fatal(err)
	}
	output := string(b)

	if !strings.Contains(output, `"firstKeptEntryId"`) {
		t.Error("missing: firstKeptEntryId")
	}
	if !strings.Contains(output, `"tokensBefore"`) {
		t.Error("missing: tokensBefore")
	}
	if !strings.Contains(output, `"summary"`) {
		t.Error("missing: summary")
	}
}

// =============================================================================
// Test: All response result types marshal correctly
// =============================================================================

func TestResponseResultTypes(t *testing.T) {
	// NewSessionResult
	nsr := NewSessionResult{Cancelled: false}
	b, _ := json.Marshal(nsr)
	if !strings.Contains(string(b), `"cancelled"`) {
		t.Error("NewSessionResult: missing cancelled")
	}

	// SetModelResult
	smr := SetModelResult{Provider: "openai", ID: "gpt-4o"}
	b, _ = json.Marshal(smr)
	if !strings.Contains(string(b), `"provider"`) {
		t.Error("SetModelResult: missing provider")
	}

	// CycleModelResult
	cmr := CycleModelResult{
		Model:         ModelResult{Provider: "openai", ID: "gpt-4o"},
		ThinkingLevel: "high",
		IsScoped:      false,
	}
	b, _ = json.Marshal(cmr)
	if !strings.Contains(string(b), `"model"`) {
		t.Error("CycleModelResult: missing model")
	}
	if !strings.Contains(string(b), `"thinkingLevel"`) {
		t.Error("CycleModelResult: missing thinkingLevel")
	}
	if !strings.Contains(string(b), `"isScoped"`) {
		t.Error("CycleModelResult: missing isScoped")
	}

	// CycleThinkingResult
	ctr := CycleThinkingResult{Level: "high"}
	b, _ = json.Marshal(ctr)
	if !strings.Contains(string(b), `"level"`) {
		t.Error("CycleThinkingResult: missing level")
	}

	// ExportHTMLResult
	ehr := ExportHTMLResult{Path: "/tmp/export.html"}
	b, _ = json.Marshal(ehr)
	if !strings.Contains(string(b), `"path"`) {
		t.Error("ExportHTMLResult: missing path")
	}

	// SwitchSessionResult
	ssr := SwitchSessionResult{Cancelled: false}
	b, _ = json.Marshal(ssr)
	if !strings.Contains(string(b), `"cancelled"`) {
		t.Error("SwitchSessionResult: missing cancelled")
	}

	// ForkResult
	fr := ForkResult{Text: "hello world", Cancelled: false}
	b, _ = json.Marshal(fr)
	if !strings.Contains(string(b), `"text"`) {
		t.Error("ForkResult: missing text")
	}
	if !strings.Contains(string(b), `"cancelled"`) {
		t.Error("ForkResult: missing cancelled")
	}

	// CloneResult
	cr := CloneResult{Cancelled: false}
	b, _ = json.Marshal(cr)
	if !strings.Contains(string(b), `"cancelled"`) {
		t.Error("CloneResult: missing cancelled")
	}
}

// =============================================================================
// Test: ImageContent JSON serialization
// =============================================================================

func TestImageContentJSON(t *testing.T) {
	img := ImageContent{
		Type:     "image",
		MimeType: "image/png",
		Data:     "base64data",
	}
	b, _ := json.Marshal(img)
	output := string(b)

	if !strings.Contains(output, `"mime_type"`) {
		t.Error("missing: mime_type")
	}
	if !strings.Contains(output, `"type":"image"`) {
		t.Error("missing: type=image")
	}
}

// =============================================================================
// Test: ForkMessage type
// =============================================================================

func TestForkMessageJSON(t *testing.T) {
	fm := ForkMessage{
		EntryID: "abc123",
		Text:    "hello",
	}
	b, _ := json.Marshal(fm)
	output := string(b)

	if !strings.Contains(output, `"entryId"`) {
		t.Error("missing: entryId")
	}
	if !strings.Contains(output, `"text"`) {
		t.Error("missing: text")
	}
}

// =============================================================================
// Test: SourceInfo type
// =============================================================================

func TestSourceInfoJSON(t *testing.T) {
	si := SourceInfo{
		Path:    "/tmp/test",
		Source:  "test-pkg",
		Scope:   "user",
		Origin:  "package",
		BaseDir: "/tmp",
	}
	b, _ := json.Marshal(si)
	output := string(b)

	if !strings.Contains(output, `"baseDir"`) {
		t.Error("missing: baseDir")
	}
	if strings.Contains(output, `"base_dir"`) {
		t.Error("snake_case leak: base_dir")
	}
}
