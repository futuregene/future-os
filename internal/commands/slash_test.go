package commands

import (
	"os"
	"path/filepath"
	"strings"
	"testing"
)

func newTestContext() *Context {
	return &Context{
		CWD:              "/tmp",
		SessionDir:       "/tmp/sessions",
		SettingsDir:      "/tmp/.pi",
		CurrentSessionID: "20260508-120000",
		SettingsPath:     "/tmp/settings.json",
		SettingsJSON:     `{"model":"gpt-4o"}`,
	}
}

func TestHandleEmptyInput(t *testing.T) {
	result, err := Handle("", newTestContext())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result != "" {
		t.Errorf("result = %q, want empty", result)
	}
}

func TestHandleWhitespaceInput(t *testing.T) {
	result, err := Handle("   ", newTestContext())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result != "" {
		t.Errorf("result = %q, want empty", result)
	}
}

func TestHandleUnknownCommand(t *testing.T) {
	_, err := Handle("/unknown", newTestContext())
	if err == nil {
		t.Fatal("expected error for unknown command")
	}
	if !strings.Contains(err.Error(), "unknown command") {
		t.Errorf("error = %v", err)
	}
}

func TestHandleHotkeys(t *testing.T) {
	result, err := Handle("/hotkeys", newTestContext())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(result, "Keybindings:") {
		t.Errorf("missing Keybindings in: %s", result)
	}
	if !strings.Contains(result, "Ctrl+C") {
		t.Errorf("missing Ctrl+C")
	}
	if !strings.Contains(result, "/quit") {
		t.Errorf("missing /quit")
	}
}

func TestHandleSettings(t *testing.T) {
	t.Run("with settings JSON", func(t *testing.T) {
		ctx := newTestContext()
		result, err := Handle("/settings", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "gpt-4o") {
			t.Errorf("missing model in settings: %s", result)
		}
	})

	t.Run("with empty settings", func(t *testing.T) {
		ctx := newTestContext()
		ctx.SettingsJSON = ""
		result, err := Handle("/settings", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "No settings loaded") {
			t.Errorf("unexpected: %s", result)
		}
	})

	t.Run("with invalid JSON settings", func(t *testing.T) {
		ctx := newTestContext()
		ctx.SettingsJSON = "not json"
		result, err := Handle("/settings", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "not json") {
			t.Errorf("should show raw JSON: %s", result)
		}
	})
}

func TestHandleQuit(t *testing.T) {
	result, err := Handle("/quit", newTestContext())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result != "QUIT" {
		t.Errorf("result = %s, want QUIT", result)
	}
}

func TestHandleImport(t *testing.T) {
	t.Run("no args", func(t *testing.T) {
		_, err := Handle("/import", newTestContext())
		if err == nil {
			t.Fatal("expected error for missing file")
		}
		if !strings.Contains(strings.ToLower(err.Error()), "usage") {
			t.Errorf("error = %v", err)
		}
	})

	t.Run("file not found", func(t *testing.T) {
		_, err := Handle("/import /nonexistent.jsonl", newTestContext())
		if err == nil {
			t.Fatal("expected error")
		}
	})

	t.Run("valid JSONL file", func(t *testing.T) {
		os.MkdirAll("/tmp/sessions", 0755)
		defer os.RemoveAll("/tmp/sessions")

		tmpDir := t.TempDir()
		path := filepath.Join(tmpDir, "test.jsonl")
		os.WriteFile(path, []byte(`{"type":"user","content":"hello"}`+"\n"), 0644)

		result, err := Handle("/import "+path, newTestContext())
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "IMPORT:") {
			t.Errorf("expected IMPORT sentinel, got: %s", result)
		}
	})

	t.Run("empty JSONL file", func(t *testing.T) {
		tmpDir := t.TempDir()
		path := filepath.Join(tmpDir, "empty.jsonl")
		os.WriteFile(path, []byte(""), 0644)

		_, err := Handle("/import "+path, newTestContext())
		if err == nil {
			t.Fatal("expected error for empty file")
		}
	})

	t.Run("invalid JSONL line", func(t *testing.T) {
		tmpDir := t.TempDir()
		path := filepath.Join(tmpDir, "bad.jsonl")
		os.WriteFile(path, []byte("not json"), 0644)

		_, err := Handle("/import "+path, newTestContext())
		if err == nil {
			t.Fatal("expected error for invalid JSONL")
		}
	})
}

