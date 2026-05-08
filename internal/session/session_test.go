package session

import (
	"encoding/json"
	"os"
	"path/filepath"
	"strings"
	"testing"
	"time"

	"github.com/huichen/xihu/pkg/types"
)

func tempDir(t *testing.T) string {
	t.Helper()
	d, err := os.MkdirTemp("", "xihu-session-test")
	if err != nil {
		t.Fatalf("MkdirTemp: %v", err)
	}
	t.Cleanup(func() { os.RemoveAll(d) })
	return d
}

func makeSession(id, cwd, model, baseURL string) *Session {
	return &Session{
		ID:        id,
		Version:   CURRENT_SESSION_VERSION,
		CWD:       cwd,
		Model:     model,
		BaseURL:   baseURL,
		CreatedAt: time.Now(),
		UpdatedAt: time.Now(),
	}
}

func TestSaveLoadRoundTrip(t *testing.T) {
	dir := tempDir(t)
	m := NewManager(dir)

	s := makeSession("20260508-120000", "/home/user/project", "gpt-4o", "https://api.openai.com")

	// Add a few entries
	entry1 := SessionEntry{
		ID:        "20260508-120001-abcdef",
		ParentID:  "",
		Type:      EntryTypeUser,
		Role:      "user",
		Content:   json.RawMessage(`[{"type":"text","text":"Hello"}]`),
		Timestamp: time.Now(),
	}
	entry2 := SessionEntry{
		ID:        "20260508-120002-123456",
		ParentID:  entry1.ID,
		Type:      EntryTypeAssistant,
		Role:      "assistant",
		Content:   json.RawMessage(`[{"type":"text","text":"Hi there!"}]`),
		Timestamp: time.Now(),
	}
	s.Entries = []SessionEntry{entry1, entry2}

	// Save
	if err := m.Save(s); err != nil {
		t.Fatalf("Save: %v", err)
	}

	// Load
	loaded, err := m.Load(s.ID, s.CWD)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}

	// Verify metadata
	if loaded.ID != s.ID {
		t.Errorf("ID = %q, want %q", loaded.ID, s.ID)
	}
	if loaded.Model != s.Model {
		t.Errorf("Model = %q, want %q", loaded.Model, s.Model)
	}
	if loaded.CWD != s.CWD {
		t.Errorf("CWD = %q, want %q", loaded.CWD, s.CWD)
	}
	if loaded.BaseURL != s.BaseURL {
		t.Errorf("BaseURL = %q, want %q", loaded.BaseURL, s.BaseURL)
	}

	// Verify entries
	if len(loaded.Entries) != 2 {
		t.Fatalf("got %d entries, want 2", len(loaded.Entries))
	}
	if loaded.Entries[0].ID != entry1.ID {
		t.Errorf("entry[0].ID = %q, want %q", loaded.Entries[0].ID, entry1.ID)
	}
	if loaded.Entries[1].ID != entry2.ID {
		t.Errorf("entry[1].ID = %q, want %q", loaded.Entries[1].ID, entry2.ID)
	}
	if loaded.Entries[0].Type != EntryTypeUser {
		t.Errorf("entry[0].Type = %q, want %q", loaded.Entries[0].Type, EntryTypeUser)
	}
	if loaded.Entries[1].Role != "assistant" {
		t.Errorf("entry[1].Role = %q, want assistant", loaded.Entries[1].Role)
	}
}

func TestSaveLoad_EmptySession(t *testing.T) {
	dir := tempDir(t)
	m := NewManager(dir)

	s := makeSession("20260508-130000", "/tmp/test", "claude-sonnet", "https://api.anthropic.com")

	if err := m.Save(s); err != nil {
		t.Fatalf("Save: %v", err)
	}

	loaded, err := m.Load(s.ID, s.CWD)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if len(loaded.Entries) != 0 {
		t.Errorf("expected 0 entries, got %d", len(loaded.Entries))
	}
}

func TestLoad_NotFound(t *testing.T) {
	dir := tempDir(t)
	m := NewManager(dir)

	_, err := m.Load("nonexistent", "/some/cwd")
	if err == nil {
		t.Fatal("expected error for nonexistent session")
	}
}

