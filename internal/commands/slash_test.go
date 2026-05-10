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