func TestHandleFork(t *testing.T) {
	t.Run("no active session", func(t *testing.T) {
		ctx := newTestContext()
		ctx.CurrentSessionID = ""
		_, err := Handle("/fork entry1", ctx)
		if err == nil {
			t.Fatal("expected error")
		}
		if !strings.Contains(err.Error(), "no active session") {
			t.Errorf("error = %v", err)
		}
	})

	t.Run("no entry id defaults to latest", func(t *testing.T) {
		result, err := Handle("/fork", newTestContext())
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "FORK:") {
			t.Errorf("expected FORK sentinel, got: %s", result)
		}
	})

	t.Run("valid", func(t *testing.T) {
		result, err := Handle("/fork entry1", newTestContext())
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "FORK:") {
			t.Errorf("expected FORK sentinel, got: %s", result)
		}
	})
}

func TestHandleClone(t *testing.T) {
	t.Run("no active session", func(t *testing.T) {
		ctx := newTestContext()
		ctx.CurrentSessionID = ""
		_, err := Handle("/clone", ctx)
		if err == nil {
			t.Fatal("expected error")
		}
	})

	t.Run("valid", func(t *testing.T) {
		result, err := Handle("/clone", newTestContext())
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "CLONE:") {
			t.Errorf("expected CLONE sentinel, got: %s", result)
		}
	})
}

func TestHandleTree(t *testing.T) {
	t.Run("no active session", func(t *testing.T) {
		ctx := newTestContext()
		ctx.CurrentSessionID = ""
		_, err := Handle("/tree", ctx)
		if err == nil {
			t.Fatal("expected error")
		}
	})

	t.Run("valid", func(t *testing.T) {
		ctx := newTestContext()
		ctx.SessionEntries = []SessionEntry{
			{ID: "e1", Type: "user", Content: "hello"},
			{ID: "e2", Type: "assistant", Content: "hi there"},
		}
		result, err := Handle("/tree", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "Session tree") {
			t.Errorf("expected tree view, got: %s", result)
		}
		if !strings.Contains(result, "2 entries") {
			t.Errorf("expected entry count, got: %s", result)
		}
	})
}

func TestHandleSession(t *testing.T) {
	ctx := newTestContext()
	ctx.SessionName = "test-session"
	ctx.Messages = []Message{
		{Role: "user", Content: "hello"},
		{Role: "assistant", Content: "hi"},
		{Role: "user", Content: "how are you"},
		{Role: "assistant", Content: "good"},
	}
	ctx.TokenUsage = &TokenUsage{
		Input:  100,
		Output: 50,
		Total:  150,
	}
	ctx.TotalCost = 0.0023

	result, err := Handle("/session", ctx)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	checks := []string{
		"Session Info",
		"test-session",
		"User: 2",
		"Assistant: 2",
		"Input: 100",
		"Output: 50",
		"Total: 150",
		"$0.0023",
	}
	for _, c := range checks {
		if !strings.Contains(result, c) {
			t.Errorf("missing '%s' in session output", c)
		}
	}
}

func TestHandleName(t *testing.T) {
	t.Run("set name", func(t *testing.T) {
		os.MkdirAll("/tmp/sessions", 0755)
		defer os.RemoveAll("/tmp/sessions")

		ctx := newTestContext()
		result, err := Handle("/name My Session", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "Session name set: My Session") {
			t.Errorf("unexpected: %s", result)
		}
	})

	t.Run("show name", func(t *testing.T) {
		ctx := newTestContext()
		ctx.SessionName = "existing-name"
		result, err := Handle("/name", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "existing-name") {
			t.Errorf("unexpected: %s", result)
		}
	})
}

func TestHandleCopy(t *testing.T) {
	ctx := newTestContext()
	ctx.Messages = []Message{
		{Role: "user", Content: "hello"},
		{Role: "assistant", Content: "Hello! How can I help?"},
	}
	result, err := Handle("/copy", ctx)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(result, "Hello! How can I help?") && !strings.Contains(result, "Copied last agent message to clipboard") {
		t.Errorf("expected last assistant message or clipboard confirmation, got: %s", result)
	}
}

func TestHandleCompactGuard(t *testing.T) {
	t.Run("not enough messages", func(t *testing.T) {
		ctx := newTestContext()
		_, err := Handle("/compact", ctx)
		if err == nil {
			t.Fatal("expected error for too few messages")
		}
	})

	t.Run("already compacting", func(t *testing.T) {
		ctx := newTestContext()
		ctx.Messages = []Message{{Role: "user"}, {Role: "assistant"}}
		ctx.IsCompacting = true
		_, err := Handle("/compact", ctx)
		if err == nil {
			t.Fatal("expected error when already compacting")
		}
	})

	t.Run("valid returns sentinel", func(t *testing.T) {
		ctx := newTestContext()
		ctx.Messages = []Message{{Role: "user"}, {Role: "assistant"}}
		result, err := Handle("/compact", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "COMPACT:") {
			t.Errorf("expected COMPACT sentinel, got: %s", result)
		}
	})
}