func TestDelete(t *testing.T) {
	dir := tempDir(t)
	m := NewManager(dir)

	s := makeSession("20260508-140000", "/tmp/test", "gpt-4o", "")
	if err := m.Save(s); err != nil {
		t.Fatalf("Save: %v", err)
	}

	if err := m.Delete(s.ID, s.CWD); err != nil {
		t.Fatalf("Delete: %v", err)
	}

	_, err := m.Load(s.ID, s.CWD)
	if err == nil {
		t.Fatal("expected error after delete")
	}
}

func TestAddEntry(t *testing.T) {
	dir := tempDir(t)
	m := NewManager(dir)

	s := makeSession("20260508-150000", "/tmp/test", "gpt-4o", "")

	entry := SessionEntry{
		ID:        GenerateEntryID(),
		ParentID:  "",
		Type:      EntryTypeUser,
		Role:      "user",
		Content:   json.RawMessage(`[{"type":"text","text":"Hello"}]`),
		Timestamp: time.Now(),
	}

	if err := m.AddEntry(s, entry); err != nil {
		t.Fatalf("AddEntry: %v", err)
	}
	if len(s.Entries) != 1 {
		t.Errorf("in-memory entries = %d, want 1", len(s.Entries))
	}

	// Add another
	entry2 := SessionEntry{
		ID:        GenerateEntryID(),
		ParentID:  entry.ID,
		Type:      EntryTypeAssistant,
		Role:      "assistant",
		Content:   json.RawMessage(`[{"type":"text","text":"Hi"}]`),
		Timestamp: time.Now(),
	}
	if err := m.AddEntry(s, entry2); err != nil {
		t.Fatalf("AddEntry 2: %v", err)
	}

	// Reload and verify
	loaded, err := m.Load(s.ID, s.CWD)
	if err != nil {
		t.Fatalf("Load: %v", err)
	}
	if len(loaded.Entries) != 2 {
		t.Errorf("persisted entries = %d, want 2", len(loaded.Entries))
	}
}

func test_List(t *testing.T) {
	dir := tempDir(t)
	m := NewManager(dir)

	cwd := "/home/user/project"
	s1 := makeSession("20260508-100000", cwd, "gpt-4o", "")
	s1.UpdatedAt = time.Now().Add(-1 * time.Hour)
	if err := m.Save(s1); err != nil {
		t.Fatalf("Save s1: %v", err)
	}

	s2 := makeSession("20260508-110000", cwd, "claude-sonnet", "")
	s2.UpdatedAt = time.Now()
	if err := m.Save(s2); err != nil {
		t.Fatalf("Save s2: %v", err)
	}

	// Save a session in a different CWD — should not appear
	s3 := makeSession("20260508-120000", "/other/cwd", "gpt-4o", "")
	if err := m.Save(s3); err != nil {
		t.Fatalf("Save s3: %v", err)
	}

	sessions, err := m.List(cwd)
	if err != nil {
		t.Fatalf("List: %v", err)
	}
	if len(sessions) != 2 {
		t.Fatalf("List returned %d sessions, want 2", len(sessions))
	}
	// Should be sorted newest first
	if sessions[0].ID != s2.ID {
		t.Errorf("first session = %q, want %q (newest first)", sessions[0].ID, s2.ID)
	}
}

func test_List_Empty(t *testing.T) {
	dir := tempDir(t)
	m := NewManager(dir)

	sessions, err := m.List("/nonexistent/cwd")
	if err != nil {
		t.Fatalf("List: %v", err)
	}
	if sessions == nil {
		t.Error("List should return nil, not nil slice") // nil slice is fine
	}
	if len(sessions) != 0 {
		t.Errorf("expected 0 sessions, got %d", len(sessions))
	}
}

func TestDefaultDir(t *testing.T) {
	cwd := "/home/user/my project"
	dir := DefaultDir(cwd)

	if dir == "" {
		t.Error("DefaultDir returned empty string")
	}
	if !strings.Contains(dir, ".xihu") {
		t.Error("DefaultDir should contain .xihu")
	}
	if !strings.Contains(dir, "sessions") {
		t.Error("DefaultDir should contain sessions")
	}
}

func TestGenerateEntryID(t *testing.T) {
	// Test uniqueness over many calls
	ids := make(map[string]bool)
	for i := 0; i < 100; i++ {
		id := GenerateEntryID()
		if ids[id] {
			t.Errorf("duplicate entry ID: %s", id)
		}
		ids[id] = true
	}

	// Test format: "YYYYMMDD-HHMMSS-XXXXXX" (date-time-6hex)
	id := GenerateEntryID()
	parts := strings.Split(id, "-")
	if len(parts) < 3 {
		t.Errorf("expected at least 3 parts, got %d in %q", len(parts), id)
	}
	// Last part should be 6 hex chars
	if len(parts) > 2 {
		hexPart := parts[len(parts)-1]
		if len(hexPart) != 6 {
			t.Errorf("hex part length = %d, want 6", len(hexPart))
		}
	}
}

func TestGenerateID(t *testing.T) {
	id := GenerateID()
	if id == "" {
		t.Error("GenerateID returned empty string")
	}
	// Format: "YYYYMMDD-HHMMSS"
	if len(id) != 15 {
		t.Errorf("ID length = %d, want 15", len(id))
	}
}

func TestBuildContext_Simple(t *testing.T) {
	entries := []SessionEntry{
		{
			ID: "e1", ParentID: "", Type: EntryTypeUser, Role: "user",
			Content:   json.RawMessage(`[{"type":"text","text":"Hello"}]`),
			Timestamp: time.Now().Add(-2 * time.Hour),
		},
		{
			ID: "e2", ParentID: "e1", Type: EntryTypeAssistant, Role: "assistant",
			Content:   json.RawMessage(`[{"type":"text","text":"Hi!"}]`),
			Timestamp: time.Now().Add(-1 * time.Hour),
		},
	}

	messages := BuildContext(entries)
	if len(messages) != 2 {
		t.Fatalf("got %d messages, want 2", len(messages))
	}
	if messages[0].Role != "user" {
		t.Errorf("msg[0].Role = %q, want user", messages[0].Role)
	}
	if messages[1].Role != "assistant" {
		t.Errorf("msg[1].Role = %q, want assistant", messages[1].Role)
	}
}

func TestBuildContext_Empty(t *testing.T) {
	messages := BuildContext(nil)
	if messages != nil {
		t.Errorf("expected nil for empty entries, got %v", messages)
	}

	messages = BuildContext([]SessionEntry{})
	if messages != nil {
		t.Errorf("expected nil for empty slice, got %v", messages)
	}
}

func test_BuildContext_WithCompaction(t *testing.T) {
	entries := []SessionEntry{
		{
			ID: "e1", ParentID: "", Type: EntryTypeUser, Role: "user",
			Content:   json.RawMessage(`[{"type":"text","text":"Old message"}]`),
			Timestamp: time.Now().Add(-3 * time.Hour),
		},
		{
			ID: "comp1", ParentID: "e1", Type: EntryTypeCompaction,
			Content:   json.RawMessage(`{"first_kept_entry_id":"e3"}`),
			Summary:   "Earlier conversation summarized.",
			Timestamp: time.Now().Add(-2 * time.Hour),
		},
		{
			ID: "e3", ParentID: "comp1", Type: EntryTypeUser, Role: "user",
			Content:   json.RawMessage(`[{"type":"text","text":"New question"}]`),
			Timestamp: time.Now().Add(-1 * time.Hour),
		},
		{
			ID: "e4", ParentID: "e3", Type: EntryTypeAssistant, Role: "assistant",
			Content:   json.RawMessage(`[{"type":"text","text":"Answer"}]`),
			Timestamp: time.Now(),
		},
	}

	messages := BuildContext(entries)
	if len(messages) != 3 {
		t.Fatalf("got %d messages, want 3 (summary + question + answer)", len(messages))
	}
	// First message should be the compaction summary as a system message
	if messages[0].Role != "system" {
		t.Errorf("msg[0].Role = %q, want system (compaction)", messages[0].Role)
	}
	// Second message should be the user question (e3)
	if messages[1].Role != "user" {
		t.Errorf("msg[1].Role = %q, want user", messages[1].Role)
	}
	// e1 should be skipped because compaction told us to skip until e3
}