func TestHandleReloadGuard(t *testing.T) {
	t.Run("streaming", func(t *testing.T) {
		ctx := newTestContext()
		ctx.IsStreaming = true
		_, err := Handle("/reload", ctx)
		if err == nil {
			t.Fatal("expected error when streaming")
		}
	})

	t.Run("compacting", func(t *testing.T) {
		ctx := newTestContext()
		ctx.IsCompacting = true
		_, err := Handle("/reload", ctx)
		if err == nil {
			t.Fatal("expected error when compacting")
		}
	})

	t.Run("valid returns sentinel", func(t *testing.T) {
		ctx := newTestContext()
		result, err := Handle("/reload", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if result != "RELOAD" {
			t.Errorf("expected RELOAD sentinel, got: %s", result)
		}
	})
}

func TestHandleLogout(t *testing.T) {
	result, err := Handle("/logout", newTestContext())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(result, "unset LLM_API_KEY") {
		t.Errorf("unexpected: %s", result)
	}
}

func TestHandleCaseInsensitivity(t *testing.T) {
	result, err := Handle("/QUIT", newTestContext())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result != "QUIT" {
		t.Errorf("result = %s, want QUIT", result)
	}
}

func TestHandleNewSentinel(t *testing.T) {
	result, err := Handle("/new", newTestContext())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result != "NEW_SESSION" {
		t.Errorf("expected NEW_SESSION sentinel, got: %s", result)
	}
}

func TestHandleResumeSentinel(t *testing.T) {
	t.Run("no args returns selector", func(t *testing.T) {
		result, err := Handle("/resume", newTestContext())
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if result != "RESUME_SELECTOR" {
			t.Errorf("expected RESUME_SELECTOR, got: %s", result)
		}
	})

	t.Run("with id returns resume", func(t *testing.T) {
		result, err := Handle("/resume abc123", newTestContext())
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "RESUME:abc123") {
			t.Errorf("expected RESUME:abc123, got: %s", result)
		}
	})
}

// ---------------------------------------------------------------------------
// handleModel tests
// ---------------------------------------------------------------------------

func TestHandleModel(t *testing.T) {
	t.Run("show current model", func(t *testing.T) {
		ctx := newTestContext()
		ctx.Model = "claude-sonnet-4"
		result, err := Handle("/model", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "Current model: claude-sonnet-4") {
			t.Errorf("expected current model, got: %s", result)
		}
	})

	t.Run("set model", func(t *testing.T) {
		ctx := newTestContext()
		result, err := Handle("/model gpt-4o", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "Model set to: gpt-4o") {
			t.Errorf("expected set message, got: %s", result)
		}
		if ctx.Model != "gpt-4o" {
			t.Errorf("ctx.Model = %s, want gpt-4o", ctx.Model)
		}
	})
}

// ---------------------------------------------------------------------------
// handleBaseURL tests
// ---------------------------------------------------------------------------

func TestHandleBaseURL(t *testing.T) {
	t.Run("show current base URL", func(t *testing.T) {
		ctx := newTestContext()
		ctx.BaseURL = "https://api.openai.com"
		result, err := Handle("/baseurl", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "Current base URL: https://api.openai.com") {
			t.Errorf("expected current base URL, got: %s", result)
		}
	})

	t.Run("set base URL", func(t *testing.T) {
		ctx := newTestContext()
		result, err := Handle("/baseurl https://api.anthropic.com", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "Base URL set to: https://api.anthropic.com") {
			t.Errorf("expected set message, got: %s", result)
		}
		if ctx.BaseURL != "https://api.anthropic.com" {
			t.Errorf("ctx.BaseURL = %s, want https://api.anthropic.com", ctx.BaseURL)
		}
	})
}

// ---------------------------------------------------------------------------
// handleMemory tests
// ---------------------------------------------------------------------------