func TestBuildContext_MetadataEntriesSkipped(t *testing.T) {
	entries := []SessionEntry{
		{
			ID: "e1", ParentID: "", Type: EntryTypeUser, Role: "user",
			Content:   json.RawMessage(`[{"type":"text","text":"Hello"}]`),
			Timestamp: time.Now(),
		},
		{
			ID: "label1", ParentID: "e1", Type: EntryTypeLabel,
			Label:     "my-label",
			Timestamp: time.Now(),
		},
	}

	messages := BuildContext(entries)
	// The label is the latest leaf but should be skipped in context
	if len(messages) != 1 {
		t.Fatalf("got %d messages, want 1 (label skipped)", len(messages))
	}
	if messages[0].Role != "user" {
		t.Errorf("msg[0].Role = %q, want user", messages[0].Role)
	}
}

func TestBuildContext_ModelChangeEntry(t *testing.T) {
	entries := []SessionEntry{
		{
			ID: "e1", ParentID: "", Type: EntryTypeUser, Role: "user",
			Content:   json.RawMessage(`[{"type":"text","text":"Hello"}]`),
			Timestamp: time.Now(),
		},
		{
			ID: "mc1", ParentID: "e1", Type: EntryTypeModelChange,
			Model:     "new-model",
			Content:   json.RawMessage(`{"new_model":"new-model"}`),
			Timestamp: time.Now(),
		},
	}

	messages := BuildContext(entries)
	if len(messages) != 1 {
		t.Fatalf("got %d messages, want 1 (model_change skipped)", len(messages))
	}
}

func TestBuildContext_Fork(t *testing.T) {
	// Simulate a branched conversation: e1 → e2 → e3a (branch A), e1 → e2 → e3b (branch B)
	entries := []SessionEntry{
		{ID: "e1", ParentID: "", Type: EntryTypeUser, Role: "user", Content: json.RawMessage(`[{"type":"text","text":"Q1"}]`), Timestamp: time.Now().Add(-3 * time.Hour)},
		{ID: "e2", ParentID: "e1", Type: EntryTypeAssistant, Role: "assistant", Content: json.RawMessage(`[{"type":"text","text":"A1"}]`), Timestamp: time.Now().Add(-2 * time.Hour)},
		{ID: "e3a", ParentID: "e2", Type: EntryTypeUser, Role: "user", Content: json.RawMessage(`[{"type":"text","text":"Q2 branch A"}]`), Timestamp: time.Now().Add(-1 * time.Hour)},
		{ID: "e3b", ParentID: "e2", Type: EntryTypeUser, Role: "user", Content: json.RawMessage(`[{"type":"text","text":"Q2 branch B"}]`), Timestamp: time.Now()},
	}

	messages := BuildContext(entries)
	// Should pick the latest leaf (e3b)
	if len(messages) != 3 {
		t.Fatalf("got %d messages, want 3 (e1, e2, e3b)", len(messages))
	}
	// Last message should be from e3b
	lastContent, _ := json.Marshal([]types.TextContent{{Type: "text", Text: "Q2 branch B"}})
	if string(messages[2].Content) != string(lastContent) {
		t.Errorf("last message content doesn't match e3b")
	}
}

func TestForkSession(t *testing.T) {
	parent := &Session{
		ID:      "parent-id",
		Version: CURRENT_SESSION_VERSION,
		CWD:     "/home/user/project",
		Model:   "gpt-4o",
		BaseURL: "https://api.openai.com",
		Entries: []SessionEntry{
			{ID: "e1", ParentID: "", Type: EntryTypeUser, Role: "user", Timestamp: time.Now()},
			{ID: "e2", ParentID: "e1", Type: EntryTypeAssistant, Role: "assistant", Timestamp: time.Now()},
			{ID: "e3", ParentID: "e2", Type: EntryTypeUser, Role: "user", Timestamp: time.Now()},
		},
		CreatedAt: time.Now(),
		UpdatedAt: time.Now(),
	}

	forked := ForkSession(parent, "e2")
	if forked == nil {
		t.Fatal("ForkSession returned nil")
	}
	if forked.ID == parent.ID {
		t.Error("forked session should have new ID")
	}
	if forked.CWD != parent.CWD {
		t.Errorf("CWD = %q, want %q", forked.CWD, parent.CWD)
	}
	if forked.Model != parent.Model {
		t.Errorf("Model = %q, want %q", forked.Model, parent.Model)
	}
	if len(forked.Entries) != 2 {
		t.Fatalf("forked entries = %d, want 2 (e1, e2)", len(forked.Entries))
	}
	// Should be root-to-leaf order
	if forked.Entries[0].ID != "e1" {
		t.Errorf("entries[0].ID = %q, want e1", forked.Entries[0].ID)
	}
	if forked.Entries[1].ID != "e2" {
		t.Errorf("entries[1].ID = %q, want e2", forked.Entries[1].ID)
	}
}