func TestHandleMemory(t *testing.T) {
	ctx := newTestContext()
	ctx.Model = "gpt-4o"
	ctx.BaseURL = "https://api.openai.com"
	result, err := Handle("/memory", ctx)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(result, "Memory:") {
		t.Errorf("expected Memory header, got: %s", result)
	}
	if !strings.Contains(result, "model=gpt-4o") {
		t.Errorf("expected model in memory, got: %s", result)
	}
	if !strings.Contains(result, "base_url=https://api.openai.com") {
		t.Errorf("expected base URL in memory, got: %s", result)
	}
}

// ---------------------------------------------------------------------------
// handleClear tests
// ---------------------------------------------------------------------------

func TestHandleClear(t *testing.T) {
	t.Run("no args shows usage", func(t *testing.T) {
		result, err := Handle("/clear", newTestContext())
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "No session specified") {
			t.Errorf("expected usage message, got: %s", result)
		}
	})

	t.Run("with session id", func(t *testing.T) {
		result, err := Handle("/clear abc123", newTestContext())
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "Deleted session abc123") {
			t.Errorf("expected deletion message, got: %s", result)
		}
	})
}

// ---------------------------------------------------------------------------
// handleScopedModels tests
// ---------------------------------------------------------------------------

func TestHandleScopedModels(t *testing.T) {
	ctx := newTestContext()
	ctx.Model = "claude-3-opus"
	result, err := Handle("/scoped-models", ctx)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(result, "Scoped models: default model=claude-3-opus") {
		t.Errorf("expected scoped models output, got: %s", result)
	}
	if !strings.Contains(result, "LLM_MODEL=") {
		t.Errorf("expected LLM_MODEL env ref, got: %s", result)
	}
	if !strings.Contains(result, "LLM_BASE_URL=") {
		t.Errorf("expected LLM_BASE_URL env ref, got: %s", result)
	}
}

// ---------------------------------------------------------------------------
// handleExport tests
// ---------------------------------------------------------------------------

func TestHandleExport(t *testing.T) {
	t.Run("no session id", func(t *testing.T) {
		ctx := newTestContext()
		ctx.CurrentSessionID = ""
		_, err := Handle("/export", ctx)
		if err == nil {
			t.Fatal("expected error when no session")
		}
		if !strings.Contains(err.Error(), "No session to export") {
			t.Errorf("error = %v", err)
		}
	})

	t.Run("JSONL export", func(t *testing.T) {
		os.MkdirAll("/tmp/sessions", 0755)
		defer os.RemoveAll("/tmp/sessions")

		srcPath := "/tmp/sessions/20260508-120000.jsonl"
		os.WriteFile(srcPath, []byte(`{"type":"user","content":"hello"}`+"\n"), 0644)

		result, err := Handle("/export /tmp/sessions/exported.jsonl", newTestContext())
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "Session exported to:") {
			t.Errorf("expected export success, got: %s", result)
		}

		// Verify the exported file exists
		data, err := os.ReadFile("/tmp/sessions/exported.jsonl")
		if err != nil {
			t.Fatalf("exported file not found: %v", err)
		}
		if !strings.Contains(string(data), "hello") {
			t.Errorf("exported content missing: %s", string(data))
		}
	})

	t.Run("HTML export default path", func(t *testing.T) {
		os.MkdirAll("/tmp/sessions", 0755)
		defer os.RemoveAll("/tmp/sessions")

		ctx := newTestContext()
		ctx.Model = "gpt-4"
		ctx.CWD = "/home/user"
		ctx.Messages = []Message{
			{Role: "user", Content: "hi"},
			{Role: "assistant", Content: "hello!"},
		}

		result, err := Handle("/export /tmp/sessions/test_export.html", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "Session exported to:") {
			t.Errorf("expected export success, got: %s", result)
		}

		data, err := os.ReadFile("/tmp/sessions/test_export.html")
		if err != nil {
			t.Fatalf("exported file not found: %v", err)
		}
		html := string(data)
		if !strings.Contains(html, "<!DOCTYPE html>") {
			t.Errorf("expected HTML doctype, got: %s", html[:200])
		}
		if !strings.Contains(html, "xihu session") {
			t.Errorf("expected session title, got: %s", html[:200])
		}
		if !strings.Contains(html, "gpt-4") {
			t.Errorf("expected model in HTML, got: %s", html[:200])
		}
	})

	t.Run("HTML export with special characters", func(t *testing.T) {
		os.MkdirAll("/tmp/sessions", 0755)
		defer os.RemoveAll("/tmp/sessions")

		ctx := newTestContext()
		ctx.Messages = []Message{
			{Role: "user", Content: "use <div> & \"quotes\""},
		}

		result, err := Handle("/export /tmp/sessions/test_escaped.html", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "Session exported to:") {
			t.Errorf("expected export success, got: %s", result)
		}

		data, err := os.ReadFile("/tmp/sessions/test_escaped.html")
		if err != nil {
			t.Fatalf("exported file not found: %v", err)
		}
		html := string(data)
		if !strings.Contains(html, "&lt;div&gt;") {
			t.Errorf("expected escaped HTML, got: %s", html)
		}
		if !strings.Contains(html, "&amp;") {
			t.Errorf("expected escaped ampersand, got: %s", html)
		}
	})
}