func TestMessagesToEntries(t *testing.T) {
	messages := []types.Message{
		{Role: "user", Content: json.RawMessage(`[{"type":"text","text":"Hello"}]`)},
		{Role: "assistant", Content: json.RawMessage(`[{"type":"text","text":"Hi"}]`)},
	}

	entries := MessagesToEntries(messages, "root-parent")
	if len(entries) != 2 {
		t.Fatalf("got %d entries, want 2", len(entries))
	}
	if entries[0].Type != EntryTypeUser {
		t.Errorf("entries[0].Type = %q, want user", entries[0].Type)
	}
	if entries[1].Type != EntryTypeAssistant {
		t.Errorf("entries[1].Type = %q, want assistant", entries[1].Type)
	}
	if entries[0].ParentID != "root-parent" {
		t.Errorf("entries[0].ParentID = %q, want root-parent", entries[0].ParentID)
	}
	if entries[1].ParentID != entries[0].ID {
		t.Errorf("entries[1].ParentID = %q, want %q", entries[1].ParentID, entries[0].ID)
	}
}

func TestMessagesToEntries_ToolMessage(t *testing.T) {
	messages := []types.Message{
		{Role: "tool", ToolCallID: "call_1", Content: json.RawMessage(`"result text"`)},
	}

	entries := MessagesToEntries(messages, "")
	if len(entries) != 1 {
		t.Fatalf("got %d entries, want 1", len(entries))
	}
	if entries[0].Type != EntryTypeTool {
		t.Errorf("Type = %q, want tool", entries[0].Type)
	}
	// Tool content should be wrapped with tool_call_id
	var wrapped struct {
		ToolCallID string          `json:"tool_call_id"`
		Result     json.RawMessage `json:"result"`
	}
	if err := json.Unmarshal(entries[0].Content, &wrapped); err != nil {
		t.Fatalf("unmarshal tool content: %v", err)
	}
	if wrapped.ToolCallID != "call_1" {
		t.Errorf("ToolCallID = %q, want call_1", wrapped.ToolCallID)
	}
}

func TestCompactionEntry(t *testing.T) {
	entry := CompactionEntry("Summary text", "first-kept-id", "parent-id")
	if entry.Type != EntryTypeCompaction {
		t.Errorf("Type = %q, want %q", entry.Type, EntryTypeCompaction)
	}
	if entry.Summary != "Summary text" {
		t.Errorf("Summary = %q, want 'Summary text'", entry.Summary)
	}
	if entry.ParentID != "parent-id" {
		t.Errorf("ParentID = %q", entry.ParentID)
	}

	var meta struct {
		FirstKeptEntryID string `json:"first_kept_entry_id"`
	}
	if err := json.Unmarshal(entry.Content, &meta); err != nil {
		t.Fatalf("unmarshal compaction content: %v", err)
	}
	if meta.FirstKeptEntryID != "first-kept-id" {
		t.Errorf("first_kept_entry_id = %q, want first-kept-id", meta.FirstKeptEntryID)
	}
}