// ---------------------------------------------------------------------------
// escapeHTML tests
// ---------------------------------------------------------------------------

func TestEscapeHTML(t *testing.T) {
	tests := []struct {
		input    string
		expected string
	}{
		{"plain text", "plain text"},
		{"<script>alert('xss')</script>", "&lt;script&gt;alert('xss')&lt;/script&gt;"},
		{"a & b", "a &amp; b"},
		{"a > b < c", "a &gt; b &lt; c"},
		{"&<>", "&amp;&lt;&gt;"},
	}

	for _, tc := range tests {
		result := escapeHTML(tc.input)
		if result != tc.expected {
			t.Errorf("escapeHTML(%q) = %q, want %q", tc.input, result, tc.expected)
		}
	}
}

// ---------------------------------------------------------------------------
// exportSessionToHTML tests
// ---------------------------------------------------------------------------

func TestExportSessionToHTML(t *testing.T) {
	ctx := &Context{
		CWD:              "/home/test",
		Model:            "gpt-4o",
		CurrentSessionID: "sid-123",
		Messages: []Message{
			{Role: "user", Content: "hello world"},
			{Role: "assistant", Content: "hi there"},
		},
	}

	html := exportSessionToHTML("sid-123", ctx)

	checks := []string{
		"<!DOCTYPE html>",
		"<title>xihu session sid-123</title>",
		"<h1>xihu Session: sid-123</h1>",
		"Model: gpt-4o",
		"CWD: /home/test",
		"class=\"user\"",
		"class=\"assistant\"",
		"<strong>user</strong>",
		"<strong>assistant</strong>",
		"hello world",
		"hi there",
		"</body></html>",
	}

	for _, c := range checks {
		if !strings.Contains(html, c) {
			t.Errorf("missing '%s' in HTML output", c)
		}
	}
}

// ---------------------------------------------------------------------------
// handleShare tests
// ---------------------------------------------------------------------------

func TestHandleShare(t *testing.T) {
	t.Run("gh not installed or not logged in", func(t *testing.T) {
		os.MkdirAll("/tmp/sessions", 0755)
		defer os.RemoveAll("/tmp/sessions")

		ctx := newTestContext()
		ctx.CurrentSessionID = "test-session-share"
		ctx.Messages = []Message{{Role: "user", Content: "hi"}}

		result, err := Handle("/share", ctx)
		if err == nil {
			// gh is fully configured; verify we got a share URL
			if !strings.Contains(result, "Share URL:") && !strings.Contains(result, "Gist:") {
				t.Errorf("expected share success with URL, got: %s", result)
			}
			t.Log("gh is installed and logged in; share succeeded")
			return
		}
		// Accept any gh-related error
		errStr := strings.ToLower(err.Error())
		if !strings.Contains(errStr, "github cli") && !strings.Contains(errStr, "gh") && !strings.Contains(errStr, "gist") {
			t.Errorf("unexpected error: %v", err)
		}
	})
}

// ---------------------------------------------------------------------------
// handleChangelog tests
// ---------------------------------------------------------------------------

func TestHandleChangelog(t *testing.T) {
	t.Run("changelog file exists", func(t *testing.T) {
		os.MkdirAll("/tmp/.pi", 0755)
		defer os.RemoveAll("/tmp/.pi")

		changelogContent := "## v3.0.0\n- New feature: slash commands"
		os.WriteFile("/tmp/.pi/CHANGELOG.md", []byte(changelogContent), 0644)

		ctx := newTestContext()
		ctx.SettingsDir = "/tmp/.pi"
		result, err := Handle("/changelog", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "v3.0.0") {
			t.Errorf("expected changelog content, got: %s", result)
		}
	})

	t.Run("changelog file missing falls back", func(t *testing.T) {
		ctx := newTestContext()
		ctx.SettingsDir = "/nonexistent_dir_xyz"
		result, err := Handle("/changelog", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "xihu v0.2.0") {
			t.Errorf("expected fallback changelog, got: %s", result)
		}
		if !strings.Contains(result, "21 slash commands") {
			t.Errorf("expected slash commands mention, got: %s", result)
		}
	})

	t.Run("empty settings dir uses session dir", func(t *testing.T) {
		ctx := newTestContext()
		ctx.SettingsDir = ""
		// Should fall back to SessionDir + "/../CHANGELOG.md" which likely doesn't exist
		result, err := Handle("/changelog", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "xihu v0.2.0") {
			t.Errorf("expected fallback changelog, got: %s", result)
		}
	})
}

// ---------------------------------------------------------------------------
// handleHelp tests
// ---------------------------------------------------------------------------

func TestHandleHelp(t *testing.T) {
	result, err := Handle("/help", newTestContext())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(result, "xihu — AI coding assistant") {
		t.Errorf("expected help header, got: %s", result)
	}
	if !strings.Contains(result, "Quick start") {
		t.Errorf("expected Quick start section, got: %s", result)
	}
	if !strings.Contains(result, "/hotkeys") {
		t.Errorf("expected /hotkeys reference, got: %s", result)
	}
}

// ---------------------------------------------------------------------------
// handleLogin tests
// ---------------------------------------------------------------------------

func TestHandleLogin(t *testing.T) {
	result, err := Handle("/login", newTestContext())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if !strings.Contains(result, "Authentication:") {
		t.Errorf("expected authentication header, got: %s", result)
	}
	if !strings.Contains(result, "LLM_API_KEY") {
		t.Errorf("expected LLM_API_KEY mention, got: %s", result)
	}
	if !strings.Contains(result, "ANTHROPIC_API_KEY") {
		t.Errorf("expected ANTHROPIC_API_KEY mention, got: %s", result)
	}
}

// ---------------------------------------------------------------------------
// countMessagesByRole tests
// ---------------------------------------------------------------------------

func TestCountMessagesByRole(t *testing.T) {
	msgs := []Message{
		{Role: "user", Content: "a"},
		{Role: "assistant", Content: "b"},
		{Role: "user", Content: "c"},
		{Role: "assistant", Content: "d"},
		{Role: "tool", Content: "tool result 1"},
		{Role: "tool", Content: "tool result 2"},
	}

	user, assistant, tool := countMessagesByRole(msgs)
	if user != 2 {
		t.Errorf("user count = %d, want 2", user)
	}
	if assistant != 2 {
		t.Errorf("assistant count = %d, want 2", assistant)
	}
	if tool != 2 {
		t.Errorf("tool count = %d, want 2", tool)
	}
}

func TestCountMessagesByRoleEmpty(t *testing.T) {
	user, assistant, tool := countMessagesByRole(nil)
	if user != 0 || assistant != 0 || tool != 0 {
		t.Errorf("expected all zeros, got: user=%d assistant=%d tool=%d", user, assistant, tool)
	}
}

func TestCountMessagesByRoleUnknownRole(t *testing.T) {
	msgs := []Message{
		{Role: "system", Content: "system prompt"},
		{Role: "unknown", Content: "something else"},
	}
	user, assistant, tool := countMessagesByRole(msgs)
	if user != 0 || assistant != 0 || tool != 0 {
		t.Errorf("expected all zeros for system/unknown roles, got: user=%d assistant=%d tool=%d", user, assistant, tool)
	}
}

// ---------------------------------------------------------------------------
// buildSessionTree tests
// ---------------------------------------------------------------------------

func TestBuildSessionTree(t *testing.T) {
	t.Run("empty entries", func(t *testing.T) {
		result := buildSessionTree(nil, "root123")
		if !strings.Contains(result, "Session tree") {
			t.Errorf("expected tree header, got: %s", result)
		}
		if !strings.Contains(result, "(no entries)") {
			t.Errorf("expected no entries message, got: %s", result)
		}
	})

	t.Run("with entries", func(t *testing.T) {
		entries := []SessionEntry{
			{ID: "e1", Type: "user", Content: "hello"},
			{ID: "e2", Type: "assistant", Content: "hi"},
			{ID: "e3", Type: "user", Content: "how are you"},
			{ID: "e4", Type: "tool", Content: "result"},
		}
		result := buildSessionTree(entries, "root123")

		if !strings.Contains(result, "4 entries") {
			t.Errorf("expected 4 entries, got: %s", result)
		}
		if !strings.Contains(result, "user: 2") {
			t.Errorf("expected user: 2, got: %s", result)
		}
		if !strings.Contains(result, "assistant: 1") {
			t.Errorf("expected assistant: 1, got: %s", result)
		}
		if !strings.Contains(result, "tool: 1") {
			t.Errorf("expected tool: 1, got: %s", result)
		}
	})
}