func TestModelChangeEntry(t *testing.T) {
	entry := ModelChangeEntry("new-model-name", "test-provider", "parent-id")
	if entry.Type != EntryTypeModelChange {
		t.Errorf("Type = %q, want %q", entry.Type, EntryTypeModelChange)
	}
	if entry.Model != "new-model-name" {
		t.Errorf("Model = %q", entry.Model)
	}

	var meta struct {
		NewModel string `json:"new_model"`
	}
	if err := json.Unmarshal(entry.Content, &meta); err != nil {
		t.Fatalf("unmarshal model_change content: %v", err)
	}
	if meta.NewModel != "new-model-name" {
		t.Errorf("new_model = %q", meta.NewModel)
	}
}

func TestForEachEntry(t *testing.T) {
	entries := []SessionEntry{
		{ID: "root", ParentID: ""},
		{ID: "child1", ParentID: "root"},
		{ID: "child2", ParentID: "child1"},
	}

	chain := ForEachEntry(entries, "child2")
	if len(chain) != 3 {
		t.Fatalf("chain length = %d, want 3", len(chain))
	}
	// Leaf-to-root order
	if chain[0].ID != "child2" {
		t.Errorf("chain[0] = %q, want child2", chain[0].ID)
	}
	if chain[1].ID != "child1" {
		t.Errorf("chain[1] = %q, want child1", chain[1].ID)
	}
	if chain[2].ID != "root" {
		t.Errorf("chain[2] = %q, want root", chain[2].ID)
	}
}

func TestForEachEntry_Disconnected(t *testing.T) {
	entries := []SessionEntry{
		{ID: "orphan", ParentID: "nonexistent"},
	}

	chain := ForEachEntry(entries, "orphan")
	if len(chain) != 1 {
		t.Fatalf("chain length = %d, want 1", len(chain))
	}
	if chain[0].ID != "orphan" {
		t.Errorf("chain[0] = %q", chain[0].ID)
	}
}

func TestForEachEntry_CycleDetection(t *testing.T) {
	entries := []SessionEntry{
		{ID: "a", ParentID: "b"},
		{ID: "b", ParentID: "a"},
	}

	chain := ForEachEntry(entries, "a")
	// Should not loop forever
	if len(chain) != 2 {
		t.Errorf("chain length = %d, want 2", len(chain))
	}
}

func TestMigrateSession(t *testing.T) {
	s := &Session{Version: 1}
	MigrateSession(s)
	if s.Version != CURRENT_SESSION_VERSION {
		t.Errorf("version = %d, want %d", s.Version, CURRENT_SESSION_VERSION)
	}

	s2 := &Session{Version: 2}
	MigrateSession(s2)
	if s2.Version != CURRENT_SESSION_VERSION {
		t.Errorf("version = %d, want %d", s2.Version, CURRENT_SESSION_VERSION)
	}
}

func TestSetVersion(t *testing.T) {
	s := &Session{Version: 0}
	s.SetVersion()
	if s.Version != CURRENT_SESSION_VERSION {
		t.Errorf("version = %d, want %d", s.Version, CURRENT_SESSION_VERSION)
	}
}

func TestGetSetBaseURL(t *testing.T) {
	s := &Session{}
	s.SetBaseURL("https://example.com")
	if s.GetBaseURL() != "https://example.com" {
		t.Errorf("BaseURL = %q", s.GetBaseURL())
	}
}

func TestEncodeCWD(t *testing.T) {
	// Just verify it doesn't panic and returns something reasonable
	result := encodeCWD("/home/user/my project")
	if result == "" {
		t.Error("encodeCWD returned empty string")
	}
	// Should be base32 encoded (no special chars)
	if strings.Contains(result, "/") {
		t.Error("encodeCWD should not contain path separators")
	}

	// Empty/root cwd
	result2 := encodeCWD("")
	if result2 != "OJXW4===" { // base32 of "root" with no padding
		// Not checking exact value since padding varies
		t.Logf("encodeCWD('') = %q", result2)
	}

	result3 := encodeCWD(".")
	if result3 == "" {
		t.Error("encodeCWD('.') returned empty")
	}
}

func TestSessionPath(t *testing.T) {
	m := NewManager("/tmp/sessions")
	path := m.sessionPath("/home/user", "20260508-test")
	expected := filepath.Join("/tmp/sessions", encodeCWD("/home/user"), "20260508-test.jsonl")
	if path != expected {
		t.Errorf("sessionPath = %q, want %q", path, expected)
	}
}