// ---------------------------------------------------------------------------
// findClipCmd tests
// ---------------------------------------------------------------------------

func TestFindClipCmd(t *testing.T) {
	// In CI environments, pbcopy/xclip/wl-copy might not be installed
	// We just verify the function doesn't panic and returns something reasonable
	result := findClipCmd()
	// Should either return nil (no clipboard found) or a valid command
	if result != nil {
		if len(result) == 0 {
			t.Error("expected non-empty command or nil")
		}
	}
}

// ---------------------------------------------------------------------------
// handleCopy edge case tests
// ---------------------------------------------------------------------------

func TestHandleCopyEdgeCases(t *testing.T) {
	t.Run("no messages", func(t *testing.T) {
		ctx := newTestContext()
		ctx.Messages = nil
		_, err := Handle("/copy", ctx)
		if err == nil {
			t.Fatal("expected error when no messages")
		}
		if !strings.Contains(err.Error(), "No agent messages") {
			t.Errorf("error = %v", err)
		}
	})

	t.Run("only user messages", func(t *testing.T) {
		ctx := newTestContext()
		ctx.Messages = []Message{
			{Role: "user", Content: "hello"},
			{Role: "user", Content: "world"},
		}
		_, err := Handle("/copy", ctx)
		if err == nil {
			t.Fatal("expected error when no assistant message")
		}
		if !strings.Contains(err.Error(), "No agent messages") {
			t.Errorf("error = %v", err)
		}
	})

	t.Run("multiple assistant messages takes last", func(t *testing.T) {
		ctx := newTestContext()
		ctx.Messages = []Message{
			{Role: "user", Content: "q1"},
			{Role: "assistant", Content: "first answer"},
			{Role: "user", Content: "q2"},
			{Role: "assistant", Content: "second answer"},
		}
		result, err := Handle("/copy", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		// Should contain the last assistant message
		if !strings.Contains(result, "second answer") && !strings.Contains(result, "Copied last agent message") {
			t.Errorf("expected last assistant message, got: %s", result)
		}
	})
}

// ---------------------------------------------------------------------------
// handleName edge case tests
// ---------------------------------------------------------------------------

func TestHandleNameEdgeCases(t *testing.T) {
	t.Run("show name when empty and no name set", func(t *testing.T) {
		ctx := newTestContext()
		ctx.SessionName = ""
		_, err := Handle("/name", ctx)
		if err == nil {
			t.Fatal("expected error when no name set")
		}
		if !strings.Contains(err.Error(), "Usage: /name") {
			t.Errorf("error = %v", err)
		}
	})
}

// ---------------------------------------------------------------------------
// handleSession edge case tests
// ---------------------------------------------------------------------------

func TestHandleSessionEdgeCases(t *testing.T) {
	t.Run("no name, no token usage, no cost", func(t *testing.T) {
		ctx := newTestContext()
		ctx.SessionName = ""
		ctx.Messages = nil
		ctx.TokenUsage = nil
		ctx.TotalCost = 0

		result, err := Handle("/session", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "Session Info") {
			t.Errorf("expected Session Info, got: %s", result)
		}
		if strings.Contains(result, "Tokens") {
			t.Errorf("should not contain Tokens section when nil: %s", result)
		}
		if strings.Contains(result, "Cost") {
			t.Errorf("should not contain Cost when zero: %s", result)
		}
	})

	t.Run("with cache tokens", func(t *testing.T) {
		ctx := newTestContext()
		ctx.TokenUsage = &TokenUsage{
			Input:      100,
			Output:     50,
			CacheRead:  200,
			CacheWrite: 80,
			Total:      430,
		}

		result, err := Handle("/session", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "Cache Read: 200") {
			t.Errorf("expected Cache Read: %s", result)
		}
		if !strings.Contains(result, "Cache Write: 80") {
			t.Errorf("expected Cache Write: %s", result)
		}
	})

	t.Run("with cost but no tokens", func(t *testing.T) {
		ctx := newTestContext()
		ctx.TotalCost = 0.005
		ctx.TokenUsage = nil

		result, err := Handle("/session", ctx)
		if err != nil {
			t.Fatalf("unexpected error: %v", err)
		}
		if !strings.Contains(result, "$0.0050") {
			t.Errorf("expected cost, got: %s", result)
		}
	})
}

// ---------------------------------------------------------------------------
// handleExport error path tests
// ---------------------------------------------------------------------------

func TestHandleExportErrors(t *testing.T) {
	t.Run("JSONL source not found", func(t *testing.T) {
		ctx := newTestContext()
		ctx.SessionDir = "/nonexistent_dir_xyz"
		ctx.CurrentSessionID = "badsession"
		_, err := Handle("/export /tmp/out.jsonl", ctx)
		if err == nil {
			t.Fatal("expected error for nonexistent source")
		}
	})

	t.Run("HTML export to read-only dir", func(t *testing.T) {
		ctx := newTestContext()
		ctx.CurrentSessionID = "test"
		ctx.Messages = []Message{{Role: "user", Content: "hi"}}
		_, err := Handle("/export /root/out.html", ctx)
		if err == nil {
			t.Fatal("expected error for read-only destination")
		}
	})
}

// ---------------------------------------------------------------------------
// Handle dispatch edge case tests
// ---------------------------------------------------------------------------

func TestHandleDispatchAllCommands(t *testing.T) {
	// Verify all 22 commands dispatch without error (happy paths)
	os.MkdirAll("/tmp/sessions", 0755)
	os.WriteFile("/tmp/sessions/20260508-120000.jsonl", []byte(`{"type":"user","content":"hello"}`+"\n"), 0644)
	defer os.RemoveAll("/tmp/sessions")

	tests := []struct {
		command     string
		wantInResult string
	}{
		{"/model", "Current model:"},
		{"/model gpt-4", "Model set to: gpt-4"},
		{"/baseurl", "Current base URL:"},
		{"/baseurl https://api.openai.com", "Base URL set to:"},
		{"/memory", "Memory:"},
		{"/clear abc", "Deleted session abc"},
		{"/settings", "gpt-4o"},
		{"/scoped-models", "Scoped models:"},
		{"/help", "xihu"},
		{"/hotkeys", "Keybindings:"},
		{"/login", "Authentication:"},
		{"/logout", "unset LLM_API_KEY"},
		{"/new", "NEW_SESSION"},
		{"/quit", "QUIT"},
		{"/resume abc", "RESUME:abc"},
		{"/fork", "FORK:"},
		{"/clone", "CLONE:"},
	}

	for _, tc := range tests {
		t.Run(tc.command, func(t *testing.T) {
			result, err := Handle(tc.command, newTestContext())
			if err != nil {
				t.Fatalf("unexpected error for %s: %v", tc.command, err)
			}
			if !strings.Contains(result, tc.wantInResult) {
				t.Errorf("%s: expected %q in result, got: %s", tc.command, tc.wantInResult, result)
			}
		})
	}
}

func TestHandleDispatchWithExtraWhitespace(t *testing.T) {
	// Test that leading/trailing whitespace and multiple spaces work
	result, err := Handle("  /quit  ", newTestContext())
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if result != "QUIT" {
		t.Errorf("expected QUIT, got: %s", result)
	}
}

// ---------------------------------------------------------------------------
// Context struct tests
// ---------------------------------------------------------------------------

func TestContextDefaults(t *testing.T) {
	ctx := newTestContext()
	if ctx.CWD != "/tmp" {
		t.Errorf("CWD = %s, want /tmp", ctx.CWD)
	}
	if ctx.SessionDir != "/tmp/sessions" {
		t.Errorf("SessionDir = %s, want /tmp/sessions", ctx.SessionDir)
	}
	if ctx.CurrentSessionID != "20260508-120000" {
		t.Errorf("CurrentSessionID = %s, want 20260508-120000", ctx.CurrentSessionID)
	}
}

func TestTokenUsageStruct(t *testing.T) {
	tu := &TokenUsage{
		Input:      500,
		Output:     300,
		CacheRead:  100,
		CacheWrite: 50,
		Total:      950,
	}
	if tu.Input != 500 {
		t.Errorf("Input = %d", tu.Input)
	}
	if tu.Total != 950 {
		t.Errorf("Total = %d", tu.Total)
	}
}

func TestSessionEntryStruct(t *testing.T) {
	se := SessionEntry{
		ID:      "entry-1",
		Type:    "user",
		Content: "hello world",
		ModelID: "gpt-4",
	}
	if se.ID != "entry-1" {
		t.Errorf("ID = %s", se.ID)
	}
	if se.Type != "user" {
		t.Errorf("Type = %s", se.Type)
	}
}
